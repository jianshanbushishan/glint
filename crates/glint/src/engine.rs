use anyhow::{Context, Result, ensure};
use crossbeam_channel::{Receiver, Sender, bounded, select, tick};
use glint_core::{
    ActionHost, ActionOverride, ActionRef, ActionSpec, Config, ConfigOverrides, ConfigRuntime,
    GestureContext, GestureTemplate, HostCommand, Matcher, Point,
};
use glint_ipc::{Command, Response, Server, Status, validate_template_id, write_atomic};
use glint_platform::{Platform, PlatformEvent, TrayAction, WindowsHost};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, UNIX_EPOCH},
};

const DEFAULT_CONFIG: &str = include_str!("../../../config/config.json");

pub fn initialize(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join("config.json");
    if !path.exists() {
        write_atomic(&path, DEFAULT_CONFIG.as_bytes())?;
    }
    Ok(())
}

struct Control {
    command: Command,
    reply: Sender<Response>,
}
struct Job {
    runtime: Arc<ConfigRuntime>,
    action: ActionSpec,
    context: GestureContext,
    generation: u64,
}
struct Completion {
    name: String,
    error: Option<String>,
    generation: u64,
}

struct Engine {
    dir: PathBuf,
    runtime: Arc<ConfigRuntime>,
    config: Config,
    matcher: Matcher,
    platform: Option<Platform>,
    status: Status,
    recording: Option<(String, String, Instant)>,
    jobs: Sender<Job>,
    generation: Arc<AtomicU64>,
    fingerprint: u64,
    pending_change: Option<(u64, Instant)>,
    quit: bool,
}

pub fn run(dir: PathBuf, no_hooks: bool) -> Result<()> {
    // Bind first, so a second process cannot install another global mouse hook.
    let server = Server::bind(&dir)?;
    initialize(&dir)?;
    let runtime = Arc::new(ConfigRuntime::load(&dir.join("config.json"))?);
    let config = runtime.config().clone();
    glint_core::logging::set_level(config.logging.level);
    let matcher = Matcher::new(&config)?;
    let (events_tx, events_rx) = bounded(256);
    let platform = if no_hooks {
        None
    } else {
        let platform = Platform::start(events_tx, dir.clone())?;
        platform.configure(&config)?;
        Some(platform)
    };
    let events_rx = if no_hooks {
        crossbeam_channel::never()
    } else {
        events_rx
    };
    let generation = Arc::new(AtomicU64::new(0));
    let (jobs, jobs_rx) = bounded::<Job>(32);
    let (completions_tx, completions_rx) = bounded(64);
    let worker_generation = generation.clone();
    std::thread::Builder::new()
        .name("glint-actions".into())
        .spawn(move || action_worker(jobs_rx, completions_tx, worker_generation))?;
    let (controls_tx, controls_rx) = bounded::<Control>(32);
    let stop_server = Arc::new(AtomicBool::new(false));
    let server_stop = stop_server.clone();
    let server_thread = std::thread::Builder::new()
        .name("glint-control".into())
        .spawn(move || {
            let _ = server.run_until(
                move |command| {
                    let (reply, receive) = bounded(1);
                    if controls_tx
                        .send_timeout(Control { command, reply }, Duration::from_secs(1))
                        .is_err()
                    {
                        return Response::error("引擎忙，请稍后重试");
                    }
                    receive
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap_or_else(|_| Response::error("引擎处理请求超时"))
                },
                server_stop,
            );
        })?;
    let mut engine = Engine {
        fingerprint: fingerprint(&dir),
        dir,
        runtime,
        matcher,
        platform,
        jobs,
        generation,
        status: Status {
            running: true,
            engine_pid: Some(std::process::id()),
            gesture_count: config.gestures.len(),
            action_count: config.packages.iter().map(|p| p.actions.len()).sum(),
            ..Status::default()
        },
        config,
        recording: None,
        pending_change: None,
        quit: false,
    };
    engine.log(if no_hooks {
        "诊断模式已启动（未安装输入钩子）"
    } else {
        "手势引擎已启动"
    });
    let ticks = tick(Duration::from_millis(250));
    let mut last_watch = Instant::now();
    while !engine.quit {
        select! {
            recv(controls_rx) -> control => if let Ok(control) = control {
                if let Command::CaptureWindow { delay_ms } = control.command {
                    std::thread::spawn(move || {
                        let response = if delay_ms > 5000 { Response::error("窗口拾取等待不能超过 5 秒") } else {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                            match glint_platform::context_at_cursor() {
                                Ok(context) => { let mut response = Response::success("已拾取窗口"); response.context = Some(context); response }
                                Err(error) => Response::error(format!("{error:#}")),
                            }
                        };
                        let _ = control.reply.send(response);
                    });
                } else {
                    let response = match engine.command(control.command) { Ok(response) => response, Err(error) => { engine.error(format!("{error:#}")); Response::error(format!("{error:#}")) } };
                    let _ = control.reply.send(response);
                }
            },
            recv(events_rx) -> event => if let Ok(event) = event { engine.event(event); },
            recv(completions_rx) -> completion => if let Ok(completion) = completion
                && completion.generation == engine.generation.load(Ordering::Relaxed) {
                    if let Some(error) = completion.error { engine.error(format!("{}：{}", completion.name, error)); }
                    else { engine.status.last_action = Some(completion.name.clone()); engine.record(log::Level::Debug, &format!("已执行 · {}", completion.name)); }
                },
            recv(ticks) -> _ => {
                if engine.recording.as_ref().is_some_and(|(_,_,start)| start.elapsed() > Duration::from_secs(60)) {
                    let _ = engine.stop_recording(); engine.log("录制已超时取消");
                }
                if last_watch.elapsed() >= Duration::from_secs(1) { engine.watch(); last_watch = Instant::now(); }
            }
        }
    }
    engine.generation.fetch_add(1, Ordering::Relaxed);
    if let Some(platform) = &engine.platform {
        platform.shutdown()?;
    }
    stop_server.store(true, Ordering::Relaxed);
    let _ = server_thread.join();
    engine.log("引擎已退出");
    Ok(())
}

