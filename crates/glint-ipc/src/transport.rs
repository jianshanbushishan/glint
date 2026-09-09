use super::*;
use anyhow::{Context, ensure};
use interprocess::{
    ConnectWaitMode,
    local_socket::{
        ConnectOptions, GenericNamespaced, Listener, ListenerNonblockingMode, ListenerOptions,
        Stream, prelude::*,
    },
};
use serde::de::DeserializeOwned;
use std::{
    io::{self, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 4 * 1024 * 1024;

// Windows named pipes do not implement SO_RCVTIMEO/SO_SNDTIMEO. Bound the whole
// frame using nonblocking I/O instead, including peers that send partial frames.
struct DeadlineIo<'a> {
    stream: &'a mut Stream,
    deadline: Instant,
}
impl<'a> DeadlineIo<'a> {
    fn new(stream: &'a mut Stream, timeout: Duration) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            deadline: Instant::now() + timeout,
        })
    }
    fn retry(&self) -> io::Result<()> {
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "本地通信超时"));
        }
        std::thread::sleep(Duration::from_millis(3));
        Ok(())
    }
}
impl Read for DeadlineIo<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.stream.read(buffer) {
                // PIPE_NOWAIT can surface an empty pipe as zero bytes on Windows.
                // The frame deadline also bounds a peer that closes mid-message.
                Ok(0) if !buffer.is_empty() => self.retry()?,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    self.retry()?
                }
                result => return result,
            }
        }
    }
}
impl Write for DeadlineIo<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        // Keep writes below the default named-pipe buffer size. PIPE_NOWAIT may
        // otherwise return zero forever for a frame larger than its capacity.
        let chunk = &buffer[..buffer.len().min(512)];
        loop {
            match self.stream.write(chunk) {
                Ok(0) if !buffer.is_empty() => self.retry()?,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    self.retry()?
                }
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// Stable across processes/toolchain releases; isolate independent config directories.
fn socket_name(dir: &Path) -> String {
    let path = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned());
    let path = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let hash = path.bytes().fold(0xcbf29ce484222325u64, |acc, b| {
        (acc ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("glint-v1-{hash:016x}")
}

fn send<T: Serialize>(stream: &mut impl Write, value: &T) -> anyhow::Result<()> {
    let body = serde_json::to_vec(value)?;
    ensure!(body.len() <= MAX_FRAME, "IPC 消息超过 4 MiB");
    stream.write_all(&(body.len() as u32).to_le_bytes())?;
    stream.write_all(&body)?;
    Ok(())
}

fn receive<T: DeserializeOwned>(stream: &mut impl Read) -> anyhow::Result<T> {
    let mut size = [0; 4];
    stream.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    ensure!((1..=MAX_FRAME).contains(&size), "IPC 消息长度无效");
    let mut body = vec![0; size];
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

pub fn request(dir: &Path, command: Command) -> anyhow::Result<Response> {
    let name = socket_name(dir);
    let mut stream = ConnectOptions::new()
        .name(name.to_ns_name::<GenericNamespaced>()?)
        .wait_mode(ConnectWaitMode::Timeout(Duration::from_secs(1)))
        .connect_sync()
        .context("无法连接 Glint，请先启动后台引擎")?;
    send(
        &mut DeadlineIo::new(&mut stream, Duration::from_secs(3))?,
        &command,
    )?;
    receive(&mut DeadlineIo::new(&mut stream, Duration::from_secs(12))?)
        .context("Glint 响应超时或连接中断")
}

pub struct Server(Listener);
impl Server {
    /// Bind before installing the global hook: named-pipe ownership is our instance lock.
    pub fn bind(dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let name = socket_name(dir);
        let listener = ListenerOptions::new()
            .name(name.to_ns_name::<GenericNamespaced>()?)
            .create_sync()
            .context("Glint 已运行，或无法创建本地控制通道")?;
        Ok(Self(listener))
    }

    pub fn run(
        self,
        handler: impl Fn(Command) -> Response + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        self.run_until(handler, Arc::new(AtomicBool::new(false)))
    }

    pub fn run_until(
        self,
        handler: impl Fn(Command) -> Response + Send + Sync + 'static,
        stop: Arc<AtomicBool>,
    ) -> anyhow::Result<()> {
        self.0.set_nonblocking(ListenerNonblockingMode::Accept)?;
        let handler = Arc::new(handler);
        let clients = Arc::new(AtomicUsize::new(0));
        while !stop.load(Ordering::Relaxed) {
            let mut stream = match self.0.accept() {
                Ok(stream) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(15));
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if clients.load(Ordering::Relaxed) >= 8 {
                continue;
            }
            clients.fetch_add(1, Ordering::Relaxed);
            let clients = clients.clone();
            let handler = handler.clone();
            std::thread::spawn(move || {
                let _ = handle_client(&mut stream, handler.as_ref());
                clients.fetch_sub(1, Ordering::Relaxed);
            });
        }
        // Deliver the Quit response before releasing the instance lock and exiting.
        while clients.load(Ordering::Relaxed) != 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

fn handle_client(stream: &mut Stream, handler: &dyn Fn(Command) -> Response) -> anyhow::Result<()> {
    let response = match receive(&mut DeadlineIo::new(stream, Duration::from_secs(2))?) {
        Ok(command) => handler(command),
        Err(error) => Response::error(format!("无效请求：{error}")),
    };
    send(
        &mut DeadlineIo::new(stream, Duration::from_secs(3))?,
        &response,
    )
}

pub fn serve(
    dir: &Path,
    handler: impl Fn(Command) -> Response + Send + Sync + 'static,
) -> anyhow::Result<()> {
    Server::bind(dir)?.run(handler)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_handles_multiple_messages_without_waiting_for_eof() {
        let mut bytes = Vec::new();
        send(&mut bytes, &Command::Pause { paused: true }).unwrap();
        send(&mut bytes, &Command::Status).unwrap();
        let mut cursor = std::io::Cursor::new(bytes);
        assert!(matches!(
            receive::<Command>(&mut cursor).unwrap(),
            Command::Pause { paused: true }
        ));
        assert!(matches!(
            receive::<Command>(&mut cursor).unwrap(),
            Command::Status
        ));
    }

    #[test]
    fn rejects_oversized_frame_before_allocation() {
        let mut bytes = std::io::Cursor::new(u32::MAX.to_le_bytes());
        assert!(receive::<Command>(&mut bytes).is_err());
    }

    #[test]
    fn socket_identity_depends_on_config_directory() {
        assert_eq!(
            socket_name(Path::new("C:/Glint")),
            socket_name(Path::new("c:\\glint"))
        );
        assert_ne!(
            socket_name(Path::new("C:/Glint")),
            socket_name(Path::new("C:/Other"))
        );
    }

    #[test]
    fn actual_local_socket_roundtrip_and_exclusive_binding() {
        let dir = std::env::temp_dir().join(format!("glint-ipc-test-{}", std::process::id()));
        let server = Server::bind(&dir).unwrap();
        assert!(Server::bind(&dir).is_err());
        let task = std::thread::spawn(move || {
            let mut stream = server.0.accept().unwrap();
            handle_client(&mut stream, &|command| {
                assert!(matches!(command, Command::Status));
                Response::success("connected")
            })
            .unwrap();
        });
        assert_eq!(request(&dir, Command::Status).unwrap().message, "connected");
        task.join().unwrap();
        let _ = std::fs::remove_dir(&dir);
    }
}
