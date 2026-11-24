//! Shared file backend for the standard `log` facade.
use crate::LogLevel;
use anyhow::{Context, Result};
use log::{LevelFilter, Log, Metadata, Record};
use std::{
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

const FILE_LIMIT: u64 = 1_048_576;

struct FileLogger {
    path: PathBuf,
    state: Mutex<Option<File>>,
}

impl FileLogger {
    fn open(&self) -> std::io::Result<File> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
    }

    fn write(&self, record: &Record<'_>) -> std::io::Result<()> {
        // Format outside the mutex to avoid re-entrant deadlocks and minimize lock hold time.
        let mut line = String::with_capacity(128);
        let _ = writeln!(
            line,
            "{} [{}] {}: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            record.level(),
            record.target(),
            record.args()
        );
        let mut guard = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let needs_rotate = match guard.as_ref() {
            Some(file) => file
                .metadata()
                .is_ok_and(|metadata| metadata.len() >= FILE_LIMIT),
            None => fs::metadata(&self.path).is_ok_and(|metadata| metadata.len() >= FILE_LIMIT),
        };
        if needs_rotate {
            // Release the open handle before renaming on Windows.
            *guard = None;
            let previous = self.path.with_extension("previous.log");
            if let Err(error) = fs::rename(&self.path, &previous)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(error);
            }
        }
        let file = match guard.as_mut() {
            Some(file) => file,
            None => guard.insert(self.open()?),
        };
        file.write_all(line.as_bytes())
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata())
            && let Err(error) = self.write(record)
        {
            eprintln!("write log {}: {error}", self.path.display());
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.state.lock()
            && let Some(file) = guard.as_mut()
        {
            let _ = file.flush();
        }
    }
}

/// Install once at process startup, before loading configuration.
pub fn init(dir: &Path, filename: &str) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create log directory {}", dir.display()))?;
    let path = dir.join(filename);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open log {}", path.display()))?;
    log::set_boxed_logger(Box::new(FileLogger {
        path,
        state: Mutex::new(Some(file)),
    }))
    .context("install file logger")?;
    set_level(LogLevel::Info);
    Ok(())
}

/// Change the filter immediately without replacing the installed logger.
pub fn set_level(level: LogLevel) {
    log::set_max_level(match level {
        LogLevel::Off => LevelFilter::Off,
        LogLevel::Error => LevelFilter::Error,
        LogLevel::Warn => LevelFilter::Warn,
        LogLevel::Info => LevelFilter::Info,
        LogLevel::Debug => LevelFilter::Debug,
        LogLevel::Trace => LevelFilter::Trace,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_filters_records_and_applies_level_changes() {
        let dir = tempfile::tempdir().unwrap();
        init(dir.path(), "filter.log").unwrap();
        let path = dir.path().join("filter.log");
        log::info!("default-info");
        log::debug!("hidden-debug");
        let initial = fs::read_to_string(&path).unwrap();
        assert!(initial.contains("default-info"));
        assert!(!initial.contains("hidden-debug"));
        set_level(LogLevel::Debug);
        log::debug!("visible-debug-action");
        assert!(fs::read_to_string(&path).unwrap().contains("[DEBUG]"));
        set_level(LogLevel::Error);
        log::info!("hidden-info");
        log::error!("visible-error");
        let errors = fs::read_to_string(&path).unwrap();
        assert!(!errors.contains("hidden-info"));
        assert!(errors.contains("visible-error"));
        set_level(LogLevel::Off);
        log::error!("hidden-error");
        assert_eq!(fs::read_to_string(&path).unwrap(), errors);
        // Leave logging disabled: the installed logger outlives the temp directory.
    }

    #[test]
    fn records_include_level_target_and_rotate_repeatedly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glint.log");
        let logger = FileLogger {
            path: path.clone(),
            state: Mutex::new(None),
        };
        logger
            .write(
                &Record::builder()
                    .level(log::Level::Debug)
                    .target("actions")
                    .args(format_args!("executed copy"))
                    .build(),
            )
            .unwrap();
        let entry = fs::read_to_string(&path).unwrap();
        assert!(entry.contains("[DEBUG] actions: executed copy"));
        for _ in 0..2 {
            fs::write(&path, vec![b'x'; FILE_LIMIT as usize]).unwrap();
            logger
                .write(
                    &Record::builder()
                        .level(log::Level::Info)
                        .target("engine")
                        .args(format_args!("started"))
                        .build(),
                )
                .unwrap();
            assert_eq!(
                fs::metadata(path.with_extension("previous.log"))
                    .unwrap()
                    .len(),
                FILE_LIMIT
            );
            assert!(
                fs::read_to_string(&path)
                    .unwrap()
                    .contains("[INFO] engine: started")
            );
        }
    }
}
