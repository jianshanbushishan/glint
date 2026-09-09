use glint_core::{ActionKind, ActionSpec, Config};
use std::collections::BTreeMap;

pub const WINDOW_OPERATION_IDS: [&str; 7] = [
    "minimize",
    "maximize",
    "toggle_maximize",
    "restore",
    "close",
    "force_close",
    "toggle_topmost",
];

pub fn window_operation_index(operation: &str) -> usize {
    let operation = match operation {
        "topmost" => "toggle_topmost",
        other => other,
    };
    WINDOW_OPERATION_IDS
        .iter()
        .position(|id| *id == operation)
        .unwrap_or(2)
}

#[derive(Debug, Clone)]
pub struct GestureRow {
    pub source_package: String,
    pub action: ActionSpec,
    pub inherited: bool,
}

/// One effective action per gesture, with scope-specific actions taking priority.
/// Sorting by gesture keeps the list stable when packages or actions are reordered.
pub fn gesture_rows(config: &Config, scope: &str, inherit_global: bool) -> Vec<GestureRow> {
    let mut rows = BTreeMap::new();
    if scope != "global" && inherit_global {
        for package in config.packages.iter().filter(|p| p.id == "global") {
            for action in &package.actions {
                rows.insert(
                    action.gesture.clone(),
                    GestureRow {
                        source_package: package.id.clone(),
                        action: action.clone(),
                        inherited: true,
                    },
                );
            }
        }
    }
    for package in config.packages.iter().filter(|p| p.id == scope) {
        for action in &package.actions {
            rows.insert(
                action.gesture.clone(),
                GestureRow {
                    source_package: package.id.clone(),
                    action: action.clone(),
                    inherited: false,
                },
            );
        }
    }
    rows.into_values().collect()
}

pub fn action_summary(action: &ActionKind) -> String {
    match action {
        ActionKind::Keys { keys } => format!("快捷键：{keys}"),
        ActionKind::Window { operation } => match operation.as_str() {
            "minimize" => "最小化窗口",
            "maximize" => "最大化窗口",
            "toggle_maximize" => "切换最大化",
            "restore" => "还原窗口",
            "close" => "关闭窗口",
            "force_close" => "强制关闭窗口",
            "topmost" | "toggle_topmost" => "切换窗口置顶",
            other => return format!("窗口操作：{other}"),
        }
        .to_owned(),
        ActionKind::Launch { program, .. } => {
            let filename = program
                .rsplit(['\\', '/'])
                .find(|part| !part.is_empty())
                .unwrap_or(program);
            format!("启动：{filename}")
        }
    }
}

pub fn split_gesture(gesture: &str) -> (String, String) {
    let (base, extra) = gesture.split_once('+').unwrap_or((gesture, ""));
    (base.to_owned(), extra.to_owned())
}

pub fn join_gesture(base: &str, extra: &str) -> String {
    if extra.is_empty() {
        base.to_owned()
    } else {
        format!("{base}+{extra}")
    }
}

