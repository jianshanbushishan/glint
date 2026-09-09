//! Real process + named-pipe integration, deliberately without global input hooks.
use glint_core::{
    ActionKind, ActionSpec, ApplicationProfile, LogLevel, LoggingConfig, Matcher, MouseButton,
    PenConfig,
};
use glint_ipc::{Command, Response, request};
use std::{
    path::Path,
    process::{Child, Command as Process, Stdio},
    time::{Duration, Instant},
};

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn binary(dir: &Path) -> Process {
    let mut process = Process::new(env!("CARGO_BIN_EXE_glint"));
    process
        .arg("--config-dir")
        .arg(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }
    process
}

fn call(dir: &Path, command: Command) -> Response {
    request(dir, command.clone()).unwrap_or_else(|e| panic!("{command:?}: {e:#}"))
}

fn start_service(dir: &Path) -> Service {
    let mut service = Service(binary(dir).arg("--no-hooks").spawn().unwrap());
    let started = Instant::now();
    while request(dir, Command::Status).is_err() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "service did not start"
        );
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "service exited during startup"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    service
}

#[test]
fn application_scopes_persist_atomically_without_flattening_base_config() {
    let dir = tempfile::tempdir().unwrap();
    assert!(binary(dir.path()).arg("init").status().unwrap().success());
    let source = r#"{"packages":[
        {"id":"global","name":"Global","match":[".*"],"actions":[
            {"id":"copy","name":"Copy","gesture":"left","action":{"type":"keys","keys":"CTRL+C"}},
            {"id":"paste","name":"Paste","gesture":"right","action":{"type":"keys","keys":"CTRL+V"}}
        ]}
    ]}"#;
    std::fs::write(dir.path().join("config.json"), source).unwrap();
    let service = start_service(dir.path());
    let mut app = ApplicationProfile {
        id: "editor".into(),
        name: "Editor".into(),
        process_path: "c:/Apps (x64)/./old/../editor[1].exe".into(),
        disabled: false,
        inherit_global: true,
    };
    let action = ActionSpec {
        id: "local_copy".into(),
        name: "Local copy".into(),
        gesture: "left".into(),
        action: ActionKind::Keys {
            keys: "CTRL+SHIFT+C".into(),
        },
    };
    let apply = |app: &ApplicationProfile, actions: Vec<ActionSpec>, remove: Vec<String>| {
        Command::ApplyScope {
            package_id: app.id.clone(),
            application: Some(app.clone()),
            upsert_actions: actions,
            remove_actions: remove,
        }
    };
    let response = call(dir.path(), apply(&app, vec![action.clone()], vec![]));
    assert!(response.ok, "{}", response.message);
    assert_eq!(
        response.config.as_ref().unwrap().applications[0].process_path,
        r"C:\Apps (x64)\editor[1].exe"
    );
    app.process_path = r"C:\Apps (x64)\editor[1].exe".into();
    let matcher = Matcher::new(response.config.as_ref().unwrap()).unwrap();
    assert_eq!(
        matcher
            .match_action(&app.process_path.to_uppercase(), "left")
            .unwrap()
            .id,
        "local_copy"
    );
    assert_eq!(
        matcher.match_action(&app.process_path, "right").unwrap().id,
        "paste"
    );
    assert_eq!(
        matcher
            .match_action(r"D:\Apps (x64)\editor[1].exe", "left")
            .unwrap()
            .id,
        "copy"
    );
    let sidecar_path = dir.path().join("overrides.json");
    let saved = std::fs::read(&sidecar_path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert!(json.get("packages").is_none());
    assert_eq!(
        json["application_overrides"][0]["process_path"],
        app.process_path
    );
    let duplicate = ApplicationProfile {
        id: "duplicate_editor".into(),
        process_path: "c:/APPS (x64)//./EDITOR[1].EXE".into(),
        ..app.clone()
    };
    assert!(!call(dir.path(), apply(&duplicate, vec![], vec![])).ok);
    assert_eq!(std::fs::read(&sidecar_path).unwrap(), saved);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.json")).unwrap(),
        source
    );

    // Metadata and bindings must both roll back when one edited action is invalid.
    let mut invalid = action.clone();
    invalid.action = ActionKind::Keys {
        keys: "NOT_A_KEY".into(),
    };
    app.name = "Unsaved".into();
    assert!(!call(dir.path(), apply(&app, vec![invalid], vec![])).ok);
    assert_eq!(std::fs::read(&sidecar_path).unwrap(), saved);
    assert_eq!(
        call(dir.path(), Command::GetConfig)
            .config
            .unwrap()
            .applications[0]
            .name,
        "Editor"
    );
    // A real filesystem write failure must likewise leave the live snapshot untouched.
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Permit config reads but deny replacement, independent of temp-file naming.
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1) // FILE_SHARE_READ, without FILE_SHARE_DELETE
            .open(&sidecar_path)
            .unwrap();
        assert!(!call(dir.path(), apply(&app, vec![], vec![])).ok);
        drop(locked);
    }
    assert_eq!(std::fs::read(&sidecar_path).unwrap(), saved);
    assert_eq!(
        call(dir.path(), Command::GetConfig)
            .config
            .unwrap()
            .applications[0]
            .name,
        "Editor"
    );

    app.name = "Edited editor".into();
    app.inherit_global = false;
    let response = call(dir.path(), apply(&app, vec![], vec![]));
    assert!(response.ok, "{}", response.message);
    let matcher = Matcher::new(response.config.as_ref().unwrap()).unwrap();
    assert!(matcher.match_action(&app.process_path, "right").is_none());
    assert_eq!(
        matcher.match_action(&app.process_path, "left").unwrap().id,
        "local_copy"
    );

    // Untouched base actions still receive source edits after GUI saves.
    assert!(
        call(
            dir.path(),
            Command::SaveSource {
                source: source.replace("CTRL+V", "CTRL+SHIFT+V")
            }
        )
        .ok
    );
    let config = call(dir.path(), Command::GetConfig).config.unwrap();
    let global = config.packages.iter().find(|p| p.id == "global").unwrap();
    assert!(matches!(&global.actions[0].action, ActionKind::Keys { keys } if keys == "CTRL+C"));
    assert!(
        matches!(&global.actions[1].action, ActionKind::Keys { keys } if keys == "CTRL+SHIFT+V")
    );

    app.disabled = true;
    let response = call(dir.path(), apply(&app, vec![], vec![]));
    assert!(response.ok);
    assert!(!response.config.unwrap().excluded.is_empty());
    drop(service);
    let _service = start_service(dir.path());
    let config = call(dir.path(), Command::GetConfig).config.unwrap();
    assert!(
        Matcher::new(&config)
            .unwrap()
            .is_excluded(&app.process_path)
    );
    assert_eq!(
        config
            .packages
            .iter()
            .find(|p| p.id == app.id)
            .unwrap()
            .actions
            .len(),
        1
    );

    app.disabled = false;
    app.inherit_global = true;
    app.process_path = r"C:\New Editor\editor.exe".into();
    let response = call(dir.path(), apply(&app, vec![], vec![action.id]));
    assert!(response.ok, "{}", response.message);
    assert_eq!(
        Matcher::new(response.config.as_ref().unwrap())
            .unwrap()
            .match_action(&app.process_path, "left")
            .unwrap()
            .id,
        "copy"
    );
    let response = call(
        dir.path(),
        Command::RemoveApplication {
            package_id: app.id.clone(),
        },
    );
    assert!(response.ok, "{}", response.message);
    let config = response.config.unwrap();
    assert!(config.applications.is_empty());
    assert!(config.packages.iter().all(|p| p.id != app.id));
    assert!(
        !Matcher::new(&config)
            .unwrap()
            .is_excluded(&app.process_path)
    );
    assert_eq!(
        Matcher::new(&config)
            .unwrap()
            .match_action(&app.process_path, "right")
            .unwrap()
            .id,
        "paste"
    );
    assert!(call(dir.path(), Command::Reload).ok);
    assert!(
        call(dir.path(), Command::GetConfig)
            .config
            .unwrap()
            .applications
            .is_empty()
    );
}

