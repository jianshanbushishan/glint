//! Shared file backend for the standard `log` facade.
use crate::LogLevel;
use anyhow::{Context, Result};
use log::{LevelFilter, Log, Metadata, Record};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

const FILE_LIMIT: u64 = 1_048_576;

struct FileLogger {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileLogger {
    fn write(&self, record: &Record<'_>) -> std::io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
        // Open per entry so rotation works on Windows without an open file handle.
        if fs::metadata(&self.path).is_ok_and(|metadata| metadata.len() >= FILE_LIMIT) {
            let previous = self.path.with_extension("previous.log");
            match fs::remove_file(&previous) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::rename(&self.path, previous)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(
            file,
            "{} [{}] {}: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            record.level(),
            record.target(),
            record.args()
        )
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

    fn flush(&self) {} // Each entry is written directly to its own file handle.
}

/// Install once at process startup, before loading configuration.
pub fn init(dir: &Path, filename: &str) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create log directory {}", dir.display()))?;
    let path = dir.join(filename);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open log {}", path.display()))?;
    log::set_boxed_logger(Box::new(FileLogger {
        path,
        lock: Mutex::new(()),
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
            lock: Mutex::new(()),
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
