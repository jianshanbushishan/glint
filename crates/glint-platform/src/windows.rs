use crate::{
    PlatformEvent, TrayAction,
    input::{Capture, parse_keys},
};
use anyhow::{Context, Result, anyhow, bail};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use glint_core::{
    ActionHost, Config, GestureContext, HostCommand, Matcher, MouseButton, PenConfig, Point,
};
use std::{
    cell::{Cell, RefCell},
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

mod overlay;
mod tray_menu;
use overlay::Overlay;

const WAKE: u32 = WM_APP + 41;
const TRAY_MESSAGE: u32 = WM_APP + 42;
const SHOW_TRAY_MENU: u32 = WM_APP + 43;
const INJECTED: usize = 0x474c_494e;
static KEY_INJECTION: Mutex<()> = Mutex::new(());
thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static REQUESTS: RefCell<Option<Receiver<Request>>> = const { RefCell::new(None) };
    static TRAY_EVENTS: RefCell<Option<Sender<PlatformEvent>>> = const { RefCell::new(None) };
    static PAUSED: Cell<bool> = const { Cell::new(false) };
    static TRAY_MENU_OPEN: Cell<bool> = const { Cell::new(false) };
}

struct RuntimeConfig {
    button: MouseButton,
    pen: PenConfig,
    min_distance: f64,
    matcher: Box<Matcher>,
}
impl RuntimeConfig {
    fn new(config: &Config) -> Result<Self> {
        if !config.min_distance.is_finite() || config.min_distance < 1.0 {
            bail!("min_distance must be at least 1 pixel")
        }
        overlay::validate_pen(&config.pen)?;
        Ok(Self {
            button: config.stroke_button,
            pen: config.pen.clone(),
            min_distance: config.min_distance,
            matcher: Box::new(Matcher::new(config)?),
        })
    }
}
enum Command {
    Configure(RuntimeConfig),
    Pause(bool),
    Record(bool),
    Shutdown,
}
struct Request {
    command: Command,
    done: Sender<Result<(), String>>,
}

