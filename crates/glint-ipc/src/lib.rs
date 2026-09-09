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
    anyhow::ensure!(
        !glint_core::SPECIAL_GESTURES.contains(&id),
        "手势 ID 不能使用鼠标按钮或滚轮事件的保留名称"
    );
    Ok(())
}

pub fn write_atomic(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    // Each writer owns a unique file on the destination volume. NamedTempFile
    // removes it on failure. Close the handle before replacing on Windows so
    // another writer can immediately replace the destination as well.
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    file.write_all(data)?;
    file.as_file().sync_all()?;
    let temporary = file.into_temp_path();
    std::fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_ids_reject_reserved_input_events() {
        for id in glint_core::SPECIAL_GESTURES {
            assert!(validate_template_id(id).is_err(), "{id}");
        }
        assert!(validate_template_id("recorded_left").is_ok());
    }

    #[test]
    fn concurrent_atomic_writers_publish_complete_files_and_clean_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for byte in b'a'..=b'h' {
                let path = &path;
                let barrier = &barrier;
                scope.spawn(move || {
                    let payload = vec![byte; 64 * 1024];
                    barrier.wait();
                    for _ in 0..8 {
                        write_atomic(path, &payload).unwrap();
                        let published = std::fs::read(path).unwrap();
                        assert_eq!(published.len(), payload.len());
                        assert!(published.iter().all(|&value| value == published[0]));
                    }
                });
            }
        });
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_atomic_replace_removes_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("directory");
        std::fs::create_dir(&destination).unwrap();
        assert!(write_atomic(&destination, b"data").is_err());
        assert!(destination.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
