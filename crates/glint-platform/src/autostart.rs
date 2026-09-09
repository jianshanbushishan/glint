//! Per-user login startup, stored in the Windows Run registry key.
use anyhow::Result;
use std::path::Path;

/// Whether the saved startup command matches this executable and configuration.
pub fn is_enabled(executable: &Path, config_dir: &Path) -> Result<bool> {
    imp::is_enabled(executable, config_dir)
}

/// Enable or disable Glint startup for the current Windows user.
pub fn set_enabled(executable: &Path, config_dir: &Path, enabled: bool) -> Result<()> {
    imp::set_enabled(executable, config_dir, enabled)
}

// Encode one argument using the Windows command-line backslash/quote rules.
// Work on UTF-16 directly so Windows paths need not be valid Unicode.
#[cfg(any(windows, test))]
fn quote_argument(argument: &[u16]) -> Vec<u16> {
    let mut quoted = vec![b'"' as u16];
    let mut slashes = 0;
    for &unit in argument {
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        let count = if unit == b'"' as u16 {
            slashes * 2 + 1
        } else {
            slashes
        };
        quoted.extend(std::iter::repeat_n(b'\\' as u16, count));
        quoted.push(unit);
        slashes = 0;
    }
    quoted.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    quoted.push(b'"' as u16);
    quoted
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::{Context, bail, ensure};
    use std::{
        os::windows::ffi::OsStrExt,
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::{Foundation::*, System::Registry::*};

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    pub(super) fn command(executable: &Path, config_dir: &Path) -> Result<Vec<u16>> {
        ensure!(
            executable.is_absolute(),
            "Startup executable path must be absolute"
        );
        ensure!(
            config_dir.is_absolute(),
            "Startup configuration path must be absolute"
        );
        let executable: Vec<_> = executable.as_os_str().encode_wide().collect();
        let config_dir: Vec<_> = config_dir.as_os_str().encode_wide().collect();
        ensure!(
            !executable.contains(&0) && !config_dir.contains(&0),
            "Startup paths cannot contain NUL characters"
        );
        let mut value = quote_argument(&executable);
        value.extend(" --config-dir ".encode_utf16());
        value.extend(quote_argument(&config_dir));
        value.push(0);
        Ok(value)
    }

    fn check(status: WIN32_ERROR, operation: &str) -> Result<()> {
        if status != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(status as i32))
                .context(operation.to_owned());
        }
        Ok(())
    }

    pub(super) fn is_enabled(executable: &Path, config_dir: &Path) -> Result<bool> {
        let expected = command(executable, config_dir)?;
        let key = wide(RUN_KEY);
        let name = wide("Glint");
        // Retry if another process changes the value between the size and data reads.
        for _ in 0..3 {
            let mut size = 0;
            let mut kind = 0;
            let status = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    RRF_RT_ANY | RRF_NOEXPAND,
                    &mut kind,
                    null_mut(),
                    &mut size,
                )
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(false);
            }
            check(status, "Read Glint startup entry")?;
            if kind != REG_SZ {
                return Ok(false);
            }
            let mut value = vec![0u16; (size as usize).div_ceil(2) + 1];
            size = (value.len() * 2) as u32;
            let status = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    RRF_RT_ANY | RRF_NOEXPAND,
                    &mut kind,
                    value.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            if status == ERROR_MORE_DATA {
                continue;
            }
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(false);
            }
            check(status, "Read Glint startup command")?;
            value.truncate(size as usize / 2);
            return Ok(kind == REG_SZ && size % 2 == 0 && value == expected);
        }
        bail!("Glint startup entry changed repeatedly while reading it")
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    pub(super) fn set_enabled(executable: &Path, config_dir: &Path, enabled: bool) -> Result<()> {
        let path = wide(RUN_KEY);
        let name = wide("Glint");
        let mut handle = null_mut();
        if enabled {
            let value = command(executable, config_dir)?;
            ensure!(
                value.len() - 1 <= 260,
                "Startup command exceeds the Windows Run limit of 260 UTF-16 characters; use shorter executable or configuration paths"
            );
            ensure!(
                executable.is_file(),
                "Startup executable does not exist: {}",
                executable.display()
            );
            let status = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    path.as_ptr(),
                    0,
                    null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    null(),
                    &mut handle,
                    null_mut(),
                )
            };
            check(status, "Open Windows startup registry key")?;
            let key = Key(handle);
            let status = unsafe {
                RegSetValueExW(
                    key.0,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    value.as_ptr().cast(),
                    (value.len() * 2) as u32,
                )
            };
            check(status, "Enable Glint startup")
        } else {
            let status = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    path.as_ptr(),
                    0,
                    KEY_SET_VALUE,
                    &mut handle,
                )
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            check(status, "Open Windows startup registry key")?;
            let key = Key(handle);
            let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            check(status, "Disable Glint startup")
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub(super) fn is_enabled(_: &Path, _: &Path) -> Result<bool> {
        anyhow::bail!("Login startup is supported only on Windows")
    }
    pub(super) fn set_enabled(_: &Path, _: &Path, _: bool) -> Result<()> {
        anyhow::bail!("Login startup is supported only on Windows")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_windows_arguments() {
        for (input, expected) in [
            ("", "\"\""),
            (
                r"C:\Program Files\Glint\glint.exe",
                r#""C:\Program Files\Glint\glint.exe""#,
            ),
            (r"D:\配置 目录\", r#""D:\配置 目录\\""#),
            (r#"a\"b"#, r#""a\\\"b""#),
        ] {
            let actual = quote_argument(&input.encode_utf16().collect::<Vec<_>>());
            assert_eq!(String::from_utf16(&actual).unwrap(), expected);
        }
    }

    #[cfg(windows)]
    #[test]
    fn command_preserves_config_path_and_rejects_relative_paths() {
        let encoded = imp::command(
            Path::new(r"C:\Program Files\Glint\glint.exe"),
            Path::new(r"D:\配置 目录\"),
        )
        .unwrap();
        assert_eq!(
            String::from_utf16(&encoded).unwrap(),
            "\"C:\\Program Files\\Glint\\glint.exe\" --config-dir \"D:\\配置 目录\\\\\"\0"
        );
        assert!(imp::command(Path::new("glint.exe"), Path::new(r"D:\config")).is_err());
        assert!(imp::command(Path::new(r"C:\glint.exe"), Path::new("config")).is_err());
    }
}