fn action_worker(jobs: Receiver<Job>, done: Sender<Completion>, generation: Arc<AtomicU64>) {
    while let Ok(job) = jobs.recv() {
        if job.generation != generation.load(Ordering::Relaxed) {
            continue;
        }
        let host = GuardedHost {
            generation: generation.clone(),
            expected: job.generation,
        };
        let result = job.runtime.execute(&job.action, &job.context, &host);
        let _ = done.send(Completion {
            name: job.action.name,
            error: result.err().map(|e| format!("{e:#}")),
            generation: job.generation,
        });
    }
}

struct GuardedHost {
    generation: Arc<AtomicU64>,
    expected: u64,
}
impl ActionHost for GuardedHost {
    fn perform(&self, command: HostCommand, context: &GestureContext) -> Result<()> {
        ensure!(
            self.generation.load(Ordering::Relaxed) == self.expected,
            "动作已取消"
        );
        WindowsHost.perform(command, context)
    }
}

impl Engine {
    fn log(&mut self, message: &str) {
        self.record(log::Level::Info, message);
    }

    fn record(&mut self, level: log::Level, message: &str) {
        log::log!(level, "{message}");
        if !log::log_enabled!(level) {
            return;
        }
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let entry = format!("{timestamp} [{level}] · {message}");
        self.status.recent_events.push(entry);
        if self.status.recent_events.len() > 60 {
            self.status.recent_events.remove(0);
        }
    }

    fn error(&mut self, message: String) {
        self.status.last_error = Some(message.clone());
        self.record(log::Level::Error, &message);
    }

    fn stop_recording(&mut self) -> Result<()> {
        if let Some(platform) = &self.platform {
            platform.record(false)?;
        }
        self.recording = None;
        self.status.recording = None;
        Ok(())
    }

