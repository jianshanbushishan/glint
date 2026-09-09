use glint_core::{
    ActionSpec, ApplicationProfile, Config, GestureContext, LoggingConfig, MouseButton, PenConfig,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Status,
    GetConfig,
    GetSource,
    SaveSource {
        source: String,
    },
    Pause {
        paused: bool,
    },
    Reload,
    ResetOverrides,
    Record {
        id: String,
        name: String,
    },
    CancelRecording,
    CaptureWindow {
        delay_ms: u64,
    },
    SetPreferences {
        pen: PenConfig,
        stroke_button: MouseButton,
        #[serde(default)]
        logging: LoggingConfig,
    },
    UpsertAction {
        package_id: String,
        action: ActionSpec,
    },
    RemoveAction {
        package_id: String,
        action_id: String,
    },
    ApplyScope {
        package_id: String,
        application: Option<ApplicationProfile>,
        upsert_actions: Vec<ActionSpec>,
        remove_actions: Vec<String>,
    },
    RemoveApplication {
        package_id: String,
    },
    Quit,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub running: bool,
    #[serde(default)]
    pub engine_pid: Option<u32>,
    pub paused: bool,
    pub recording: Option<String>,
    pub last_gesture: Option<String>,
    pub last_action: Option<String>,
    pub last_error: Option<String>,
    pub gesture_count: usize,
    pub action_count: usize,
    pub recent_events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub message: String,
    pub status: Option<Status>,
    pub config: Option<Config>,
    pub source: Option<String>,
    pub context: Option<GestureContext>,
}

impl Response {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            status: None,
            config: None,
            source: None,
            context: None,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            ..Self::success(message)
        }
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("GLINT_CONFIG_DIR") {
        return path.into();
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
        })
        .join("Glint")
}

mod transport;
pub use transport::{Server, request, serve};

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn validate_template_id(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= 80
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "手势 ID 仅支持英文字母、数字、短横线和下划线（1–80 字符）"
    );
    Ok(())
}

pub fn write_atomic(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    use std::io::Write;
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    Ok(())
}