/// Owns a dedicated hook/message-loop thread. Configuration changes are acknowledged.
pub struct Platform {
    requests: Sender<Request>,
    thread_id: u32,
    command_window: usize,
    thread: Mutex<Option<JoinHandle<()>>>,
    stopped: AtomicBool,
}
impl Platform {
    pub fn start(events: Sender<PlatformEvent>, config_dir: PathBuf) -> Result<Self> {
        // Low-level hook coordinates require process-wide awareness as well as
        // thread awareness; otherwise Windows can virtualize target-window APIs.
        unsafe {
            if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == 0
                && GetAwarenessFromDpiAwarenessContext(GetDpiAwarenessContextForProcess(
                    GetCurrentProcess(),
                )) != DPI_AWARENESS_PER_MONITOR_AWARE
            {
                return Err(native_error("Enable per-monitor DPI awareness"));
            }
        }
        let (requests, receiver) = unbounded();
        let (ready_tx, ready_rx) = bounded(1);
        let errors = events.clone();
        let thread = thread::Builder::new()
            .name("glint-mouse".into())
            .spawn(move || {
                tray_menu::set_config_dir(config_dir);
                if let Err(error) = engine_thread(events, receiver, &ready_tx) {
                    let message = format!("{error:#}");
                    let _ = ready_tx.send(Err(message.clone()));
                    let _ = errors.try_send(PlatformEvent::Error(message));
                }
            })?;
        match ready_rx
            .recv()
            .context("Mouse engine stopped during startup")?
        {
            Ok((thread_id, command_window)) => Ok(Self {
                requests,
                thread_id,
                command_window,
                thread: Mutex::new(Some(thread)),
                stopped: AtomicBool::new(false),
            }),
            Err(message) => {
                let _ = thread.join();
                bail!("{message}")
            }
        }
    }
    pub fn configure(&self, config: &Config) -> Result<()> {
        self.request(Command::Configure(RuntimeConfig::new(config)?))
    }
    pub fn pause(&self, paused: bool) -> Result<()> {
        self.request(Command::Pause(paused))
    }
    pub fn record(&self, recording: bool) -> Result<()> {
        self.request(Command::Record(recording))
    }
    fn request(&self, command: Command) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            bail!("Mouse engine is stopped")
        }
        let (done, reply) = bounded(1);
        self.requests
            .send(Request { command, done })
            .context("Mouse engine disconnected")?;
        if unsafe { PostMessageW(self.command_window as HWND, WAKE, 0, 0) } == 0 {
            return Err(native_error("Wake mouse engine"));
        }
        reply
            .recv_timeout(Duration::from_secs(10))
            .context("Mouse engine did not acknowledge the command")?
            .map_err(|s| anyhow!(s))
    }
    pub fn shutdown(&self) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = self.request(Command::Shutdown);
        self.stopped.store(true, Ordering::Release);
        if result.is_err() {
            unsafe {
                PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
            }
        }
        if let Some(thread) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = thread.join();
        }
        result
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct Engine {
    tray_window: HWND,
    events: Sender<PlatformEvent>,
    config: Option<RuntimeConfig>,
    paused: bool,
    recording: bool,
    recording_pending: bool,
    lost_events: u64,
    capture: Capture,
    overlay: Overlay,
}
impl Engine {
    fn cancel(&mut self) {
        self.capture.cancel();
        self.overlay.clear();
    }
    fn emit(&mut self, event: PlatformEvent) -> bool {
        if self.events.try_send(event).is_err() {
            self.lost_events = self.lost_events.saturating_add(1);
            return false;
        }
        true
    }
    fn flush_errors(&mut self) {
        if self.lost_events != 0
            && self
                .events
                .try_send(PlatformEvent::Error(format!(
                    "{} mouse events were dropped because the controller queue was full",
                    self.lost_events
                )))
                .is_ok()
        {
            self.lost_events = 0;
        }
    }
    fn error(&mut self, error: anyhow::Error) {
        self.emit(PlatformEvent::Error(format!("{error:#}")));
    }
    fn handle(&mut self, message: u32, data: &MSLLHOOKSTRUCT) -> bool {
        let point = Point {
            x: data.pt.x as f64,
            y: data.pt.y as f64,
        };
        let button_event = mouse_button(message, data.mouseData);
        if let Some((button, down)) = button_event {
            if !down {
                // Owned ups remain swallowed even if capture was canceled or reconfigured.
                let owned = self.capture.release(button);
                if !owned {
                    return false;
                }
                if self
                    .capture
                    .active
                    .as_ref()
                    .is_some_and(|s| s.button == button)
                {
                    let stroke = self
                        .capture
                        .finish(point, self.config.as_ref().unwrap().min_distance)
                        .unwrap();
                    self.overlay.clear();
                    if log::log_enabled!(log::Level::Trace) {
                        log::trace!(
                            "capture samples={} start={:?} end={:?}",
                            stroke.points.len(),
                            stroke.points.first(),
                            stroke.points.last()
                        );
                    }
                    if !stroke.special && !stroke.invalid() {
                        if stroke.moved {
                            let recording = stroke.recording;
                            let delivered = self.emit(PlatformEvent::Gesture {
                                points: stroke.points,
                                context: stroke.context,
                                recording,
                            });
                            // Only wait for the controller after it owns the result.
                            // A full queue leaves recording armed so the user can retry.
                            if recording && delivered {
                                self.recording = false;
                                self.recording_pending = true;
                            }
                        } else if !stroke.recording
                            && let Err(e) = replay_click(button)
                        {
                            self.error(e);
                        }
                    }
                }
                return true;
            }
            if self.capture.owns(button) {
                return true;
            }
            if self.capture.active.is_some() {
                if let Some((points, context)) = self.capture.secondary_press(
                    button,
                    point,
                    self.config.as_ref().unwrap().min_distance,
                ) {
                    self.overlay.clear();
                    let gesture = match button {
                        MouseButton::Left => "left_click",
                        MouseButton::Right => "right_click",
                        MouseButton::Middle => "middle_click",
                        MouseButton::X1 => "x1_click",
                        MouseButton::X2 => "x2_click",
                    };
                    self.emit(PlatformEvent::Special {
                        gesture: gesture.into(),
                        points,
                        context,
                    });
                } else if self
                    .capture
                    .active
                    .as_ref()
                    .is_some_and(|stroke| stroke.invalid())
                {
                    self.overlay.clear();
                }
                return true;
            }
            let Some(config) = &self.config else {
                return false;
            };
            if button != config.button || self.recording_pending || (self.paused && !self.recording)
            {
                return false;
            }
            // A failed process query must never swallow the user's click.
            let Ok(context) = context_at(point) else {
                return false;
            };
            if !self.recording && config.matcher.is_excluded(&context.process) {
                return false;
            }
            let prefix = (!self.recording).then(|| {
                config
                    .matcher
                    .prefix_tracker(&context.process, config.min_distance)
            });
            self.capture.begin(button, point, context, self.recording);
            if let Some(prefix) = prefix {
                self.capture.track_prefix(prefix);
            }
            self.overlay.begin(point, config.pen.clone());
            return true;
        }
        if message == WM_MOUSEMOVE {
            if let Some(config) = &self.config {
                self.capture.move_to(point, config.min_distance);
                if let Some(stroke) = self.capture.active.as_ref().filter(|s| !s.special) {
                    if stroke.invalid() {
                        self.overlay.invalidate();
                    }
                    self.overlay.point(point);
                }
                // Bound memory during an arbitrarily long held gesture. Keep endpoints.
                if let Some(stroke) = &mut self.capture.active
                    && stroke.points.len() > 16384
                {
                    let last = *stroke.points.last().unwrap();
                    stroke.points = stroke.points.iter().step_by(2).copied().collect();
                    if stroke.points.last() != Some(&last) {
                        stroke.points.push(last);
                    }
                }
            }
            return false;
        }
        if message == WM_MOUSEWHEEL && self.capture.active.is_some() {
            if let Some((points, context)) = self
                .capture
                .wheel(point, self.config.as_ref().unwrap().min_distance)
            {
                self.overlay.clear();
                let gesture = if (data.mouseData >> 16) as i16 > 0 {
                    "wheel_up"
                } else {
                    "wheel_down"
                };
                let event = PlatformEvent::Special {
                    gesture: gesture.into(),
                    points,
                    context,
                };
                self.emit(event);
            } else if self
                .capture
                .active
                .as_ref()
                .is_some_and(|stroke| !stroke.recording)
            {
                self.overlay.clear();
            }
            return true;
        }
        false
    }
}