pub fn gesture_label(gesture: &str) -> String {
    static TEMPLATES: std::sync::OnceLock<Vec<glint_core::GestureTemplate>> =
        std::sync::OnceLock::new();
    let templates = TEMPLATES.get_or_init(glint_core::default_templates);
    gesture
        .split('+')
        .map(|part| match part {
            "wheel_up" => "滚轮向上",
            "wheel_down" => "滚轮向下",
            "left_click" => "左键",
            "right_click" => "右键",
            "middle_click" => "中键",
            "x1_click" => "侧键 1",
            "x2_click" => "侧键 2",
            _ => templates
                .iter()
                .find(|t| t.id == part)
                .map(|t| t.name.as_str())
                .unwrap_or(part),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Parse argument text using Windows double-quote/backslash rules, without a shell.
/// Existing JSON string arrays remain accepted for compatibility with older settings.
pub fn parse_launch_args(input: &str) -> Result<Vec<String>, String> {
    let input = input.trim();
    if input.starts_with('[')
        && let Ok(args) = serde_json::from_str::<Vec<String>>(input)
    {
        return Ok(args);
    }
    let mut chars = input.chars().peekable();
    let mut args = Vec::new();
    while chars.peek().is_some() {
        while chars
            .peek()
            .is_some_and(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))
        {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut arg = String::new();
        let mut quoted = false;
        while let Some(&ch) = chars.peek() {
            if !quoted && matches!(ch, ' ' | '\t' | '\r' | '\n') {
                break;
            }
            if ch == '\\' {
                let mut slashes = 0;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    slashes += 1;
                }
                if chars.peek() == Some(&'"') {
                    arg.extend(std::iter::repeat_n('\\', slashes / 2));
                    if slashes % 2 == 1 {
                        chars.next();
                        arg.push('"');
                    } else {
                        chars.next();
                        if quoted && chars.peek() == Some(&'"') {
                            chars.next();
                            arg.push('"');
                        } else {
                            quoted = !quoted;
                        }
                    }
                } else {
                    arg.extend(std::iter::repeat_n('\\', slashes));
                }
            } else if ch == '"' {
                chars.next();
                if quoted && chars.peek() == Some(&'"') {
                    chars.next();
                    arg.push('"');
                } else {
                    quoted = !quoted;
                }
            } else {
                chars.next();
                arg.push(ch);
            }
        }
        if quoted {
            return Err("参数中的双引号未闭合".into());
        }
        args.push(arg);
    }
    Ok(args)
}

/// Quote every argument so empty strings and trailing backslashes round-trip.
pub fn format_launch_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            let mut result = String::from("\"");
            let mut slashes = 0;
            for ch in arg.chars() {
                if ch == '\\' {
                    slashes += 1;
                } else {
                    result.extend(std::iter::repeat_n(
                        '\\',
                        if ch == '"' { slashes * 2 + 1 } else { slashes },
                    ));
                    slashes = 0;
                    result.push(ch);
                }
            }
            result.extend(std::iter::repeat_n('\\', slashes * 2));
            result.push('"');
            result
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use glint_core::Package;

    #[test]
    fn window_operation_editing_preserves_every_supported_operation() {
        for operation in WINDOW_OPERATION_IDS {
            assert_eq!(
                WINDOW_OPERATION_IDS[window_operation_index(operation)],
                operation
            );
        }
        assert_eq!(
            WINDOW_OPERATION_IDS[window_operation_index("topmost")],
            "toggle_topmost"
        );
    }

    fn package(id: &str, actions: &[(&str, &str)]) -> Package {
        Package {
            id: id.into(),
            name: id.into(),
            patterns: vec![],
            actions: actions
                .iter()
                .map(|(id, gesture)| ActionSpec {
                    id: (*id).into(),
                    name: (*id).into(),
                    gesture: (*gesture).into(),
                    action: ActionKind::Keys {
                        keys: "CTRL+C".into(),
                    },
                })
                .collect(),
        }
    }

    fn config() -> Config {
        Config {
            packages: vec![
                package(
                    "browser",
                    &[("browser_left", "left"), ("browser_right", "right")],
                ),
                package("global", &[("global_up", "up"), ("global_left", "left")]),
                package("editor", &[("editor_down", "down")]),
            ],
            ..Config::default()
        }
    }

    #[test]
    fn scope_overrides_global_even_when_global_is_later() {
        let rows = gesture_rows(&config(), "browser", true);
        assert_eq!(
            rows.iter()
                .map(|r| r.action.id.as_str())
                .collect::<Vec<_>>(),
            ["browser_left", "browser_right", "global_up"]
        );
        assert!(!rows[0].inherited);
        assert_eq!(rows[0].source_package, "browser");
        assert!(rows[2].inherited);
        assert_eq!(rows[2].source_package, "global");
    }

    #[test]
    fn global_never_inherits_other_packages() {
        for inherit in [true, false] {
            let rows = gesture_rows(&config(), "global", inherit);
            assert_eq!(rows.len(), 2);
            assert!(
                rows.iter()
                    .all(|r| !r.inherited && r.source_package == "global")
            );
        }
    }

    #[test]
    fn disabled_inheritance_and_missing_scopes() {
        assert_eq!(gesture_rows(&config(), "editor", false).len(), 1);
        assert!(gesture_rows(&config(), "new_app", false).is_empty());
        assert_eq!(gesture_rows(&config(), "new_app", true).len(), 2);
        assert_eq!(gesture_rows(&config(), "editor", true).len(), 3);
    }

    #[test]
    fn last_duplicate_matches_runtime_priority() {
        let config = Config {
            packages: vec![package("global", &[("old", "left"), ("new", "left")])],
            ..Config::default()
        };
        assert_eq!(gesture_rows(&config, "global", true)[0].action.id, "new");
    }

    #[test]
    fn gesture_editing_preserves_special_and_custom_gestures() {
        for gesture in ["left", "wheel_up", "left+wheel_down", "custom+middle_click"] {
            let (base, extra) = split_gesture(gesture);
            assert_eq!(join_gesture(&base, &extra), gesture);
        }
        assert_eq!(split_gesture("wheel_up"), ("wheel_up".into(), "".into()));
        assert_eq!(gesture_label("left+wheel_up"), "向左 + 滚轮向上");
        assert_eq!(gesture_label("custom"), "custom");
        for special in glint_core::SPECIAL_GESTURES {
            assert_ne!(gesture_label(special), special);
        }
    }

    #[test]
    fn summaries_describe_the_action() {
        assert_eq!(
            action_summary(&ActionKind::Launch {
                program: r"C:\Program Files\Editor\edit.exe".into(),
                args: vec![]
            }),
            "启动：edit.exe"
        );
        assert_eq!(
            action_summary(&ActionKind::Window {
                operation: "toggle_maximize".into()
            }),
            "切换最大化"
        );
        assert_eq!(
            action_summary(&ActionKind::Keys {
                keys: "ALT+LEFT".into()
            }),
            "快捷键：ALT+LEFT"
        );
    }

    #[test]
    fn parses_normal_command_line_and_legacy_json() {
        assert_eq!(
            parse_launch_args(r#"--file "C:\Program Files\a.txt" "" plain" quoted""#).unwrap(),
            ["--file", r"C:\Program Files\a.txt", "", "plain quoted"]
        );
        assert_eq!(
            parse_launch_args(r#"["one", "two words", ""]"#).unwrap(),
            ["one", "two words", ""]
        );
        assert!(parse_launch_args("\"unclosed").is_err());
        assert!(parse_launch_args("  \t ").unwrap().is_empty());
        assert_eq!(parse_launch_args(r#""a""b""#).unwrap(), ["a\"b"]);
    }

    #[test]
    fn formatted_arguments_round_trip_windows_edge_cases() {
        let args: Vec<String> = [
            "",
            "simple",
            "two words",
            "a\"b",
            r"C:\Program Files\",
            r"C:\plain\",
            "\\\\\"",
            "中文 参数",
            "line\nbreak",
            "[\"legacy\"]",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(parse_launch_args(&format_launch_args(&args)).unwrap(), args);
        assert_eq!(format_launch_args(&[]), "");
    }
}