#[test]
fn first_gui_binding_can_create_the_global_scope_in_an_empty_config() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), "{}").unwrap();
    let _service = start_service(dir.path());
    let response = call(
        dir.path(),
        Command::ApplyScope {
            package_id: "global".into(),
            application: None,
            upsert_actions: vec![ActionSpec {
                id: "first_global".into(),
                name: "First global".into(),
                gesture: "left".into(),
                action: ActionKind::Keys {
                    keys: "CTRL+C".into(),
                },
            }],
            remove_actions: vec![],
        },
    );
    assert!(response.ok, "{}", response.message);
    assert_eq!(
        Matcher::new(response.config.as_ref().unwrap())
            .unwrap()
            .match_action("app.exe", "left")
            .unwrap()
            .id,
        "first_global"
    );
}

#[test]
fn configuration_control_is_transactional_and_hot_reload_recovers() {
    let dir = tempfile::tempdir().unwrap();
    assert!(binary(dir.path()).arg("init").status().unwrap().success());
    assert!(binary(dir.path()).arg("check").status().unwrap().success());
    let mut service = Service(binary(dir.path()).arg("--no-hooks").spawn().unwrap());
    let started = Instant::now();
    while request(dir.path(), Command::Status).is_err() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "service did not start"
        );
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "service exited during startup"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let initial = call(dir.path(), Command::GetConfig).config.unwrap();
    assert_eq!(initial.gestures.len(), 28);
    let source = call(dir.path(), Command::GetSource).source.unwrap();

    // A second process must fail before it can install another hook.
    assert!(
        !binary(dir.path())
            .arg("--no-hooks")
            .status()
            .unwrap()
            .success()
    );

    let invalid = call(
        dir.path(),
        Command::SaveSource {
            source: r#"{"pen":{"width":-10}}"#.into(),
        },
    );
    assert!(!invalid.ok);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.json")).unwrap(),
        source
    );
    assert_eq!(
        call(dir.path(), Command::GetConfig)
            .config
            .unwrap()
            .pen
            .width,
        initial.pen.width
    );

    // Malformed JSON must fail without killing the service.
    assert!(
        !call(
            dir.path(),
            Command::SaveSource {
                source: "{invalid json".into()
            }
        )
        .ok
    );
    assert!(call(dir.path(), Command::Status).ok);

    let pen = PenConfig {
        color: "#FF8844".into(),
        invalid_color: "#9CA3AF".into(),
        width: 6.0,
        opacity: 0.7,
    };
    assert!(
        call(
            dir.path(),
            Command::SetPreferences {
                pen: pen.clone(),
                stroke_button: MouseButton::X1,
                logging: LoggingConfig::default(),
            }
        )
        .ok
    );
    let invalid_pen = PenConfig {
        color: "not-a-color".into(),
        ..pen
    };
    assert!(
        !call(
            dir.path(),
            Command::SetPreferences {
                pen: invalid_pen,
                stroke_button: MouseButton::Left,
                logging: LoggingConfig::default(),
            }
        )
        .ok
    );
    assert_eq!(
        call(dir.path(), Command::GetConfig)
            .config
            .unwrap()
            .stroke_button,
        MouseButton::X1
    );

    let action = ActionSpec {
        id: "custom_test".into(),
        name: "测试动作".into(),
        gesture: "middle_click".into(),
        action: ActionKind::Keys {
            keys: "CTRL+SHIFT+T".into(),
        },
    };
    let response = call(
        dir.path(),
        Command::UpsertAction {
            package_id: "global".into(),
            action,
        },
    );
    assert!(response.ok, "{}", response.message);
    assert!(
        call(dir.path(), Command::GetConfig)
            .config
            .unwrap()
            .packages
            .iter()
            .flat_map(|p| &p.actions)
            .any(|a| a.id == "custom_test")
    );
    assert!(
        call(
            dir.path(),
            Command::RemoveAction {
                package_id: "global".into(),
                action_id: "custom_test".into()
            }
        )
        .ok
    );

    assert!(
        call(dir.path(), Command::Pause { paused: true })
            .status
            .unwrap()
            .paused
    );
    assert!(
        !call(dir.path(), Command::Pause { paused: false })
            .status
            .unwrap()
            .paused
    );
    assert!(
        !call(
            dir.path(),
            Command::Record {
                id: "new".into(),
                name: "录制".into()
            }
        )
        .ok
    );

    // Watcher rejects invalid edits once, keeps the live snapshot, then recovers.
    std::fs::write(dir.path().join("config.json"), "syntax error").unwrap();
    let started = Instant::now();
    loop {
        if call(dir.path(), Command::Status)
            .status
            .unwrap()
            .last_error
            .is_some_and(|e| e.contains("重载失败"))
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(8));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        call(dir.path(), Command::GetConfig)
            .config
            .unwrap()
            .gestures
            .len(),
        28
    );
    std::fs::write(dir.path().join("config.json"), &source).unwrap();
    let started = Instant::now();
    while call(dir.path(), Command::Status)
        .status
        .unwrap()
        .last_error
        .is_some()
    {
        assert!(started.elapsed() < Duration::from_secs(8));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(call(dir.path(), Command::Quit).ok);
    let started = Instant::now();
    while service.0.try_wait().unwrap().is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "quit did not exit"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn logging_level_is_applied_immediately_and_persists_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let service = start_service(dir.path());
    let config = call(dir.path(), Command::GetConfig).config.unwrap();
    assert_eq!(config.logging.level, LogLevel::Info);
    let log_path = dir.path().join("glint.log");
    assert!(
        std::fs::read_to_string(&log_path)
            .unwrap()
            .contains("[INFO]")
    );

    let set_level = |level| Command::SetPreferences {
        pen: config.pen.clone(),
        stroke_button: config.stroke_button,
        logging: LoggingConfig { level },
    };
    assert!(call(dir.path(), set_level(LogLevel::Off)).ok);
    let before = std::fs::read(&log_path).unwrap();
    assert!(call(dir.path(), Command::Pause { paused: true }).ok);
    assert!(call(dir.path(), Command::Pause { paused: false }).ok);
    assert_eq!(std::fs::read(&log_path).unwrap(), before);

    assert!(call(dir.path(), set_level(LogLevel::Error)).ok);
    let before = std::fs::read(&log_path).unwrap();
    assert!(call(dir.path(), Command::Pause { paused: true }).ok);
    assert_eq!(std::fs::read(&log_path).unwrap(), before);
    std::fs::write(dir.path().join("config.json"), "invalid json").unwrap();
    assert!(!call(dir.path(), Command::Reload).ok);
    let entries = std::fs::read_to_string(&log_path).unwrap();
    assert!(entries[before.len()..].contains("[ERROR]"));
    assert!(!entries[before.len()..].contains("[INFO]"));
    std::fs::write(dir.path().join("config.json"), "{}").unwrap();
    assert!(call(dir.path(), Command::Reload).ok);

    let response = call(dir.path(), set_level(LogLevel::Debug));
    assert!(response.ok, "{}", response.message);
    assert_eq!(response.config.unwrap().logging.level, LogLevel::Debug);
    let before = std::fs::read(&log_path).unwrap().len();
    assert!(call(dir.path(), Command::Pause { paused: false }).ok);
    assert!(std::fs::read_to_string(&log_path).unwrap()[before..].contains("[INFO]"));
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("overrides.json")).unwrap()).unwrap();
    assert_eq!(saved["logging"]["level"], "debug");
    drop(service);
    let _service = start_service(dir.path());
    assert_eq!(
        call(dir.path(), Command::GetConfig)
            .config
            .unwrap()
            .logging
            .level,
        LogLevel::Debug,
    );
}