fn engine_thread(
    events: Sender<PlatformEvent>,
    receiver: Receiver<Request>,
    ready: &Sender<Result<(u32, usize), String>>,
) -> Result<()> {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut message: MSG = unsafe { zeroed() };
    unsafe {
        PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
    }
    let overlay = Overlay::start(events.clone())?;
    TRAY_EVENTS.with(|s| *s.borrow_mut() = Some(events.clone()));
    let tray = Tray::new()?;
    ENGINE.with(|s| {
        *s.borrow_mut() = Some(Engine {
            tray_window: tray.window,
            events,
            config: None,
            paused: false,
            recording: false,
            recording_pending: false,
            lost_events: 0,
            capture: Capture::default(),
            overlay,
        })
    });
    let hook =
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), GetModuleHandleW(null()), 0) };
    if hook.is_null() {
        ENGINE.with(|s| s.borrow_mut().take());
        return Err(native_error("Install global mouse hook"));
    }
    REQUESTS.with(|s| *s.borrow_mut() = Some(receiver));
    let _ = ready.send(Ok((unsafe { GetCurrentThreadId() }, tray.window as usize)));
    let timer = unsafe { SetTimer(null_mut(), 0, 500, None) };
    let mut loop_error = None;
    loop {
        let received = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if received <= 0 {
            if received < 0 {
                loop_error = Some(native_error("Read mouse messages"));
            }
            break;
        }
        if message.message == WM_TIMER && message.wParam == timer {
            ENGINE.with(|s| {
                if let Some(engine) = s.borrow_mut().as_mut() {
                    engine.flush_errors();
                }
            });
        } else {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    unsafe {
        KillTimer(null_mut(), timer);
        UnhookWindowsHookEx(hook);
    }
    ENGINE.with(|s| s.borrow_mut().take());
    REQUESTS.with(|s| s.borrow_mut().take());
    drop(tray);
    TRAY_EVENTS.with(|s| s.borrow_mut().take());
    if let Some(error) = loop_error {
        Err(error)
    } else {
        Ok(())
    }
}

fn apply_requests() {
    // Window messages also dispatch inside native modal loops (e.g. the tray menu).
    let receiver = REQUESTS.with(|s| s.borrow().clone());
    let Some(receiver) = receiver else { return };
    for request in receiver.try_iter() {
        let mut exit = false;
        let applied = ENGINE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(engine) = slot.as_mut() else {
                return false;
            };
            engine.cancel();
            match request.command {
                Command::Configure(config) => engine.config = Some(config),
                Command::Pause(paused) => {
                    engine.paused = paused;
                    PAUSED.with(|s| s.set(paused));
                    if let Err(error) = Tray::update(engine.tray_window, paused) {
                        engine.error(error);
                    }
                }
                Command::Record(recording) => {
                    engine.recording = recording;
                    engine.recording_pending = false;
                }
                Command::Shutdown => exit = true,
            }
            true
        });
        let _ = request.done.try_send(if applied {
            Ok(())
        } else {
            Err("Mouse engine is stopped".into())
        });
        if exit {
            unsafe {
                EndMenu();
                PostQuitMessage(0);
            }
            break;
        }
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let data = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if data.dwExtraInfo != INJECTED {
            // Never unwind through a foreign callback. Failing open preserves input.
            let swallowed = std::panic::catch_unwind(|| {
                ENGINE.with(|slot| {
                    slot.try_borrow_mut()
                        .ok()
                        .and_then(|mut slot| slot.as_mut().map(|e| e.handle(wparam as u32, data)))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
            if swallowed {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

fn mouse_button(message: u32, data: u32) -> Option<(MouseButton, bool)> {
    Some(match message {
        WM_LBUTTONDOWN => (MouseButton::Left, true),
        WM_LBUTTONUP => (MouseButton::Left, false),
        WM_RBUTTONDOWN => (MouseButton::Right, true),
        WM_RBUTTONUP => (MouseButton::Right, false),
        WM_MBUTTONDOWN => (MouseButton::Middle, true),
        WM_MBUTTONUP => (MouseButton::Middle, false),
        WM_XBUTTONDOWN | WM_XBUTTONUP => (
            if data >> 16 == 1 {
                MouseButton::X1
            } else {
                MouseButton::X2
            },
            message == WM_XBUTTONDOWN,
        ),
        _ => return None,
    })
}

fn replay_click(button: MouseButton) -> Result<()> {
    // Hook messages use the logical button; injected mouse flags name hardware buttons.
    let button = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        match button {
            MouseButton::Left => MouseButton::Right,
            MouseButton::Right => MouseButton::Left,
            other => other,
        }
    } else {
        button
    };
    let (down, up, data) = match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
        MouseButton::X1 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 1),
        MouseButton::X2 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 2),
    };
    let make = |flags| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTED,
            },
        },
    };
    let inputs = [make(down), make(up)];
    let count = unsafe { SendInput(2, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if count != 2 {
        if count == 1 {
            unsafe {
                SendInput(1, &inputs[1], size_of::<INPUT>() as i32);
            }
        }
        bail!(
            "Windows rejected mouse click injection ({count}/2); the target may have higher privileges (UIPI)"
        )
    }
    Ok(())
}

pub fn context_at_cursor() -> Result<GestureContext> {
    // Window picking also runs on the IPC worker, outside the DPI-aware hook thread.
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let result = (|| {
        let mut point: POINT = unsafe { zeroed() };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            return Err(native_error("Read cursor position"));
        }
        context_at(Point {
            x: point.x as f64,
            y: point.y as f64,
        })
    })();
    if !previous.is_null() {
        unsafe {
            SetThreadDpiAwarenessContext(previous);
        }
    }
    result
}
fn context_at(point: Point) -> Result<GestureContext> {
    unsafe {
        let window = GetAncestor(
            WindowFromPhysicalPoint(POINT {
                x: point.x as i32,
                y: point.y as i32,
            }),
            GA_ROOT,
        );
        if window.is_null() || IsWindow(window) == 0 {
            bail!("No window at gesture start")
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(native_error("Open gesture target process"));
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        let error = if ok == 0 {
            Some(native_error("Read gesture target process path"))
        } else {
            None
        };
        CloseHandle(process);
        if let Some(error) = error {
            return Err(error);
        }
        let mut title = vec![0u16; 1024];
        let title_length =
            GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
        Ok(GestureContext {
            window: window as usize as u64,
            process: String::from_utf16_lossy(&path[..length as usize]),
            title: String::from_utf16_lossy(&title[..title_length]),
            start: point,
            modifiers: 0,
        })
    }
}

pub struct WindowsHost;
impl ActionHost for WindowsHost {
    fn perform(&self, command: HostCommand, context: &GestureContext) -> Result<()> {
        match command {
            HostCommand::Launch { program, args } => {
                std::process::Command::new(&program)
                    .args(args)
                    .spawn()
                    .with_context(|| format!("Launch {program}"))?;
                Ok(())
            }
            HostCommand::Keys(keys) => send_keys(&keys, context),
            HostCommand::Window(operation) => window_action(&operation, context),
        }
    }
}
fn target(context: &GestureContext) -> Result<HWND> {
    let window = context.window as usize as HWND;
    if window.is_null() || unsafe { IsWindow(window) } == 0 {
        bail!("The gesture target window has closed")
    }
    Ok(window)
}
fn window_action(operation: &str, context: &GestureContext) -> Result<()> {
    let window = target(context)?;
    unsafe {
        match operation {
            "minimize" => {
                if ShowWindowAsync(window, SW_MINIMIZE) == 0 {
                    return Err(native_error("Minimize target window"));
                }
            }
            "maximize" => {
                if ShowWindowAsync(window, SW_MAXIMIZE) == 0 {
                    return Err(native_error("Maximize target window"));
                }
            }
            "restore" => {
                if ShowWindowAsync(window, SW_RESTORE) == 0 {
                    return Err(native_error("Restore target window"));
                }
            }
            "toggle_maximize" => {
                let state = if IsZoomed(window) != 0 {
                    SW_RESTORE
                } else {
                    SW_MAXIMIZE
                };
                if ShowWindowAsync(window, state) == 0 {
                    return Err(native_error("Toggle target window maximized state"));
                }
            }
            "close" => {
                if PostMessageW(window, WM_CLOSE, 0, 0) == 0 {
                    return Err(native_error("Close target window"));
                }
            }
            "force_close" => {
                let mut pid = 0;
                if GetWindowThreadProcessId(window, &mut pid) == 0 || pid == 0 {
                    return Err(native_error("Find target window process"));
                }
                let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
                if process.is_null() {
                    return Err(native_error("Open target process for termination"));
                }
                // Capture the termination error before closing the handle changes
                // the thread's last-error value; always release the owned handle.
                let termination_error = (TerminateProcess(process, 1) == 0)
                    .then(|| native_error("Terminate target window process"));
                let close_error = (CloseHandle(process) == 0)
                    .then(|| native_error("Close target process handle"));
                if let Some(error) = termination_error.or(close_error) {
                    return Err(error);
                }
            }
            "toggle_topmost" => {
                let order = if GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0 {
                    HWND_NOTOPMOST
                } else {
                    HWND_TOPMOST
                };
                if SetWindowPos(
                    window,
                    order,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                ) == 0
                {
                    return Err(native_error("Change target window topmost state"));
                }
            }
            _ => bail!("Unknown window operation: {operation}"),
        }
    }
    Ok(())
}
fn send_keys(text: &str, context: &GestureContext) -> Result<()> {
    let keys = parse_keys(text)?;
    let _injection = KEY_INJECTION.lock().unwrap_or_else(|e| e.into_inner());
    let window = target(context)?;
    unsafe {
        if GetForegroundWindow() != window {
            SetForegroundWindow(window);
            if GetForegroundWindow() != window {
                bail!("Windows would not focus the gesture target; no keys were sent")
            }
        }
    }
    // Snapshot all keys before injection; preserve any keys physically held by the user.
    let held: Vec<bool> = keys
        .iter()
        .map(|k| unsafe { GetAsyncKeyState(*k as i32) < 0 })
        .collect();
    let mut pressed = Vec::new();
    let mut outcome = Ok(());
    for (&key, &was_held) in keys.iter().zip(&held) {
        if was_held {
            continue;
        }
        if let Err(e) = inject_key(key, false) {
            outcome = Err(e);
            break;
        }
        pressed.push(key);
    }
    for key in pressed.into_iter().rev() {
        if let Err(e) = inject_key(key, true)
            && outcome.is_ok()
        {
            outcome = Err(e)
        }
    }
    outcome
}
fn inject_key(key: u16, up: bool) -> Result<()> {
    let extended =
        matches!(key, 0x21..=0x28 | 0x2c..=0x2e | 0x5b | 0x5c | 0xa3 | 0xa5 | 0xa6..=0xb7);
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: (if up { KEYEVENTF_KEYUP } else { 0 })
                    | (if extended { KEYEVENTF_EXTENDEDKEY } else { 0 }),
                time: 0,
                dwExtraInfo: INJECTED,
            },
        },
    };
    if unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) } != 1 {
        bail!("Windows rejected keyboard injection; the target may have higher privileges (UIPI)")
    }
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn native_error(operation: &str) -> anyhow::Error {
    anyhow!("{operation}: {}", std::io::Error::last_os_error())
}