    fn install(&mut self, runtime: ConfigRuntime) -> Result<()> {
        let config = runtime.config().clone();
        let matcher = Matcher::new(&config)?;
        if let Some(platform) = &self.platform {
            platform.configure(&config)?;
        }
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.runtime = Arc::new(runtime);
        self.matcher = matcher;
        self.config = config;
        glint_core::logging::set_level(self.config.logging.level);
        self.status.gesture_count = self.config.gestures.len();
        self.status.action_count = self.config.packages.iter().map(|p| p.actions.len()).sum();
        self.status.last_error = None;
        self.fingerprint = fingerprint(&self.dir);
        self.pending_change = None;
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        let runtime = ConfigRuntime::load(&self.dir.join("config.json"))?;
        self.install(runtime)?;
        self.log("配置已重新加载");
        Ok(())
    }

    // Stage file updates, validate the complete resulting configuration, restore on failure.
    fn update_file(&mut self, file: &str, bytes: &[u8]) -> Result<()> {
        let path = self.dir.join(file);
        let previous = fs::read(&path).ok();
        write_atomic(&path, bytes)?;
        let result = self.reload();
        if let Err(error) = result {
            if let Some(previous) = previous {
                write_atomic(&path, &previous).context("配置无效，且恢复原文件失败")?;
            } else {
                fs::remove_file(&path).context("配置无效，且移除新文件失败")?;
            }
            self.fingerprint = fingerprint(&self.dir);
            return Err(error.context("未应用更改，已保留原配置"));
        }
        Ok(())
    }

    fn overrides(&self) -> Result<ConfigOverrides> {
        let path = self.dir.join("overrides.json");
        if path.exists() {
            Ok(serde_json::from_slice(&fs::read(path)?)?)
        } else {
            Ok(ConfigOverrides::default())
        }
    }

