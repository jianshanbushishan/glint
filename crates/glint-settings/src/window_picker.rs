//! The picker runs on a worker thread so its native message loop never blocks GPUI.

#[cfg(not(windows))]
pub fn pick_window(_: isize) -> anyhow::Result<Option<glint_core::GestureContext>> {
    anyhow::bail!("窗口选择仅支持 Windows")
}

#[cfg(windows)]
pub use native::pick_window;

#[cfg(windows)]
mod native {
    use std::{
        cell::Cell,
        mem::size_of,
        ptr::null_mut,
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result, bail};
    use glint_core::{GestureContext, Point};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{
                GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
        UI::WindowsAndMessaging::*,
    };

    static PICKING: AtomicBool = AtomicBool::new(false);

    #[derive(Clone, Copy, Default)]
    struct Selection {
        point: Option<(i32, i32)>,
        window: usize,
        released: bool,
        cancelled: bool,
        escape_down: bool,
    }

    thread_local! {
        static SELECTION: Cell<Selection> = Cell::new(Selection::default());
    }

    struct ActivePicker;
    impl Drop for ActivePicker {
        fn drop(&mut self) {
            PICKING.store(false, Ordering::Release);
        }
    }

    struct HiddenSettings {
        window: HWND,
        placement: WINDOWPLACEMENT,
        visible: bool,
    }
    impl HiddenSettings {
        unsafe fn new(window: HWND) -> Result<Self> {
            let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
            placement.length = size_of::<WINDOWPLACEMENT>() as u32;
            if unsafe { GetWindowPlacement(window, &mut placement) } == 0 {
                return Err(std::io::Error::last_os_error()).context("无法读取设置窗口状态");
            }
            let guard = Self {
                window,
                placement,
                visible: unsafe { IsWindowVisible(window) } != 0,
            };
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
            Ok(guard)
        }
    }
    impl Drop for HiddenSettings {
        fn drop(&mut self) {
            unsafe {
                if IsWindow(self.window) != 0 {
                    SetWindowPlacement(self.window, &self.placement);
                    if self.visible {
                        ShowWindow(self.window, self.placement.showCmd as i32);
                        SetForegroundWindow(self.window);
                    } else {
                        ShowWindow(self.window, SW_HIDE);
                    }
                }
            }
        }
    }

    /// SetSystemCursor consumes its handle. Keep private copies of the originals
    /// instead of resetting the user's cursor theme to registry defaults.
    struct Crosshair(Vec<(u32, HCURSOR)>);
    impl Crosshair {
        unsafe fn new() -> Result<Self> {
            let mut guard = Self(Vec::new());
            let cross = unsafe { LoadCursorW(null_mut(), IDC_CROSS) };
            if cross.is_null() {
                return Err(std::io::Error::last_os_error()).context("无法加载选择光标");
            }
            for id in [
                OCR_NORMAL,
                OCR_IBEAM,
                OCR_WAIT,
                OCR_CROSS,
                OCR_UP,
                OCR_SIZENWSE,
                OCR_SIZENESW,
                OCR_SIZEWE,
                OCR_SIZENS,
                OCR_SIZEALL,
                OCR_NO,
                OCR_HAND,
                OCR_APPSTARTING,
                OCR_HELP,
            ] {
                let original =
                    unsafe { CopyIcon(LoadCursorW(null_mut(), id as usize as *const u16)) };
                if original.is_null() {
                    return Err(std::io::Error::last_os_error()).context("无法保存原始光标");
                }
                guard.0.push((id, original));
                let replacement = unsafe { CopyIcon(cross) };
                if replacement.is_null() {
                    return Err(std::io::Error::last_os_error()).context("无法复制选择光标");
                }
                if unsafe { SetSystemCursor(replacement, id) } == 0 {
                    return Err(std::io::Error::last_os_error()).context("无法设置选择光标");
                }
            }
            unsafe {
                SetCursor(cross);
            }
            Ok(guard)
        }
    }
    impl Drop for Crosshair {
        fn drop(&mut self) {
            for (id, cursor) in self.0.drain(..) {
                unsafe {
                    SetSystemCursor(cursor, id);
                }
            }
            unsafe {
                SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
            }
        }
    }