struct Tray {
    window: HWND,
    icon: NOTIFYICONDATAW,
}
impl Tray {
    fn notification(window: HWND, paused: bool) -> Result<NOTIFYICONDATAW> {
        let (resource, tip) = tray_presentation(paused);
        unsafe {
            let mut icon: NOTIFYICONDATAW = zeroed();
            icon.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            icon.hWnd = window;
            icon.uID = 1;
            icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            icon.uCallbackMessage = TRAY_MESSAGE;
            icon.hIcon = LoadImageW(
                GetModuleHandleW(null()),
                resource as usize as _,
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_SHARED,
            ) as HICON;
            if icon.hIcon.is_null() {
                return Err(native_error("Load Glint tray icon"));
            }
            let tip = wide(tip);
            icon.szTip[..tip.len()].copy_from_slice(&tip);
            Ok(icon)
        }
    }
    fn update(window: HWND, paused: bool) -> Result<()> {
        let icon = Self::notification(window, paused)?;
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &icon) } == 0 {
            bail!("Could not update the Glint tray status icon");
        }
        Ok(())
    }
    fn new() -> Result<Self> {
        unsafe {
            let class = wide("Glint.Tray");
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(tray_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..zeroed()
            };
            if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(native_error("Register tray window"));
            }
            let window = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("Glint").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if window.is_null() {
                return Err(native_error("Create tray window"));
            }
            let icon = match Self::notification(window, false) {
                Ok(icon) => icon,
                Err(error) => {
                    DestroyWindow(window);
                    return Err(error);
                }
            };
            if Shell_NotifyIconW(NIM_ADD, &icon) == 0 {
                DestroyWindow(window);
                bail!("Could not add the Glint tray icon")
            }
            Ok(Self { window, icon })
        }
    }
}
fn tray_presentation(paused: bool) -> (u16, &'static str) {
    if paused {
        (2, "Glint · 已暂停鼠标手势")
    } else {
        (1, "Glint · 鼠标手势运行中")
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.icon);
            DestroyWindow(self.window);
        }
    }
}
unsafe extern "system" fn tray_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(result) = unsafe { tray_menu::handle_message(message, lparam) } {
        return result;
    }
    if message == WAKE {
        apply_requests();
        return 0;
    }
    if message == TRAY_MESSAGE && lparam as u32 == WM_RBUTTONUP {
        // Return Explorer's notification before activating our window. Opening
        // the menu here can make Explorer and the hook thread wait on each other.
        unsafe { PostMessageW(window, SHOW_TRAY_MENU, 0, 0) };
        return 0;
    }
    if message == TRAY_MESSAGE || message == SHOW_TRAY_MENU {
        let action = if message == TRAY_MESSAGE && lparam as u32 == WM_LBUTTONDBLCLK {
            Some(TrayAction::OpenSettings)
        } else if message == SHOW_TRAY_MENU {
            if TRAY_MENU_OPEN.with(|open| open.replace(true)) {
                return 0;
            }
            unsafe {
                let label = if PAUSED.with(|s| s.get()) {
                    "恢复鼠标手势"
                } else {
                    "暂停鼠标手势"
                };
                let mut point: POINT = zeroed();
                GetCursorPos(&mut point);
                SetForegroundWindow(window);
                let choice = tray_menu::show(window, point, label);
                PostMessageW(window, WM_NULL, 0, 0);
                TRAY_MENU_OPEN.with(|open| open.set(false));
                match choice {
                    1 => Some(TrayAction::OpenSettings),
                    2 => Some(TrayAction::TogglePause),
                    3 => Some(TrayAction::Reload),
                    4 => Some(TrayAction::Quit),
                    5 => Some(TrayAction::RestartElevated),
                    _ => None,
                }
            }
        } else {
            None
        };
        if let Some(action) = action {
            TRAY_EVENTS.with(|s| {
                if let Some(events) = s.borrow().as_ref() {
                    let _ = events.try_send(PlatformEvent::Tray(action));
                }
            });
        }
        return 0;
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

#[cfg(test)]
mod tray_tests {
    use super::*;
    #[test]
    #[ignore = "creates a temporary native trail window on the desktop"]
    fn recording_queue_overflow_allows_retry_before_waiting_for_controller() {
        let (events, received) = bounded(1);
        let overlay = Overlay::start(events.clone()).unwrap();
        let mut engine = Engine {
            tray_window: null_mut(),
            events,
            config: Some(RuntimeConfig::new(&Config::default()).unwrap()),
            paused: false,
            recording: true,
            recording_pending: false,
            lost_events: 0,
            capture: Capture::default(),
            overlay,
        };
        engine
            .events
            .try_send(PlatformEvent::Error("full".into()))
            .unwrap();
        let data = MSLLHOOKSTRUCT {
            pt: POINT { x: 100, y: 0 },
            ..unsafe { zeroed() }
        };
        for delivered in [false, true] {
            engine.capture.begin(
                MouseButton::Right,
                Point::default(),
                GestureContext::default(),
                true,
            );
            assert!(engine.handle(WM_RBUTTONUP, &data));
            assert_eq!(engine.recording, !delivered);
            assert_eq!(engine.recording_pending, delivered);
            let event = received.try_recv().unwrap();
            assert_eq!(
                matches!(
                    event,
                    PlatformEvent::Gesture {
                        recording: true,
                        ..
                    }
                ),
                delivered
            );
        }
        assert_eq!(engine.lost_events, 1);
    }

    #[test]
    fn pause_and_resume_choose_distinct_resources_and_status_text() {
        assert_eq!(tray_presentation(true), (2, "Glint · 已暂停鼠标手势"));
        assert_eq!(tray_presentation(false), (1, "Glint · 鼠标手势运行中"));
    }
}
