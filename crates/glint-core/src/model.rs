use serde::{Deserialize, Serialize};

pub const SPECIAL_GESTURES: [&str; 7] = [
    "wheel_up",
    "wheel_down",
    "left_click",
    "right_click",
    "middle_click",
    "x1_click",
    "x2_click",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    #[default]
    Right,
    Middle,
    X1,
    X2,
}

impl MouseButton {
    pub fn mask(self) -> u32 {
        match self {
            Self::Left => 1,
            Self::Right => 2,
            Self::Middle => 4,
            Self::X1 => 8,
            Self::X2 => 16,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GestureContext {
    pub window: u64,
    pub process: String,
    pub title: String,
    pub start: Point,
    pub modifiers: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PenConfig {
    pub color: String,
    pub invalid_color: String,
    pub width: f32,
    pub opacity: f32,
}
impl Default for PenConfig {
    fn default() -> Self {
        Self {
            color: "#69E0C3".into(),
            invalid_color: "#9CA3AF".into(),
            width: 4.0,
            opacity: 0.85,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GestureTemplate {
    pub id: String,
    pub name: String,
    pub points: Vec<Point>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActionKind {
    Keys {
        keys: String,
    },
    Window {
        operation: String,
    },
    Launch {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSpec {
    pub id: String,
    pub name: String,
    pub gesture: String,
    pub action: ActionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub id: String,
    pub name: String,
    #[serde(rename = "match")]
    pub patterns: Vec<String>,
    pub actions: Vec<ActionSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationProfile {
    pub id: String,
    pub name: String,
    pub process_path: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default = "inherit_global_default")]
    pub inherit_global: bool,
}

fn inherit_global_default() -> bool {
    true
}

impl ApplicationProfile {
    pub fn pattern(&self) -> anyhow::Result<String> {
        Ok(format!(
            "^{}$",
            regex::escape(&normalize_application_path(&self.process_path)?)
        ))
    }
}

/// Normalize a Windows executable path lexically, without requiring it to exist.
pub fn normalize_application_path(path: &str) -> anyhow::Result<String> {
    use anyhow::ensure;
    ensure!(
        path.len() <= 32768 && !path.chars().any(char::is_control),
        "process_path must be a full executable path"
    );
    let path = path.replace('/', "\\");
    ensure!(
        !path.ends_with('\\'),
        "process_path must name an executable, not a directory"
    );
    let (prefix, remainder) = if path.len() >= 3
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path.as_bytes()[1] == b':'
        && path.as_bytes()[2] == b'\\'
    {
        (format!("{}:\\", path[..1].to_ascii_uppercase()), &path[3..])
    } else if let Some(unc) = path.strip_prefix("\\\\") {
        let mut parts = unc.splitn(3, '\\');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        ensure!(
            valid_path_component(server) && valid_path_component(share),
            "process_path must contain a valid UNC server and share"
        );
        (
            format!("\\\\{server}\\{share}\\"),
            parts.next().unwrap_or_default(),
        )
    } else {
        anyhow::bail!("process_path must be a full executable path");
    };
    let mut components = Vec::new();
    for component in remainder.split('\\') {
        match component {
            "" | "." => {}
            ".." => {
                ensure!(
                    components.pop().is_some(),
                    "process_path cannot escape its root"
                );
            }
            _ => {
                ensure!(
                    valid_path_component(component),
                    "process_path contains an invalid or ambiguous Windows path component"
                );
                components.push(component);
            }
        }
    }
    ensure!(
        components
            .last()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".exe")),
        "process_path must be a full executable path ending in .exe"
    );
    Ok(format!("{prefix}{}", components.join("\\")))
}

fn valid_path_component(component: &str) -> bool {
    !component.is_empty()
        && !component.ends_with(['.', ' '])
        && !component.contains(['<', '>', ':', '"', '|', '?', '*'])
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: LogLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub logging: LoggingConfig,
    pub version: u32,
    pub stroke_button: MouseButton,
    pub pen: PenConfig,
    pub threshold: f32,
    pub min_distance: f64,
    pub excluded: Vec<String>,
    pub gestures: Vec<GestureTemplate>,
    pub packages: Vec<Package>,
    pub applications: Vec<ApplicationProfile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            logging: LoggingConfig::default(),
            version: 1,
            stroke_button: MouseButton::Right,
            pen: PenConfig::default(),
            threshold: 0.82,
            min_distance: 24.0,
            excluded: Vec::new(),
            gestures: crate::default_templates(),
            packages: Vec::new(),
            applications: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pen: Option<PenConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_button: Option<MouseButton>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages: Option<Vec<Package>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub action_overrides: Vec<ActionOverride>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_actions: Vec<ActionRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub application_overrides: Vec<ApplicationProfile>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_applications: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionOverride {
    pub package_id: String,
    pub action: ActionSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRef {
    pub package_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone)]
pub enum HostCommand {
    Keys(String),
    Window(String),
    Launch { program: String, args: Vec<String> },
}

pub trait ActionHost {
    fn perform(&self, command: HostCommand, context: &GestureContext) -> anyhow::Result<()>;
}

#[cfg(test)]
mod application_path_tests {
    use super::normalize_application_path;

    #[test]
    fn normalizes_windows_paths_without_accessing_the_filesystem() {
        assert_eq!(
            normalize_application_path("c:/Apps//./Old/../Editor.exe").unwrap(),
            r"C:\Apps\Editor.exe"
        );
        assert_eq!(
            normalize_application_path("//server/share/Apps/../Editor.exe").unwrap(),
            r"\\server\share\Editor.exe"
        );
        for path in [
            "C:/../Editor.exe",
            "//server/share/../Editor.exe",
            "C:/Apps./Editor.exe",
            "C:/Apps /Editor.exe",
            "C:/Editor.exe/",
            "//server/../Editor.exe",
        ] {
            assert!(
                normalize_application_path(path).is_err(),
                "accepted {path:?}"
            );
        }
    }
}