    struct Hooks {
        mouse: HHOOK,
        keyboard: HHOOK,
        timer: usize,
    }
    impl Drop for Hooks {
        fn drop(&mut self) {
            unsafe {
                if self.timer != 0 {
                    KillTimer(null_mut(), self.timer);
                }
                if !self.keyboard.is_null() {
                    UnhookWindowsHookEx(self.keyboard);
                }
                if !self.mouse.is_null() {
                    UnhookWindowsHookEx(self.mouse);
                }
            }
        }
    }

    unsafe extern "system" fn mouse_hook(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
        if code >= 0 && matches!(message as u32, WM_LBUTTONDOWN | WM_LBUTTONUP) {
            // The UI may start us before the release of the invoking click.
            if message as u32 == WM_LBUTTONUP && SELECTION.with(|state| state.get().point.is_none())
            {
                return unsafe { CallNextHookEx(null_mut(), code, message, data) };
            }
            SELECTION.with(|state| {
                let mut selection = state.get();
                if message as u32 == WM_LBUTTONDOWN {
                    let event = unsafe { &*(data as *const MSLLHOOKSTRUCT) };
                    // Low-level mouse hook coordinates are physical screen coordinates.
                    if selection.point.is_none() {
                        selection.point = Some((event.pt.x, event.pt.y));
                        selection.window = unsafe {
                            GetAncestor(WindowFromPhysicalPoint(event.pt), GA_ROOT) as usize
                        };
                    }
                } else if selection.point.is_some() {
                    selection.released = true;
                }
                state.set(selection);
            });
            // Eat both halves, preventing activation or clicks in the target app.
            return 1;
        }
        unsafe { CallNextHookEx(null_mut(), code, message, data) }
    }

    unsafe extern "system" fn keyboard_hook(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
        if code >= 0 {
            let event = unsafe { &*(data as *const KBDLLHOOKSTRUCT) };
            if event.vkCode == 0x1b {
                SELECTION.with(|state| {
                    let mut selection = state.get();
                    if matches!(message as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
                        selection.escape_down = true;
                    } else if matches!(message as u32, WM_KEYUP | WM_SYSKEYUP) {
                        selection.escape_down = false;
                        selection.cancelled = true;
                    }
                    state.set(selection);
                });
                return 1;
            }
        }
        unsafe { CallNextHookEx(null_mut(), code, message, data) }
    }

