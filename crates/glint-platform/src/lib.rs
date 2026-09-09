//! Windows integration, independent of the settings UI.
pub mod autostart;

use glint_core::{GestureContext, Point};

#[derive(Debug, Clone)]
pub enum PlatformEvent {
    Gesture {
        points: Vec<Point>,
        context: GestureContext,
        recording: bool,
    },
    Special {
        gesture: String,
        /// Empty for a direct button/wheel action; otherwise the preceding stroke.
        points: Vec<Point>,
        context: GestureContext,
    },
    Tray(TrayAction),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenSettings,
    TogglePause,
    Reload,
    Quit,
}

mod input;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{Platform, WindowsHost, context_at_cursor};

#[cfg(not(windows))]
mod unsupported {
    use super::*;
    use anyhow::{Result, bail};
    use glint_core::{ActionHost, Config, HostCommand};
    use std::path::PathBuf;
    pub struct Platform;
    impl Platform {
        pub fn start(_: crossbeam_channel::Sender<PlatformEvent>, _: PathBuf) -> Result<Self> {
            bail!("Glint's global mouse engine currently requires Windows")
        }
        pub fn configure(&self, _: &Config) -> Result<()> {
            bail!("Windows is required")
        }
        pub fn pause(&self, _: bool) -> Result<()> {
            bail!("Windows is required")
        }
        pub fn record(&self, _: bool) -> Result<()> {
            bail!("Windows is required")
        }
        pub fn shutdown(&self) -> Result<()> {
            Ok(())
        }
    }
    pub struct WindowsHost;
    impl ActionHost for WindowsHost {
        fn perform(&self, _: HostCommand, _: &GestureContext) -> Result<()> {
            bail!("Windows is required")
        }
    }
    pub fn context_at_cursor() -> Result<GestureContext> {
        bail!("Windows is required")
    }
}
#[cfg(not(windows))]
pub use unsupported::*;