    fn command(&mut self, command: Command) -> Result<Response> {
        let mut response = Response::success("完成");
        match command {
            Command::Status => {
                response.status = Some(self.status.clone());
            }
            Command::GetConfig => {
                response.config = Some(self.config.clone());
                response.status = Some(self.status.clone());
            }
            Command::GetSource => {
                response.source = Some(fs::read_to_string(self.dir.join("config.json"))?);
            }
            Command::SaveSource { source } => {
                ensure!(source.len() <= 1024 * 1024, "配置源码不能超过 1 MiB");
                self.update_file("config.json", source.as_bytes())?;
                response.message = "JSON 配置已保存并应用".into();
                response.config = Some(self.config.clone());
            }
            Command::Pause { paused } => {
                self.stop_recording()?;
                if let Some(platform) = &self.platform {
                    platform.pause(paused)?;
                }
                self.generation.fetch_add(1, Ordering::Relaxed);
                self.status.paused = paused;
                self.log(if paused {
                    "手势已暂停"
                } else {
                    "手势已恢复"
                });
                response.status = Some(self.status.clone());
            }
            Command::Reload => {
                self.reload()?;
                response.config = Some(self.config.clone());
            }
            Command::ResetOverrides => {
                self.update_file("overrides.json", b"{}")?;
                response.message = "已恢复 JSON 定义的偏好与动作，录制手势已保留".into();
                response.config = Some(self.config.clone());
            }
            Command::Record { id, name } => {
                validate_template_id(&id)?;
                ensure!(
                    !name.trim().is_empty() && name.len() <= 200,
                    "请填写有效的手势名称"
                );
                let platform = self.platform.as_ref().context("诊断模式不能录制手势")?;
                platform.record(true)?;
                self.generation.fetch_add(1, Ordering::Relaxed);
                self.recording = Some((id, name.clone(), Instant::now()));
                self.status.recording = Some(name.clone());
                self.log(&format!("正在录制 · {name}（60 秒内按住手势键绘制）"));
                response.status = Some(self.status.clone());
            }
            Command::CancelRecording => {
                self.stop_recording()?;
                response.status = Some(self.status.clone());
            }
            Command::CaptureWindow { .. } => {
                unreachable!("window picking is dispatched independently")
            }
            Command::SetPreferences {
                pen,
                stroke_button,
                logging,
            } => {
                let mut overrides = self.overrides()?;
                overrides.pen = Some(pen);
                overrides.stroke_button = Some(stroke_button);
                overrides.logging = Some(logging);
                self.update_file("overrides.json", &serde_json::to_vec_pretty(&overrides)?)?;
                response.config = Some(self.config.clone());
            }
            Command::UpsertAction { package_id, action } => {
                ensure!(
                    self.config.packages.iter().any(|p| p.id == package_id),
                    "动作包不存在，请先在 JSON 中定义动作包"
                );
                let mut overrides = self.overrides()?;
                overrides
                    .removed_actions
                    .retain(|a| a.package_id != package_id || a.action_id != action.id);
                overrides
                    .action_overrides
                    .retain(|a| a.package_id != package_id || a.action.id != action.id);
                overrides
                    .action_overrides
                    .push(ActionOverride { package_id, action });
                self.update_file("overrides.json", &serde_json::to_vec_pretty(&overrides)?)?;
                response.config = Some(self.config.clone());
            }
            Command::RemoveAction {
                package_id,
                action_id,
            } => {
                let package = self
                    .config
                    .packages
                    .iter()
                    .find(|p| p.id == package_id)
                    .context("动作包不存在")?;
                ensure!(
                    package.actions.iter().any(|a| a.id == action_id),
                    "动作不存在"
                );
                let mut overrides = self.overrides()?;
                overrides
                    .action_overrides
                    .retain(|a| a.package_id != package_id || a.action.id != action_id);
                overrides
                    .removed_actions
                    .retain(|a| a.package_id != package_id || a.action_id != action_id);
                overrides.removed_actions.push(ActionRef {
                    package_id,
                    action_id,
                });
                self.update_file("overrides.json", &serde_json::to_vec_pretty(&overrides)?)?;
                response.config = Some(self.config.clone());
            }
            Command::ApplyScope {
                package_id,
                application,
                upsert_actions,
                remove_actions,
            } => {
                let package = self.config.packages.iter().find(|p| p.id == package_id);
                ensure!(
                    application.is_some() || package.is_some() || package_id == "global",
                    "动作包不存在"
                );
                let mut changed_ids = std::collections::HashSet::new();
                for action in &upsert_actions {
                    ensure!(changed_ids.insert(&action.id), "重复的动作 ID");
                }
                for id in &remove_actions {
                    ensure!(changed_ids.insert(id), "同一个动作不能同时保存和删除");
                    ensure!(
                        package.is_some_and(|p| p.actions.iter().any(|a| a.id == *id)),
                        "待删除动作不存在"
                    );
                }
                let mut overrides = self.overrides()?;
                if let Some(mut application) = application {
                    application.process_path =
                        glint_core::normalize_application_path(&application.process_path)?;
                    ensure!(
                        application.id == package_id && package_id != "global",
                        "应用 ID 必须与作用域一致，且不能为 global"
                    );
                    overrides
                        .removed_applications
                        .retain(|id| id != &package_id);
                    overrides
                        .application_overrides
                        .retain(|app| app.id != package_id);
                    overrides.application_overrides.push(application);
                }
                for action in upsert_actions {
                    overrides
                        .removed_actions
                        .retain(|a| a.package_id != package_id || a.action_id != action.id);
                    overrides
                        .action_overrides
                        .retain(|a| a.package_id != package_id || a.action.id != action.id);
                    overrides.action_overrides.push(ActionOverride {
                        package_id: package_id.clone(),
                        action,
                    });
                }
                for action_id in remove_actions {
                    overrides
                        .action_overrides
                        .retain(|a| a.package_id != package_id || a.action.id != action_id);
                    overrides
                        .removed_actions
                        .retain(|a| a.package_id != package_id || a.action_id != action_id);
                    overrides.removed_actions.push(ActionRef {
                        package_id: package_id.clone(),
                        action_id,
                    });
                }
                self.update_file("overrides.json", &serde_json::to_vec_pretty(&overrides)?)?;
                response.config = Some(self.config.clone());
            }
            Command::RemoveApplication { package_id } => {
                ensure!(
                    package_id != "global"
                        && self
                            .config
                            .packages
                            .iter()
                            .any(|package| package.id == package_id),
                    "应用不存在"
                );
                let mut overrides = self.overrides()?;
                overrides
                    .application_overrides
                    .retain(|app| app.id != package_id);
                overrides
                    .removed_applications
                    .retain(|id| id != &package_id);
                overrides.removed_applications.push(package_id.clone());
                overrides
                    .action_overrides
                    .retain(|a| a.package_id != package_id);
                overrides
                    .removed_actions
                    .retain(|a| a.package_id != package_id);
                self.update_file("overrides.json", &serde_json::to_vec_pretty(&overrides)?)?;
                response.message = "已删除应用手势".into();
                response.config = Some(self.config.clone());
            }
            Command::QuitIfProcess { pid } => {
                ensure!(
                    pid == std::process::id(),
                    "后台进程已改变，请重新从托盘操作"
                );
                self.quit = true;
                response.message = "引擎正在退出".into();
            }
            Command::Quit => {
                self.quit = true;
                response.message = "引擎正在退出".into();
            }
        }
        Ok(response)
    }

