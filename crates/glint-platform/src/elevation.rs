//! Explicit, UAC-mediated restart for gestures in elevated applications.
use anyhow::Result;
use std::path::Path;

pub fn is_elevated() -> Result<bool> {
    imp::is_elevated()
}

/// Start the replacement before asking the existing engine to stop.
/// A rejected UAC prompt therefore leaves the existing engine untouched.
pub fn restart_elevated(config_dir: &Path) -> Result<()> {
    imp::restart_elevated(config_dir)
}

/// Pin the original process before stopping it, so PID reuse cannot make us
/// wait on an unrelated process. The callback must request a graceful stop.
pub fn wait_for_exit(pid: u32, stop: impl FnOnce() -> Result<()>) -> Result<()> {
    imp::wait_for_exit(pid, stop)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::{Context, bail, ensure};
    use std::{mem::size_of, os::windows::ffi::OsStrExt, ptr::null_mut};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
        UI::{
            Shell::{
                SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
            },
            WindowsAndMessaging::SW_HIDE,
        },
    };

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    pub(super) fn is_elevated() -> Result<bool> {
        let mut token = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(std::io::Error::last_os_error()).context("无法读取 Glint 运行权限");
        }
        let token = Handle(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0;
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error()).context("无法读取 Glint 管理员权限");
        }
        Ok(elevation.TokenIsElevated != 0)
    }

    pub(super) fn parameters(config_dir: &Path, pid: u32) -> Result<Vec<u16>> {
        ensure!(config_dir.is_absolute(), "配置目录必须为绝对路径");
        let path: Vec<u16> = config_dir.as_os_str().encode_wide().collect();
        ensure!(!path.contains(&0), "配置目录不能包含 NUL 字符");
        let mut parameters: Vec<u16> = "--config-dir ".encode_utf16().collect();
        parameters.extend(crate::autostart::quote_argument(&path));
        parameters.extend(format!(" --replace-process {pid} run").encode_utf16());
        parameters.push(0);
        Ok(parameters)
    }

    pub(super) fn restart_elevated(config_dir: &Path) -> Result<()> {
        let executable = std::env::current_exe().context("无法定位 Glint 后台程序")?;
        let executable: Vec<u16> = executable
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let parameters = parameters(config_dir, std::process::id())?;
        let verb: Vec<u16> = "runas\0".encode_utf16().collect();
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: verb.as_ptr(),
            lpFile: executable.as_ptr(),
            lpParameters: parameters.as_ptr(),
            nShow: SW_HIDE,
            ..Default::default()
        };
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
                bail!("已取消管理员权限授权，原后台仍在运行");
            }
            return Err(error).context("无法以管理员权限重新启动 Glint，原后台仍在运行");
        }
        let _process = Handle(info.hProcess);
        Ok(())
    }

    pub(super) fn wait_for_exit(pid: u32, stop: impl FnOnce() -> Result<()>) -> Result<()> {
        ensure!(
            pid != 0 && pid != std::process::id(),
            "替换的后台进程编号无效"
        );
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if process.is_null() {
            return Err(std::io::Error::last_os_error()).context("无法等待原后台退出");
        }
        let process = Handle(process);
        stop()?;
        match unsafe { WaitForSingleObject(process.0, 10_000) } {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => bail!("等待原后台退出超时，请重试"),
            _ => Err(std::io::Error::last_os_error()).context("等待原后台退出失败"),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub(super) fn is_elevated() -> Result<bool> {
        anyhow::bail!("管理员手势支持仅适用于 Windows")
    }
    pub(super) fn restart_elevated(_: &Path) -> Result<()> {
        anyhow::bail!("管理员手势支持仅适用于 Windows")
    }
    pub(super) fn wait_for_exit(_: u32, _: impl FnOnce() -> Result<()>) -> Result<()> {
        anyhow::bail!("后台进程替换仅适用于 Windows")
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn reads_current_token_without_requesting_elevation() {
        is_elevated().unwrap();
    }

    #[test]
    fn replacement_arguments_preserve_spaces_unicode_and_trailing_slashes() {
        let parameters = imp::parameters(Path::new(r"D:\配置 目录\"), 123).unwrap();
        assert_eq!(
            String::from_utf16(&parameters).unwrap(),
            "--config-dir \"D:\\配置 目录\\\\\" --replace-process 123 run\0"
        );
        assert!(imp::parameters(Path::new("relative"), 123).is_err());
        assert!(imp::parameters(Path::new("D:\\bad\0path"), 123).is_err());
    }

    #[test]
    fn rejects_own_pid_before_requesting_stop() {
        let called = std::cell::Cell::new(false);
        assert!(
            wait_for_exit(std::process::id(), || {
                called.set(true);
                Ok(())
            })
            .is_err()
        );
        assert!(!called.get());
    }
}
