use crate::{
    ActionHost, ActionKind, ActionSpec, Config, ConfigOverrides, GestureContext, GestureTemplate,
    HostCommand, Matcher, Package,
};
use anyhow::{Context, Result, ensure};
use std::{collections::HashSet, fs, path::Path};

const FILE_LIMIT: u64 = 2 * 1024 * 1024;

/// Loads validated JSON configuration and dispatches declarative host actions.
pub struct ConfigRuntime {
    config: Config,
}

impl ConfigRuntime {
    pub fn load(path: &Path) -> Result<Self> {
        let path = path
            .canonicalize()
            .with_context(|| format!("resolve config {}", path.display()))?;
        let mut config: Config = serde_json::from_str(&read_bounded(&path)?)
            .with_context(|| format!("parse config {}", path.display()))?;
        let directory = path.parent().unwrap_or(Path::new("."));
        let gestures_path = directory.join("gestures.json");
        if gestures_path
            .try_exists()
            .with_context(|| format!("inspect {}", gestures_path.display()))?
        {
            let templates: Vec<GestureTemplate> =
                serde_json::from_str(&read_bounded(&gestures_path)?)
                    .with_context(|| format!("parse {}", gestures_path.display()))?;
            let mut ids = HashSet::new();
            for template in templates {
                ensure!(
                    ids.insert(template.id.clone()),
                    "{}: duplicate gesture {:?}",
                    gestures_path.display(),
                    template.id
                );
                if let Some(existing) = config.gestures.iter_mut().find(|t| t.id == template.id) {
                    *existing = template;
                } else {
                    config.gestures.push(template);
                }
            }
        }
        // JSON templates are available to base bindings, too. Validate
        // those bindings before preferences can replace the package list.
        merge_application_packages(&mut config)?;
        validate(&config)
            .with_context(|| format!("validate {} and gestures.json", path.display()))?;
        let overrides_path = directory.join("overrides.json");
        if overrides_path
            .try_exists()
            .with_context(|| format!("inspect {}", overrides_path.display()))?
        {
            let overrides: ConfigOverrides = serde_json::from_str(&read_bounded(&overrides_path)?)
                .with_context(|| format!("parse {}", overrides_path.display()))?;
            if let Some(logging) = overrides.logging {
                config.logging = logging;
            }
            if let Some(pen) = overrides.pen {
                config.pen = pen;
            }
            if let Some(button) = overrides.stroke_button {
                config.stroke_button = button;
            }
            if let Some(packages) = overrides.packages {
                config.packages = packages;
            }
            let mut removed_applications = HashSet::new();
            for id in overrides.removed_applications {
                ensure!(
                    valid_id(&id) && id != "global",
                    "invalid removed application ID {id:?}"
                );
                ensure!(
                    removed_applications.insert(id.clone()),
                    "duplicate removed application {id:?}"
                );
                config.applications.retain(|app| app.id != id);
                config.packages.retain(|package| package.id != id);
            }
            let mut applications = HashSet::new();
            for application in overrides.application_overrides {
                ensure!(
                    applications.insert(application.id.clone()),
                    "duplicate application override {:?}",
                    application.id
                );
                ensure!(
                    !removed_applications.contains(&application.id),
                    "application is both removed and overridden"
                );
                if let Some(existing) = config
                    .applications
                    .iter_mut()
                    .find(|app| app.id == application.id)
                {
                    *existing = application;
                } else {
                    config.applications.push(application);
                }
            }
            merge_application_packages(&mut config)?;
            // GUI bindings can establish the global scope even for an empty config.
            // Keep it below base packages, and retain it for deletion deltas too.
            if !config.packages.iter().any(|package| package.id == "global")
                && (overrides
                    .action_overrides
                    .iter()
                    .any(|action| action.package_id == "global")
                    || overrides
                        .removed_actions
                        .iter()
                        .any(|action| action.package_id == "global"))
            {
                config.packages.insert(
                    0,
                    Package {
                        id: "global".into(),
                        name: "全局手势".into(),
                        patterns: vec![".*".into()],
                        actions: Vec::new(),
                    },
                );
            }
            let mut removed = HashSet::new();
            for reference in overrides.removed_actions {
                ensure!(
                    valid_id(&reference.package_id) && valid_id(&reference.action_id),
                    "{}: removed_actions has an invalid package or action ID",
                    overrides_path.display()
                );
                ensure!(
                    removed.insert((reference.package_id.clone(), reference.action_id.clone())),
                    "{}: duplicate removed action {}/{}",
                    overrides_path.display(),
                    reference.package_id,
                    reference.action_id
                );
                let package = config
                    .packages
                    .iter_mut()
                    .find(|p| p.id == reference.package_id)
                    .with_context(|| {
                        format!(
                            "{}: removed_actions references unknown package {:?}",
                            overrides_path.display(),
                            reference.package_id
                        )
                    })?;
                package.actions.retain(|a| a.id != reference.action_id);
            }
            let mut changed = HashSet::new();
            for replacement in overrides.action_overrides {
                ensure!(
                    valid_id(&replacement.package_id),
                    "{}: action_overrides has invalid package ID {:?}",
                    overrides_path.display(),
                    replacement.package_id
                );
                ensure!(
                    changed.insert((
                        replacement.package_id.clone(),
                        replacement.action.id.clone()
                    )),
                    "{}: duplicate action override {}/{}",
                    overrides_path.display(),
                    replacement.package_id,
                    replacement.action.id
                );
                let package = config
                    .packages
                    .iter_mut()
                    .find(|p| p.id == replacement.package_id)
                    .with_context(|| {
                        format!(
                            "{}: action_overrides references unknown package {:?}",
                            overrides_path.display(),
                            replacement.package_id
                        )
                    })?;
                if let Some(action) = package
                    .actions
                    .iter_mut()
                    .find(|a| a.id == replacement.action.id)
                {
                    *action = replacement.action;
                } else {
                    package.actions.push(replacement.action);
                }
            }
        }
        validate(&config).with_context(|| {
            format!(
                "validate config and JSON sidecars in {}",
                directory.display()
            )
        })?;
        // The native hook consumes excluded patterns before starting capture.
        // These derived exclusions never get written back into the JSON source.
        config.excluded.extend(
            config
                .applications
                .iter()
                .filter(|app| app.disabled)
                .map(|app| app.pattern())
                .collect::<Result<Vec<_>>>()?,
        );
        Ok(Self { config })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn execute(
        &self,
        action: &ActionSpec,
        context: &GestureContext,
        host: &dyn ActionHost,
    ) -> Result<()> {
        validate_action(action)?;
        let command = match &action.action {
            ActionKind::Keys { keys } => HostCommand::Keys(keys.clone()),
            ActionKind::Window { operation } => {
                HostCommand::Window(canonical_window(operation).into())
            }
            ActionKind::Launch { program, args } => HostCommand::Launch {
                program: program.clone(),
                args: args.clone(),
            },
        };
        host.perform(command, context)
            .with_context(|| format!("action {:?} ({:?})", action.id, action.name))
    }
}

fn read_bounded(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
    ensure!(
        metadata.len() <= FILE_LIMIT,
        "{} exceeds 2 MiB size limit",
        path.display()
    );
    use std::io::Read;
    let mut data = String::new();
    fs::File::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .take(FILE_LIMIT + 1)
        .read_to_string(&mut data)
        .with_context(|| format!("read UTF-8 {}", path.display()))?;
    ensure!(
        data.len() as u64 <= FILE_LIMIT,
        "{} exceeds 2 MiB size limit",
        path.display()
    );
    Ok(data)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn merge_application_packages(config: &mut Config) -> Result<()> {
    for application in &mut config.applications {
        application.process_path = crate::normalize_application_path(&application.process_path)
            .with_context(|| format!("application {:?}", application.id))?;
        if let Some(package) = config.packages.iter_mut().find(|p| p.id == application.id) {
            package.name = application.name.clone();
            package.patterns = vec![application.pattern()?];
        } else {
            config.packages.push(Package {
                id: application.id.clone(),
                name: application.name.clone(),
                patterns: vec![application.pattern()?],
                actions: Vec::new(),
            });
        }
    }
    Ok(())
}

fn validate(config: &Config) -> Result<()> {
    ensure!(
        config.applications.len() <= 256,
        "at most 256 applications are supported"
    );
    let mut application_ids = HashSet::new();
    let mut application_paths = HashSet::new();
    for application in &config.applications {
        ensure!(
            valid_id(&application.id) && application.id != "global",
            "invalid application id {:?}",
            application.id
        );
        ensure!(
            application_ids.insert(&application.id),
            "duplicate application {:?}",
            application.id
        );
        ensure!(
            !application.name.trim().is_empty() && application.name.len() <= 200,
            "application name must contain 1..200 bytes"
        );
        let normalized_path = crate::normalize_application_path(&application.process_path)?;
        ensure!(
            application_paths.insert(normalized_path.to_lowercase()),
            "duplicate application executable path {:?}",
            application.process_path
        );
    }
    ensure!(
        config.version == 1,
        "version must be 1, found {}",
        config.version
    );
    ensure!(
        config.threshold.is_finite() && (0.0..=1.0).contains(&config.threshold),
        "threshold must be between 0 and 1"
    );
    ensure!(
        config.min_distance.is_finite() && (1.0..=10000.0).contains(&config.min_distance),
        "min_distance must be between 1 and 10000 pixels"
    );
    ensure!(
        config.pen.color.len() == 7
            && config.pen.color.starts_with('#')
            && config.pen.color[1..].bytes().all(|b| b.is_ascii_hexdigit()),
        "pen.color must be #RRGGBB"
    );
    ensure!(
        config.pen.invalid_color.len() == 7
            && config.pen.invalid_color.starts_with('#')
            && config.pen.invalid_color[1..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "pen.invalid_color must be #RRGGBB"
    );
    ensure!(
        config.pen.width.is_finite() && (1.0..=32.0).contains(&config.pen.width),
        "pen.width must be between 1 and 32"
    );
    ensure!(
        config.pen.opacity.is_finite() && (0.0..=1.0).contains(&config.pen.opacity),
        "pen.opacity must be between 0 and 1"
    );
    ensure!(
        config.gestures.len() <= 256,
        "at most 256 gesture templates are supported"
    );
    let mut gestures = HashSet::new();
    for template in &config.gestures {
        ensure!(
            valid_id(&template.id),
            "gesture id {:?} must use 1..128 ASCII letters, digits, _ or -",
            template.id
        );
        ensure!(
            gestures.insert(template.id.as_str()),
            "duplicate gesture {:?}",
            template.id
        );
        ensure!(
            !template.name.trim().is_empty(),
            "gesture {:?}: name is empty",
            template.id
        );
        ensure!(
            (2..=8192).contains(&template.points.len()),
            "gesture {:?}: expected 2..8192 points",
            template.id
        );
        ensure!(
            template.points.iter().all(|p| p.x.is_finite()
                && p.y.is_finite()
                && p.x.abs() <= 1e9
                && p.y.abs() <= 1e9),
            "gesture {:?}: point coordinates must be finite and within +/-1e9",
            template.id
        );
        ensure!(
            template
                .points
                .windows(2)
                .any(|p| (p[1].x - p[0].x).hypot(p[1].y - p[0].y) > 1e-8),
            "gesture {:?}: stationary template",
            template.id
        );
    }
    ensure!(
        config.packages.len() <= 256,
        "at most 256 packages are supported"
    );
    let mut packages = HashSet::new();
    for package in &config.packages {
        ensure!(valid_id(&package.id), "invalid package id {:?}", package.id);
        ensure!(
            packages.insert(package.id.as_str()),
            "duplicate package {:?}",
            package.id
        );
        ensure!(
            !package.name.trim().is_empty(),
            "package {:?}: name is empty",
            package.id
        );
        ensure!(
            !package.patterns.is_empty(),
            "package {:?}: match must contain at least one regex",
            package.id
        );
        ensure!(
            package.patterns.len() <= 128,
            "package {:?}: at most 128 regexes",
            package.id
        );
        ensure!(
            package.actions.len() <= 512,
            "package {:?}: at most 512 actions",
            package.id
        );
        let mut actions = HashSet::new();
        let mut bindings = HashSet::new();
        for action in &package.actions {
            ensure!(
                actions.insert(action.id.as_str()),
                "package {:?}: duplicate action {:?}",
                package.id,
                action.id
            );
            ensure!(
                bindings.insert(action.gesture.as_str()),
                "package {:?}: duplicate gesture binding {:?}",
                package.id,
                action.gesture
            );
            ensure!(
                gestures.contains(action.gesture.as_str())
                    || crate::SPECIAL_GESTURES.contains(&action.gesture.as_str())
                    || action
                        .gesture
                        .split_once('+')
                        .is_some_and(|(stroke, button)| {
                            gestures.contains(stroke) && crate::SPECIAL_GESTURES.contains(&button)
                        }),
                "package {:?}, action {:?}: unknown gesture {:?}",
                package.id,
                action.id,
                action.gesture
            );
            validate_action(action)
                .with_context(|| format!("package {:?}, action {:?}", package.id, action.id))?;
        }
    }
    Matcher::new(config)?;
    Ok(())
}

fn validate_action(action: &ActionSpec) -> Result<()> {
    ensure!(valid_id(&action.id), "invalid action id {:?}", action.id);
    ensure!(!action.name.trim().is_empty(), "action name is empty");
    match &action.action {
        ActionKind::Keys { keys } => validate_keys(keys),
        ActionKind::Window { operation } => validate_window(operation),
        ActionKind::Launch { program, args } => validate_launch(program, args),
    }
}

fn validate_keys(keys: &str) -> Result<()> {
    ensure!(
        !keys.is_empty() && keys.len() <= 256,
        "keys must contain 1..256 characters"
    );
    let parts = keys.split('+').map(str::trim).collect::<Vec<_>>();
    ensure!(
        parts.len() <= 8,
        "keys supports at most 8 simultaneous keys"
    );
    let mut seen = HashSet::new();
    for part in parts {
        let upper = part.to_ascii_uppercase();
        let upper = match upper.as_str() {
            "CONTROL" => "CTRL",
            "WINDOWS" | "SUPER" => "WIN",
            "RETURN" => "ENTER",
            "ESCAPE" => "ESC",
            "DEL" => "DELETE",
            "INS" => "INSERT",
            "PGUP" => "PAGEUP",
            "PGDN" => "PAGEDOWN",
            other => other,
        };
        ensure!(
            seen.insert(upper.to_string()),
            "repeated key {part:?} in {keys:?}"
        );
        let valid = upper.len() == 1 && upper.as_bytes()[0].is_ascii_alphanumeric()
            || matches!(
                upper,
                "CTRL"
                    | "ALT"
                    | "SHIFT"
                    | "WIN"
                    | "LEFT"
                    | "RIGHT"
                    | "UP"
                    | "DOWN"
                    | "ENTER"
                    | "ESC"
                    | "TAB"
                    | "SPACE"
                    | "BACKSPACE"
                    | "DELETE"
                    | "INSERT"
                    | "HOME"
                    | "END"
                    | "PAGEUP"
                    | "PAGEDOWN"
                    | "PRINTSCREEN"
                    | "PLUS"
                    | "MINUS"
                    | "COMMA"
                    | "PERIOD"
                    | "VOLUMEUP"
                    | "VOLUMEDOWN"
                    | "VOLUMEMUTE"
                    | "MEDIAPLAYPAUSE"
                    | "MEDIANEXT"
                    | "MEDIAPREVIOUS"
            )
            || upper
                .strip_prefix('F')
                .and_then(|v| v.parse::<u8>().ok())
                .is_some_and(|v| (1..=24).contains(&v));
        ensure!(valid, "unsupported key {part:?} in {keys:?}");
    }
    Ok(())
}
fn validate_window(operation: &str) -> Result<()> {
    ensure!(
        matches!(
            operation,
            "minimize"
                | "maximize"
                | "toggle_maximize"
                | "restore"
                | "close"
                | "force_close"
                | "topmost"
                | "toggle_topmost"
        ),
        "unsupported window operation {operation:?}"
    );
    Ok(())
}
fn canonical_window(operation: &str) -> &str {
    if operation == "topmost" {
        "toggle_topmost"
    } else {
        operation
    }
}
#[test]
fn screenshot_and_maximize_toggle_actions_validate() {
    assert!(validate_keys("PRINTSCREEN").is_ok());
    assert!(validate_keys("Alt+PrintScreen").is_ok());
    assert!(validate_keys("PRINTSCREEN+PrintScreen").is_err());
    assert!(validate_window("toggle_maximize").is_ok());
    assert!(validate_window("force_close").is_ok());
    assert_eq!(canonical_window("toggle_maximize"), "toggle_maximize");
}
fn validate_launch(program: &str, args: &[String]) -> Result<()> {
    ensure!(
        !program.trim().is_empty() && program.len() <= 32767 && !program.contains('\0'),
        "launch.program must be nonempty, contain no NUL, and fit 32767 bytes"
    );
    ensure!(
        args.len() <= 128 && args.iter().all(|s| s.len() <= 32767 && !s.contains('\0')),
        "launch.args supports at most 128 NUL-free arguments of <=32767 bytes"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicU64, Ordering},
        },
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new(value: Value) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "glint-json-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            let fixture = Self(dir);
            fixture.write("config.json", value);
            fixture
        }
        fn write(&self, name: &str, value: Value) {
            fs::write(self.0.join(name), value.to_string()).unwrap();
        }
        fn load(&self) -> Result<ConfigRuntime> {
            ConfigRuntime::load(&self.0.join("config.json"))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[derive(Default)]
    struct Host(Mutex<Vec<(HostCommand, GestureContext)>>);
    impl ActionHost for Host {
        fn perform(&self, command: HostCommand, context: &GestureContext) -> Result<()> {
            self.0.lock().unwrap().push((command, context.clone()));
            Ok(())
        }
    }
    fn action(id: &str, gesture: &str, kind: Value) -> Value {
        json!({"id":id,"name":id,"gesture":gesture,"action":kind})
    }
    fn source(actions: Vec<Value>) -> Value {
        json!({"packages":[{"id":"test","name":"Test","match":[".*"],"actions":actions}]})
    }
    #[test]
    fn json_defaults_and_logging_levels_round_trip() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<ConfigRuntime>();
        let fixture = Fixture::new(json!({}));
        assert_eq!(
            fixture.load().unwrap().config().logging.level,
            crate::LogLevel::Info
        );
        assert_eq!(
            serde_json::to_string(&ConfigOverrides::default()).unwrap(),
            "{}"
        );
        for level in ["off", "error", "warn", "info", "debug", "trace"] {
            fixture.write("overrides.json", json!({"logging":{"level":level}}));
            let runtime = fixture.load().unwrap();
            assert_eq!(
                serde_json::to_value(&runtime.config().logging).unwrap(),
                json!({"level":level})
            );
        }
        fixture.write("overrides.json", json!({"logging":{"level":"verbose"}}));
        assert!(fixture.load().is_err());
    }
    #[test]
    fn declarative_actions_dispatch_context_and_canonical_operations() {
        let fixture = Fixture::new(source(vec![
            action("copy", "left", json!({"type":"keys","keys":"CTRL+C"})),
            action(
                "pin",
                "right",
                json!({"type":"window","operation":"topmost"}),
            ),
            action(
                "launch",
                "up",
                json!({"type":"launch","program":"notepad.exe","args":["hello.txt"]}),
            ),
        ]));
        let runtime = fixture.load().unwrap();
        let host = Host::default();
        let context = GestureContext {
            window: 42,
            process: "test.exe".into(),
            ..Default::default()
        };
        for action in &runtime.config().packages[0].actions {
            runtime.execute(action, &context, &host).unwrap();
        }
        let calls = host.0.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(matches!(&calls[0].0, HostCommand::Keys(keys) if keys == "CTRL+C"));
        assert!(
            matches!(&calls[1].0, HostCommand::Window(operation) if operation == "toggle_topmost")
        );
        assert!(
            matches!(&calls[2].0, HostCommand::Launch { program, args } if program == "notepad.exe" && args == &["hello.txt"])
        );
        assert!(
            calls
                .iter()
                .all(|(_, ctx)| ctx.window == 42 && ctx.process == "test.exe")
        );
    }
    #[test]
    fn malformed_or_invalid_config_reports_context() {
        for (value, expected) in [
            (json!({"typo":1}), "typo"),
            (json!({"pen":{"invalid_color":"gray"}}), "pen.invalid_color"),
            (
                source(vec![action(
                    "a",
                    "left",
                    json!({"type":"keys","keys":"BOGUS"}),
                )]),
                "test",
            ),
            (
                source(vec![action(
                    "a",
                    "left",
                    json!({"type":"lua","function":"test/a"}),
                )]),
                "unknown variant",
            ),
        ] {
            let error = Fixture::new(value).load().err().unwrap();
            assert!(format!("{error:#}").contains(expected), "{error:#}");
        }
        let fixture = Fixture::new(json!({}));
        fs::write(fixture.0.join("config.json"), "return {}").unwrap();
        assert!(format!("{:#}", fixture.load().err().unwrap()).contains("config.json"));
        fs::write(
            fixture.0.join("config.json"),
            " ".repeat(FILE_LIMIT as usize + 1),
        )
        .unwrap();
        assert!(format!("{:#}", fixture.load().err().unwrap()).contains("2 MiB"));
    }
    #[test]
    fn compound_bindings_require_one_known_stroke_and_event() {
        for gesture in [
            "missing+left_click",
            "wheel_up+wheel_down",
            "up+wheel_up+wheel_down",
            "up+typo",
            "up+left_click+left_click",
        ] {
            assert!(
                Fixture::new(source(vec![action(
                    "a",
                    gesture,
                    json!({"type":"keys","keys":"F11"})
                )]))
                .load()
                .is_err()
            );
        }
    }
    #[test]
    fn sidecars_merge_gestures_preferences_and_action_deltas() {
        let base = source(vec![
            action("a", "left", json!({"type":"keys","keys":"CTRL+C"})),
            action("b", "right", json!({"type":"keys","keys":"CTRL+V"})),
            action("c", "up", json!({"type":"keys","keys":"CTRL+HOME"})),
        ]);
        let fixture = Fixture::new(base.clone());
        fixture.write(
            "gestures.json",
            json!([{"id":"left","name":"Custom left","points":[{"x":20,"y":0},{"x":0,"y":0}]}]),
        );
        fixture.write("overrides.json",json!({"pen":{"color":"#123456"},"stroke_button":"middle","removed_actions":[{"package_id":"test","action_id":"b"}],"action_overrides":[{"package_id":"test","action":action("a","down",json!({"type":"keys","keys":"CTRL+X"}))}]}));
        let runtime = fixture.load().unwrap();
        let config = runtime.config();
        assert_eq!(config.pen.color, "#123456");
        assert_eq!(config.pen.invalid_color, "#9CA3AF");
        assert_eq!(config.stroke_button, crate::MouseButton::Middle);
        assert_eq!(config.gestures[0].name, "Custom left");
        assert_eq!(
            config.packages[0]
                .actions
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "c"]
        );
        assert_eq!(config.packages[0].actions[0].gesture, "down");
        let mut updated = base;
        updated["packages"][0]["actions"][2]["action"]["keys"] = json!("CTRL+END");
        fixture.write("config.json", updated);
        assert!(
            matches!(&fixture.load().unwrap().config().packages[0].actions[1].action,ActionKind::Keys { keys } if keys == "CTRL+END")
        );
    }
    #[test]
    fn sidecar_unknown_packages_and_duplicate_gestures_are_rejected() {
        let fixture = Fixture::new(json!({}));
        fixture.write(
            "overrides.json",
            json!({"removed_actions":[{"package_id":"absent","action_id":"a"}]}),
        );
        let error = format!("{:#}", fixture.load().err().unwrap());
        assert!(error.contains("overrides.json") && error.contains("unknown package"));
        fixture.write("overrides.json", json!({}));
        let template = json!({"id":"left","name":"Left","points":[{"x":10,"y":0},{"x":0,"y":0}]});
        fixture.write("gestures.json", json!([template, template]));
        assert!(format!("{:#}", fixture.load().err().unwrap()).contains("duplicate gesture"));
    }
    #[test]
    fn gui_global_scope_and_application_overrides_preserve_base_actions() {
        let fixture = Fixture::new(source(vec![action(
            "a",
            "left",
            json!({"type":"keys","keys":"CTRL+C"}),
        )]));
        fixture.write("overrides.json",json!({"application_overrides":[{"id":"test","name":"Editor","process_path":"c:/Apps/./old/../editor.exe","disabled":true}],"action_overrides":[{"package_id":"global","action":action("global_copy","left",json!({"type":"keys","keys":"CTRL+C"}))}]}));
        let runtime = fixture.load().unwrap();
        let config = runtime.config();
        assert_eq!(config.packages[0].id, "global");
        assert_eq!(config.packages[1].actions[0].id, "a");
        assert_eq!(config.applications[0].process_path, r"C:\Apps\editor.exe");
        assert!(
            regex::RegexSetBuilder::new(&config.excluded)
                .case_insensitive(true)
                .build()
                .unwrap()
                .is_match(r"C:\APPS\EDITOR.EXE")
        );
        fixture.write("overrides.json", json!({"removed_applications":["test"]}));
        assert!(fixture.load().unwrap().config().packages.is_empty());
    }
    #[test]
    fn bundled_json_config_loads() {
        let runtime = ConfigRuntime::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/config.json"),
        )
        .unwrap();
        assert!(!runtime.config().packages.is_empty());
    }
}