    fn event(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::Gesture {
                points,
                context,
                recording,
            } => {
                if recording {
                    let Some((id, name, _)) = self.recording.take() else {
                        return;
                    };
                    let result = (|| -> Result<()> {
                        self.stop_recording()?;
                        ensure!(points.len() >= 2, "轨迹过短，请重新录制");
                        let path = self.dir.join("gestures.json");
                        let mut gestures: Vec<GestureTemplate> = if path.exists() {
                            serde_json::from_slice(&fs::read(path)?)?
                        } else {
                            Vec::new()
                        };
                        gestures.retain(|g| g.id != id);
                        gestures.push(GestureTemplate {
                            id,
                            name: name.clone(),
                            points: compact_recording(points),
                        });
                        self.update_file("gestures.json", &serde_json::to_vec_pretty(&gestures)?)?;
                        self.log(&format!("已保存手势 · {name}"));
                        Ok(())
                    })();
                    if let Err(error) = result {
                        self.error(format!("录制失败：{error:#}"));
                    }
                } else if !self.status.paused && self.recording.is_none() {
                    if let Some((id, score)) =
                        self.matcher
                            .recognize_binding(&context.process, &points, None)
                    {
                        self.status.last_gesture = Some(format!("{id} · {:.0}%", score * 100.0));
                        self.dispatch(&id, context);
                    } else {
                        self.status.last_gesture = Some("未匹配".into());
                        self.record(log::Level::Debug, "轨迹未匹配任何手势");
                    }
                }
            }
            PlatformEvent::Special {
                gesture,
                points,
                context,
            } => {
                if !self.status.paused && self.recording.is_none() {
                    if let Some((binding, _)) =
                        self.matcher
                            .recognize_binding(&context.process, &points, Some(&gesture))
                    {
                        self.status.last_gesture = Some(binding.clone());
                        self.dispatch(&binding, context);
                    } else {
                        self.status.last_gesture = Some("未匹配".into());
                        self.record(log::Level::Debug, "组合手势的轨迹未匹配");
                    }
                }
            }
            PlatformEvent::Error(message) => self.error(message),
            PlatformEvent::Tray(action) => {
                let result = match action {
                    TrayAction::TogglePause => self
                        .command(Command::Pause {
                            paused: !self.status.paused,
                        })
                        .map(|_| ()),
                    TrayAction::Reload => self.reload(),
                    TrayAction::Quit => {
                        self.quit = true;
                        Ok(())
                    }
                    TrayAction::OpenSettings => self.open_settings(),
                    TrayAction::RestartElevated => {
                        glint_platform::elevation::restart_elevated(&self.dir)
                    }
                };
                if let Err(error) = result {
                    self.error(format!("{error:#}"));
                }
            }
        }
    }

    fn dispatch(&mut self, gesture: &str, context: GestureContext) {
        if self.matcher.is_excluded(&context.process) {
            return;
        }
        if let Some(action) = self.matcher.match_action(&context.process, gesture) {
            let job = Job {
                runtime: self.runtime.clone(),
                action,
                context,
                generation: self.generation.load(Ordering::Relaxed),
            };
            if self.jobs.try_send(job).is_err() {
                self.error("动作队列已满，本次动作已丢弃".into());
            }
        } else {
            self.record(log::Level::Debug, &format!("没有适用的动作 · {gesture}"));
        }
    }

    fn open_settings(&self) -> Result<()> {
        let mut command = std::process::Command::new(
            std::env::current_exe()?.with_file_name("glint-settings.exe"),
        );
        command.arg("--config-dir").arg(&self.dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command
            .spawn()
            .context("无法打开设置，请确认 glint-settings.exe 与引擎在同一目录")?;
        Ok(())
    }

    fn watch(&mut self) {
        let current = fingerprint(&self.dir);
        if current == self.fingerprint {
            self.pending_change = None;
            return;
        }
        match self.pending_change {
            Some((seen, since))
                if seen == current && since.elapsed() >= Duration::from_millis(500) =>
            {
                if let Err(error) = self.reload() {
                    self.error(format!("配置重载失败，继续使用原配置：{error:#}"));
                    self.fingerprint = current;
                    self.pending_change = None;
                }
            }
            Some((seen, _)) if seen == current => {}
            _ => self.pending_change = Some((current, Instant::now())),
        }
    }
}