    pub fn pick_window(settings_hwnd: isize) -> Result<Option<GestureContext>> {
        if PICKING
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            bail!("窗口选择已在进行中");
        }
        let _active = ActivePicker;
        unsafe {
            let _settings = HiddenSettings::new(settings_hwnd as HWND)?;
            SELECTION.with(|state| state.set(Selection::default()));
            let mut hooks = Hooks {
                mouse: null_mut(),
                keyboard: null_mut(),
                timer: 0,
            };
            let module = GetModuleHandleW(std::ptr::null());
            hooks.mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0);
            if hooks.mouse.is_null() {
                return Err(std::io::Error::last_os_error()).context("无法安装窗口选择鼠标钩子");
            }
            hooks.keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0);
            if hooks.keyboard.is_null() {
                return Err(std::io::Error::last_os_error()).context("无法安装窗口选择键盘钩子");
            }
            hooks.timer = SetTimer(null_mut(), 0, 50, None);
            if hooks.timer == 0 {
                return Err(std::io::Error::last_os_error()).context("无法启动窗口选择计时器");
            }
            let crosshair = Crosshair::new()?;
            let started = Instant::now();
            let mut message: MSG = std::mem::zeroed();
            let point = loop {
                let selection = SELECTION.with(Cell::get);
                if (selection.cancelled && (selection.point.is_none() || selection.released))
                    || started.elapsed() >= Duration::from_secs(60)
                {
                    break None;
                }
                if selection.released && !selection.escape_down {
                    break selection.point.map(|point| (point, selection.window));
                }
                match GetMessageW(&mut message, null_mut(), 0, 0) {
                    -1 => {
                        return Err(std::io::Error::last_os_error()).context("等待窗口选择时出错");
                    }
                    0 => break None,
                    _ => {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            };
            drop(hooks);
            drop(crosshair);
            point
                .map(|((x, y), window)| context_at(POINT { x, y }, window as HWND))
                .transpose()
        }
    }

    unsafe fn context_at(point: POINT, window: HWND) -> Result<GestureContext> {
        unsafe {
            if window.is_null() || IsWindow(window) == 0 {
                bail!("所选位置没有可用窗口");
            }
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            if pid == GetCurrentProcessId() {
                bail!("请选择其他应用的窗口");
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return Err(std::io::Error::last_os_error()).context("无法打开所选应用的进程");
            }
            let mut path = vec![0u16; 32768];
            let mut length = path.len() as u32;
            let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
            let error = std::io::Error::last_os_error();
            CloseHandle(process);
            if ok == 0 {
                return Err(error).context("无法读取所选应用的路径");
            }
            let mut title = vec![0u16; 1024];
            let length_title =
                GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
            Ok(GestureContext {
                window: window as usize as u64,
                process: String::from_utf16_lossy(&path[..length as usize]),
                title: String::from_utf16_lossy(&title[..length_title]),
                start: Point {
                    x: point.x as f64,
                    y: point.y as f64,
                },
                modifiers: 0,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn picker_consumes_complete_click_and_escape_pairs() {
            unsafe {
                SELECTION.with(|state| state.set(Selection::default()));
                let mut mouse: MSLLHOOKSTRUCT = std::mem::zeroed();
                mouse.pt = POINT { x: -320, y: 750 };
                let mouse_ptr = &mouse as *const _ as LPARAM;
                mouse_hook(0, WM_LBUTTONUP as WPARAM, mouse_ptr);
                assert!(SELECTION.with(|state| state.get().point.is_none()));
                assert_eq!(mouse_hook(0, WM_LBUTTONDOWN as WPARAM, mouse_ptr), 1);
                assert!(!SELECTION.with(|state| state.get().released));
                assert_eq!(mouse_hook(0, WM_LBUTTONUP as WPARAM, mouse_ptr), 1);
                let selected = SELECTION.with(Cell::get);
                assert_eq!(selected.point, Some((-320, 750)));
                assert!(selected.released);

                SELECTION.with(|state| state.set(Selection::default()));
                let mut key: KBDLLHOOKSTRUCT = std::mem::zeroed();
                key.vkCode = 0x1b;
                let key_ptr = &key as *const _ as LPARAM;
                assert_eq!(keyboard_hook(0, WM_KEYDOWN as WPARAM, key_ptr), 1);
                assert!(!SELECTION.with(|state| state.get().cancelled));
                assert!(SELECTION.with(|state| state.get().escape_down));
                assert_eq!(keyboard_hook(0, WM_KEYUP as WPARAM, key_ptr), 1);
                assert!(SELECTION.with(|state| state.get().cancelled));
                assert!(!SELECTION.with(|state| state.get().escape_down));
            }
        }

        #[test]
        fn settings_visibility_is_restored_after_error() {
            unsafe {
                let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
                // A tiny tool window is sufficient; no global hooks or cursor changes.
                let window = CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    class.as_ptr(),
                    class.as_ptr(),
                    WS_POPUP,
                    -10000,
                    -10000,
                    1,
                    1,
                    null_mut(),
                    null_mut(),
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null(),
                );
                assert!(!window.is_null());
                struct TestWindow(HWND);
                impl Drop for TestWindow {
                    fn drop(&mut self) {
                        unsafe {
                            DestroyWindow(self.0);
                        }
                    }
                }
                let _window = TestWindow(window);
                let foreground = GetForegroundWindow();
                ShowWindow(window, SW_SHOWNOACTIVATE);
                let fail_after_hiding = || -> Result<()> {
                    let _hidden = HiddenSettings::new(window)?;
                    assert_eq!(IsWindowVisible(window), 0);
                    bail!("Simulated hook setup failure")
                };
                assert!(fail_after_hiding().is_err());
                assert_ne!(IsWindowVisible(window), 0);
                ShowWindow(window, SW_HIDE);
                {
                    let _hidden = HiddenSettings::new(window).unwrap();
                }
                assert_eq!(IsWindowVisible(window), 0);
                if !foreground.is_null() {
                    SetForegroundWindow(foreground);
                }
            }
        }
    }
}
