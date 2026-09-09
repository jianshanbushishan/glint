use super::{native_error, wide};
use crate::PlatformEvent;
use anyhow::{Context, Result, bail};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use glint_core::{PenConfig, Point};
use std::{
    cell::RefCell,
    mem::zeroed,
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

const WAKE: u32 = WM_APP + 51;
enum DrawCommand {
    Begin(u64, Point, PenConfig),
    Invalidate(u64),
    Clear(u64),
    Quit,
}
pub(super) struct Overlay {
    sender: Sender<DrawCommand>,
    points: Sender<(u64, Point)>,
    epoch: AtomicU64,
    invalidated: AtomicBool,
    notified: Arc<AtomicBool>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}
impl Overlay {
    pub fn start(events: Sender<PlatformEvent>) -> Result<Self> {
        let (sender, receiver) = unbounded();
        let (points, point_receiver) = bounded(4096);
        let notified = Arc::new(AtomicBool::new(false));
        let thread_notified = notified.clone();
        let (ready_tx, ready_rx) = bounded(1);
        let thread = thread::Builder::new()
            .name("glint-trail".into())
            .spawn(move || {
                if let Err(error) = run(receiver, point_receiver, thread_notified, &ready_tx) {
                    let message = format!("{error:#}");
                    let _ = ready_tx.send(Err(message.clone()));
                    let _ = events.try_send(PlatformEvent::Error(message));
                }
            })?;
        match ready_rx
            .recv()
            .context("Trail renderer stopped during startup")?
        {
            Ok(thread_id) => Ok(Self {
                sender,
                points,
                epoch: AtomicU64::new(0),
                invalidated: AtomicBool::new(false),
                notified,
                thread_id,
                thread: Some(thread),
            }),
            Err(message) => {
                let _ = thread.join();
                bail!("{message}")
            }
        }
    }
    fn wake(&self) {
        if !self.notified.swap(true, Ordering::AcqRel)
            && unsafe { PostThreadMessageW(self.thread_id, WAKE, 0, 0) } == 0
        {
            self.notified.store(false, Ordering::Release);
        }
    }
    fn send(&self, command: DrawCommand) {
        if self.sender.send(command).is_ok() {
            self.wake();
        }
    }
    pub fn begin(&self, point: Point, pen: PenConfig) {
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.invalidated.store(false, Ordering::Release);
        self.send(DrawCommand::Begin(epoch, point, pen));
    }
    pub fn invalidate(&self) {
        if !self.invalidated.swap(true, Ordering::AcqRel) {
            self.send(DrawCommand::Invalidate(self.epoch.load(Ordering::Acquire)));
        }
    }
    pub fn point(&self, point: Point) {
        // Rendering may drop samples under load; recognition retains its own path.
        if self
            .points
            .try_send((self.epoch.load(Ordering::Acquire), point))
            .is_ok()
        {
            self.wake();
        }
    }
    pub fn clear(&self) {
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.send(DrawCommand::Clear(epoch));
    }
}
impl Drop for Overlay {
    fn drop(&mut self) {
        self.send(DrawCommand::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Pen {
    rgb: u32,
    width: f32,
}
struct Surface {
    epoch: u64,
    points: Vec<Point>,
    pen: Option<Pen>,
    invalid_rgb: u32,
    opacity: u8,
    visible: bool,
}
impl Surface {
    fn invalidate(&mut self, epoch: u64) -> bool {
        if self.epoch != epoch {
            return false;
        }
        if let Some(pen) = self.pen.as_mut() {
            pen.rgb = self.invalid_rgb;
            return true;
        }
        false
    }
}
thread_local! { static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) }; }

pub(super) fn validate_pen(pen: &PenConfig) -> Result<()> {
    color(&pen.color)?;
    color(&pen.invalid_color)?;
    if !pen.width.is_finite() || !(0.0..=64.0).contains(&pen.width) {
        bail!("Pen width must be between 0 and 64")
    }
    if !pen.opacity.is_finite() || !(0.0..=1.0).contains(&pen.opacity) {
        bail!("Pen opacity must be between 0 and 1")
    }
    Ok(())
}
fn color(value: &str) -> Result<u32> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Pen color must be #RRGGBB")
    }
    let rgb = u32::from_str_radix(hex, 16)?;
    Ok(rgb)
}