fn fingerprint(dir: &Path) -> u64 {
    fn visit(path: &Path, depth: usize, hash: &mut u64) {
        if depth > 4 {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries.into_iter().take(256) {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                visit(&path, depth + 1, hash);
                continue;
            }
            if !matches!(path.extension().and_then(|e| e.to_str()), Some("json")) {
                continue;
            }
            let stamp = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|t| t.as_nanos())
                .unwrap_or(0);
            for byte in format!("{}:{}:{stamp}", path.display(), meta.len()).bytes() {
                *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
    }
    let mut hash = 0xcbf29ce484222325;
    visit(dir, 0, &mut hash);
    hash
}

fn compact_recording(points: Vec<Point>) -> Vec<Point> {
    const LIMIT: usize = 2048;
    if points.len() <= LIMIT {
        return points;
    }
    (0..LIMIT)
        .map(|i| points[i * (points.len() - 1) / (LIMIT - 1)])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn events_recognize_only_the_captured_process_and_button_bindings() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.json"), r#"{
            "applications":[{"id":"editor","name":"Editor","process_path":"C:/Apps/editor.exe"}],
            "packages":[
                {"id":"global","name":"Global","match":[".*"],"actions":[{"id":"global_left","name":"Global left","gesture":"left","action":{"type":"keys","keys":"CTRL+C"}}]},
                {"id":"editor","name":"Editor","match":[],"actions":[
                    {"id":"local_left","name":"Local left","gesture":"recorded_left","action":{"type":"keys","keys":"CTRL+S"}},
                    {"id":"local_combo","name":"Local combo","gesture":"recorded_left+left_click","action":{"type":"keys","keys":"CTRL+V"}}
                ]}
            ]
        }"#).unwrap();
        let points = vec![Point { x: 100., y: 0. }, Point { x: 0., y: 0. }];
        fs::write(
            dir.path().join("gestures.json"),
            serde_json::to_vec(&vec![GestureTemplate {
                id: "recorded_left".into(),
                name: "Recorded".into(),
                points: points.clone(),
            }])
            .unwrap(),
        )
        .unwrap();
        let runtime = Arc::new(ConfigRuntime::load(&dir.path().join("config.json")).unwrap());
        let config = runtime.config().clone();
        let (jobs, received) = bounded(8);
        let mut engine = Engine {
            dir: dir.path().into(),
            matcher: Matcher::new(&config).unwrap(),
            config,
            runtime,
            platform: None,
            status: Status::default(),
            recording: None,
            jobs,
            generation: Arc::new(AtomicU64::new(0)),
            fingerprint: 0,
            pending_change: None,
            quit: false,
        };
        for (process, expected) in [
            (r"C:\Apps\editor.exe", "local_left"),
            (r"C:\Apps\other.exe", "global_left"),
        ] {
            let context = GestureContext {
                process: process.into(),
                ..Default::default()
            };
            engine.event(PlatformEvent::Gesture {
                points: points.clone(),
                context: context.clone(),
                recording: false,
            });
            assert_eq!(received.try_recv().unwrap().action.id, expected);
            engine.event(PlatformEvent::Special {
                gesture: "left_click".into(),
                points: points.clone(),
                context: context.clone(),
            });
            if process.ends_with("editor.exe") {
                assert_eq!(received.try_recv().unwrap().action.id, "local_combo");
            } else {
                assert!(received.try_recv().is_err());
            }
            engine.event(PlatformEvent::Special {
                gesture: "middle_click".into(),
                points: points.clone(),
                context,
            });
            assert!(received.try_recv().is_err());
        }
    }

    #[test]
    fn table_combinations_dispatch_without_falling_back_to_plain_actions() {
        let dir = tempfile::tempdir().unwrap();
        initialize(dir.path()).unwrap();
        let runtime = ConfigRuntime::load(&dir.path().join("config.json")).unwrap();
        let config = runtime.config();
        let matcher = Matcher::new(config).unwrap();
        for (shape, expected) in [
            ("right_down", "reopen_tab"),
            ("down_then_left", "force_close"),
            ("up_right", "fullscreen"),
        ] {
            let template = config.gestures.iter().find(|t| t.id == shape).unwrap();
            let (binding, _) = matcher
                .recognize_binding("test.exe", &template.points, Some("left_click"))
                .unwrap();
            assert_eq!(
                matcher.match_action("test.exe", &binding).unwrap().id,
                expected
            );
            assert_ne!(
                matcher.match_action("test.exe", shape).unwrap().id,
                expected
            );
        }
        let (direct, _) = matcher
            .recognize_binding("test.exe", &[], Some("left_click"))
            .unwrap();
        assert_eq!(
            matcher.match_action("test.exe", &direct).unwrap().id,
            "new_tab_click"
        );
        let up = config.gestures.iter().find(|t| t.id == "up").unwrap();
        assert!(
            matcher
                .recognize_binding("test.exe", &up.points, Some("left_click"))
                .is_none()
        );
        assert!(
            matcher
                .recognize_binding("test.exe", &[Point::default()], Some("left_click"))
                .is_none()
        );
    }
    #[test]
    fn cancel_guard_blocks_host_calls_from_a_previous_generation() {
        let generation = Arc::new(AtomicU64::new(5));
        let host = GuardedHost {
            generation,
            expected: 4,
        };
        assert!(
            host.perform(
                HostCommand::Keys("Ctrl+C".into()),
                &GestureContext::default()
            )
            .unwrap_err()
            .to_string()
            .contains("取消")
        );
    }
    #[test]
    fn long_recordings_are_bounded_and_keep_both_endpoints() {
        let points = (0..16000)
            .map(|i| Point {
                x: i as f64,
                y: (i % 100) as f64,
            })
            .collect();
        let compact = compact_recording(points);
        assert_eq!(compact.len(), 2048);
        assert_eq!(compact.first().unwrap().x, 0.0);
        assert_eq!(compact.last().unwrap().x, 15999.0);
    }
}
