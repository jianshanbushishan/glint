use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

/// Keep the handle open so PID reuse cannot attach us to a different process.
pub struct EngineProcess(HANDLE);

impl EngineProcess {
    pub fn open(pid: u32) -> std::io::Result<Self> {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }

    pub fn has_exited(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