fn run(
    receiver: Receiver<DrawCommand>,
    points: Receiver<(u64, Point)>,
    notified: Arc<AtomicBool>,
    ready: &Sender<Result<u32, String>>,
) -> Result<()> {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut message: MSG = unsafe { zeroed() };
    unsafe {
        PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
    }
    let window = unsafe {
        let class = wide("Glint.Trail");
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(native_error("Register trail window"));
        }
        CreateWindowExW(
            WS_EX_LAYERED
                | WS_EX_TOPMOST
                | WS_EX_TRANSPARENT
                | WS_EX_NOACTIVATE
                | if cfg!(test) { 0 } else { WS_EX_TOOLWINDOW },
            class.as_ptr(),
            wide("Glint Trail").as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };
    if window.is_null() {
        return Err(native_error("Create trail window"));
    }
    SURFACE.with(|s| {
        *s.borrow_mut() = Some(Surface {
            epoch: 0,
            points: Vec::new(),
            pen: None,
            invalid_rgb: 0x9ca3af,
            opacity: 255,
            visible: false,
        })
    });
    let timer = unsafe { SetTimer(window, 1, 16, None) };
    if timer == 0 {
        unsafe {
            DestroyWindow(window);
        }
        return Err(native_error("Start trail frame timer"));
    }
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    let mut exit = false;
    let mut error = None;
    let mut needs_paint = false;
    let mut last_paint = Instant::now();
    let mut presented_epoch = None;
    let mut frames = 0_u64;
    while !exit {
        let got = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if got <= 0 {
            if got < 0 {
                error = Some(native_error("Read trail messages"));
            }
            break;
        }
        if message.message == WAKE
            || (message.message == WM_TIMER && message.hwnd == window && message.wParam == timer)
        {
            if message.message == WAKE {
                notified.store(false, Ordering::Release);
            }
            let mut show = None;
            let mut dirty = false;
            for command in receiver.try_iter() {
                match command {
                    DrawCommand::Quit => {
                        exit = true;
                        break;
                    }
                    DrawCommand::Invalidate(epoch) => {
                        SURFACE.with(|slot| {
                            if let Some(surface) = slot.borrow_mut().as_mut() {
                                dirty |= surface.invalidate(epoch);
                            }
                        });
                    }
                    DrawCommand::Clear(epoch) => {
                        presented_epoch = None;
                        unsafe {
                            ShowWindow(window, SW_HIDE);
                        }
                        if log::log_enabled!(log::Level::Trace) {
                            SURFACE.with(|s| { if let Some(s) = s.borrow().as_ref() {
                                log::trace!("trail clear: samples={} frames={frames} visible={} opacity={}", s.points.len(), s.visible, s.opacity);
                            }});
                        }
                        frames = 0;
                        SURFACE.with(|s| {
                            if let Some(s) = s.borrow_mut().as_mut() {
                                s.epoch = epoch;
                                s.points.clear();
                                s.visible = false;
                            }
                        });
                        show = Some(false);
                        dirty = true;
                    }
                    DrawCommand::Begin(epoch, point, config) => {
                        // A layered window retains its last bitmap while hidden.
                        // Keep it hidden until this stroke submits its first frame.
                        presented_epoch = None;
                        unsafe {
                            ShowWindow(window, SW_HIDE);
                        }
                        let new_pen = Pen {
                            rgb: color(&config.color).unwrap_or(0x69e0c3),
                            width: config.width,
                        };
                        SURFACE.with(|s| {
                            if let Some(s) = s.borrow_mut().as_mut() {
                                s.epoch = epoch;
                                s.pen = Some(new_pen);
                                s.invalid_rgb = color(&config.invalid_color).unwrap_or(0x9ca3af);
                                s.points = vec![point];
                                s.opacity = (config.opacity * 255.0).round() as u8;
                                s.visible = config.opacity > 0.0 && config.width > 0.0;
                                show = Some(s.visible);
                            }
                        });
                        dirty = true;
                    }
                }
            }
            // Epochs prevent old queued points from appearing after clear/new capture.
            SURFACE.with(|s| {
                if let Some(s) = s.borrow_mut().as_mut() {
                    // A continuously producing mouse must not monopolize this loop.
                    for (epoch, point) in points.try_iter().take(4096) {
                        if epoch == s.epoch && s.visible && s.points.last() != Some(&point) {
                            s.points.push(point);
                            dirty = true;
                            if s.points.len() > 16384 {
                                s.points = s.points.iter().step_by(2).copied().collect();
                                if s.points.last() != Some(&point) {
                                    s.points.push(point);
                                }
                            }
                        }
                    }
                }
            });
            if show == Some(false) {
                unsafe {
                    ShowWindow(window, SW_HIDE);
                }
            }
            needs_paint |= dirty;
            if needs_paint && (show.is_some() || last_paint.elapsed() >= Duration::from_millis(16))
            {
                let painted = SURFACE.with(|slot| {
                    let slot = slot.borrow();
                    if log::log_enabled!(log::Level::Trace)
                        && frames == 0
                        && let Some(s) = slot.as_ref()
                    {
                        log::trace!(
                            "trail first frame: samples={} first={:?} last={:?}",
                            s.points.len(),
                            s.points.first(),
                            s.points.last()
                        );
                    }
                    slot.as_ref()
                        .map_or(Ok(false), |surface| present(window, surface))
                });
                match painted {
                    Ok(true) => {
                        presented_epoch =
                            SURFACE.with(|slot| slot.borrow().as_ref().map(|s| s.epoch));
                    }
                    Ok(false) => {}
                    Err(failure) => {
                        error = Some(failure);
                        break;
                    }
                }
                needs_paint = false;
                frames += 1;
                last_paint = Instant::now();
            }
            if SURFACE.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .is_some_and(|s| s.visible && presented_epoch == Some(s.epoch))
            }) && let Err(failure) = raise_trail(window)
            {
                error = Some(failure);
                break;
            }
        } else {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    unsafe {
        KillTimer(window, timer);
        ShowWindow(window, SW_HIDE);
        DestroyWindow(window);
    }
    SURFACE.with(|s| s.borrow_mut().take());
    if let Some(error) = error {
        Err(error)
    } else {
        Ok(())
    }
}
// Render an explicit BGRA surface. No color-keyed WM_PAINT redirection is needed.
struct Frame {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// Rasterize coverage into premultiplied BGRA for UpdateLayeredWindow.
fn rasterize(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    points: &[Point],
    origin: Point,
    pen: &Pen,
) -> Result<()> {
    use tiny_skia::{LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Stroke, Transform};

    pixels.fill(0);
    let mut path = PathBuilder::new();
    for (index, point) in points.iter().enumerate() {
        let x = (point.x - origin.x) as f32;
        let y = (point.y - origin.y) as f32;
        if index == 0 {
            path.move_to(x, y);
        } else {
            path.line_to(x, y);
        }
    }
    let path = path.finish().context("Build trail path")?;
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        (pen.rgb >> 16) as u8,
        (pen.rgb >> 8) as u8,
        pen.rgb as u8,
        255,
    );
    paint.anti_alias = true;
    let stroke = Stroke {
        width: pen.width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let mut pixmap =
        PixmapMut::from_bytes(pixels, width, height).context("Create trail raster surface")?;
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    // tiny-skia produces premultiplied RGBA; the Windows DIB expects BGRA.
    // Preserve coverage alpha, including for pure black and overlapping segments.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(())
}

fn present(window: HWND, surface: &Surface) -> Result<bool> {
    if !surface.visible || surface.points.len() < 2 {
        return Ok(false);
    }
    let Some(pen) = &surface.pen else {
        return Ok(false);
    };
    unsafe {
        let screen_left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let screen_top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let screen_right = screen_left + GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let screen_bottom = screen_top + GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let left = (surface.points.iter().map(|p| p.x as i32).min().unwrap() - 34).max(screen_left);
        let top = (surface.points.iter().map(|p| p.y as i32).min().unwrap() - 34).max(screen_top);
        let right =
            (surface.points.iter().map(|p| p.x as i32).max().unwrap() + 35).min(screen_right);
        let bottom =
            (surface.points.iter().map(|p| p.y as i32).max().unwrap() + 35).min(screen_bottom);
        let (width, height) = (right - left, bottom - top);
        if log::log_enabled!(log::Level::Trace) {
            log::trace!(
                "present screen={screen_left},{screen_top}..{screen_right},{screen_bottom} bitmap={left},{top} {width}x{height}"
            );
        }
        if width <= 0 || height <= 0 {
            return Ok(false);
        }
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return Err(native_error("Create trail bitmap DC"));
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            let error = native_error("Create trail bitmap");
            DeleteDC(dc);
            return Err(error);
        }
        let _frame = Frame {
            dc,
            bitmap,
            previous: SelectObject(dc, bitmap),
        };
        let pixels =
            std::slice::from_raw_parts_mut(bits as *mut u8, width as usize * height as usize * 4);
        rasterize(
            pixels,
            width as u32,
            height as u32,
            &surface.points,
            Point {
                x: left as f64,
                y: top as f64,
            },
            pen,
        )?;
        if log::log_enabled!(log::Level::Trace) {
            let ink = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[3] != 0)
                .count();
            log::trace!("present ink={ink}");
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: surface.opacity,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        if UpdateLayeredWindow(
            window,
            null_mut(),
            &POINT { x: left, y: top },
            &SIZE {
                cx: width,
                cy: height,
            },
            dc,
            &POINT { x: 0, y: 0 },
            0,
            &blend,
            ULW_ALPHA,
        ) == 0
        {
            return Err(native_error("Present transparent trail bitmap"));
        }
        raise_trail(window)?;
        if log::log_enabled!(log::Level::Trace) {
            let mut rect: RECT = zeroed();
            GetWindowRect(window, &mut rect);
            log::trace!(
                "window visible={} rect={},{},{},{}",
                IsWindowVisible(window),
                rect.left,
                rect.top,
                rect.right,
                rect.bottom
            );
        }
    }
    Ok(true)
}
fn raise_trail(window: HWND) -> Result<()> {
    unsafe {
        // Show first, then establish z-order. Showing and ordering together can
        // leave a non-activating window behind the foreground window.
        if IsWindowVisible(window) == 0 {
            ShowWindow(window, SW_SHOWNOACTIVATE);
        }
        if SetWindowPos(
            window,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        ) == 0
        {
            return Err(native_error("Keep trail above foreground"));
        }
    }
    Ok(())
}
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => return HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            unsafe {
                ValidateRect(window, null());
            }
            return 0;
        }
        _ => {}
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "creates a temporary native trail window on the desktop"]
    fn native_trail_submission() {
        let (events, errors) = unbounded();
        let overlay = Overlay::start(events).unwrap();
        overlay.begin(Point { x: 100., y: 100. }, PenConfig::default());
        overlay.point(Point { x: 200., y: 200. });
        overlay.invalidate();
        std::thread::sleep(Duration::from_millis(
            std::env::var("GLINT_TRAIL_TEST_MS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(150)
                .min(30000),
        ));
        overlay.clear();
        overlay.begin(Point { x: 100., y: 100. }, PenConfig::default());
        overlay.point(Point { x: 200., y: 100. });
        std::thread::sleep(Duration::from_millis(50));
        overlay.clear();
        drop(overlay);
        assert!(errors.try_iter().collect::<Vec<_>>().is_empty());
    }
    #[test]
    fn color_preserves_rgb_including_black() {
        assert_eq!(color("#112233").unwrap(), 0x112233);
        assert_eq!(color("#000000").unwrap(), 0);
        assert!(color("#fff").is_err());
        assert!(color("#gg0000").is_err());
    }

    #[test]
    fn invalidation_recolors_the_whole_stroke_and_ignores_old_epochs() {
        let mut surface = Surface {
            epoch: 2,
            points: vec![Point::default(), Point { x: 20., y: 10. }],
            pen: Some(Pen {
                rgb: 0x69e0c3,
                width: 4.,
            }),
            invalid_rgb: 0x123456,
            opacity: 200,
            visible: true,
        };
        assert!(!surface.invalidate(1));
        assert_eq!(surface.pen.as_ref().unwrap().rgb, 0x69e0c3);
        assert!(surface.invalidate(2));
        surface.points.push(Point { x: 30., y: 15. });
        assert_eq!(surface.pen.as_ref().unwrap().rgb, 0x123456);
        assert_eq!(surface.opacity, 200);
        surface.epoch = 3;
        surface.pen.as_mut().unwrap().rgb = 0x69e0c3;
        assert!(!surface.invalidate(2));
        assert_eq!(surface.pen.as_ref().unwrap().rgb, 0x69e0c3);
    }

    #[test]
    fn diagonal_has_coverage_alpha_and_premultiplied_bgra() {
        for rgb in [0x000000, 0x4080ff] {
            let mut pixels = vec![0; 32 * 32 * 4];
            rasterize(
                &mut pixels,
                32,
                32,
                &[Point { x: -15.25, y: 5.5 }, Point { x: 6.5, y: 24.25 }],
                Point { x: -20., y: 0. },
                &Pen { rgb, width: 3.5 },
            )
            .unwrap();
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[3] > 0 && p[3] < 255)
            );
            assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 255));
            assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
            for p in pixels.as_chunks::<4>().0.iter() {
                for (channel, value) in
                    p[..3]
                        .iter()
                        .zip([rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8])
                {
                    let expected = (u16::from(value) * u16::from(p[3]) + 127) / 255;
                    assert!((i32::from(*channel) - i32::from(expected)).abs() <= 1);
                }
            }
        }
    }
}
