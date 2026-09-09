#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod appearance;
#[cfg(windows)]
mod engine_process;
mod preview_path;
mod settings;
mod settings_model;
use appearance::{Appearance, Palette, UiSettings};
use glint_core::{ActionKind, ActionSpec, Config, GestureTemplate, MouseButton, PenConfig};
use glint_ipc::{Command, Response, Status};
use gpui::{prelude::*, *};
use gpui_component::{Disableable, Sizable};
use gpui_component::{
    Root,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_component_assets::Assets;
use settings::SettingsView;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

enum Job {
    Connect,
    Command(Command),
}
struct Reply {
    command: Option<Command>,
    passive: bool,
    result: Result<Response, String>,
}

fn launch_engine(dir: &Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("glint.exe");
    if !executable.is_file() {
        return Err(format!(
            "找不到后台程序：{}。请将 glint.exe 与设置程序放在同一目录。",
            executable.display()
        ));
    }
    let mut command = std::process::Command::new(executable);
    command.arg("--config-dir").arg(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法启动后台：{e}"))
}

fn start_worker(dir: PathBuf, smoke_test: bool) -> (mpsc::Sender<Job>, mpsc::Receiver<Reply>) {
    let (tx, jobs) = mpsc::channel();
    let (replies, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let send = |command: Command, passive: bool| {
            let result = glint_ipc::request(&dir, command.clone()).map_err(|e| e.to_string());
            let ok = result.is_ok();
            let _ = replies.send(Reply {
                command: Some(command),
                passive,
                result,
            });
            ok
        };
        loop {
            match jobs.recv_timeout(Duration::from_secs(1)) {
                Ok(Job::Connect) => {
                    if glint_ipc::request(&dir, Command::Status).is_err() {
                        if let Err(error) = launch_engine(&dir) {
                            let _ = replies.send(Reply {
                                command: None,
                                passive: false,
                                result: Err(error),
                            });
                            continue;
                        }
                        for _ in 0..20 {
                            std::thread::sleep(Duration::from_millis(150));
                            if glint_ipc::request(&dir, Command::Status).is_ok() {
                                break;
                            }
                        }
                    }
                    send(Command::GetConfig, false);
                }
                Ok(Job::Command(command)) => {
                    send(command, false);
                    send(Command::GetConfig, true);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // GetConfig includes Status, so one round trip refreshes both.
                    if !smoke_test {
                        send(Command::GetConfig, true);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    (tx, rx)
}

fn preview_point_at(points: &[glint_core::Point], mut distance: f64) -> glint_core::Point {
    for pair in points.windows(2) {
        let length = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
        if length > 0. && distance <= length {
            let ratio = distance / length;
            return glint_core::Point {
                x: pair[0].x + (pair[1].x - pair[0].x) * ratio,
                y: pair[0].y + (pair[1].y - pair[0].y) * ratio,
            };
        }
        distance -= length;
    }
    points.last().copied().unwrap_or_default()
}
fn preview(
    template: GestureTemplate,
    color: u32,
    width: f32,
    physical_width: bool,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            // The native trail uses physical pixels; GPUI paths use logical pixels.
            let width = if physical_width {
                width / window.scale_factor()
            } else {
                width
            };
            let start_radius = width;
            let arrow_length = width * 2.;
            let arrow_half_width = width;
            if template.points.len() < 2
                || template
                    .points
                    .iter()
                    .any(|p| !p.x.is_finite() || !p.y.is_finite())
            {
                return;
            }
            let min_x = template
                .points
                .iter()
                .map(|p| p.x)
                .fold(f64::INFINITY, f64::min);
            let min_y = template
                .points
                .iter()
                .map(|p| p.y)
                .fold(f64::INFINITY, f64::min);
            let max_x = template
                .points
                .iter()
                .map(|p| p.x)
                .fold(f64::NEG_INFINITY, f64::max);
            let max_y = template
                .points
                .iter()
                .map(|p| p.y)
                .fold(f64::NEG_INFINITY, f64::max);
            let padding = (width * 1.5 + 2.).max(16.);
            let available_x = (f32::from(bounds.size.width) - padding * 2.).max(1.);
            let available_y = (f32::from(bounds.size.height) - padding * 2.).max(1.);
            let extent_x = (max_x - min_x) as f32;
            let extent_y = (max_y - min_y) as f32;
            let scale = (available_x / extent_x.max(1.)).min(available_y / extent_y.max(1.));
            let offset_x = (f32::from(bounds.size.width) - extent_x * scale) / 2.;
            let offset_y = (f32::from(bounds.size.height) - extent_y * scale) / 2.;
            let projected: Vec<_> = template
                .points
                .iter()
                .map(|p| glint_core::Point {
                    x: f64::from(offset_x + (p.x - min_x) as f32 * scale),
                    y: f64::from(offset_y + (p.y - min_y) as f32 * scale),
                })
                .collect();
            let segments = preview_path::separate_retraces(&projected);
            let position = |p: glint_core::Point| {
                point(
                    bounds.origin.x + px(p.x as f32),
                    bounds.origin.y + px(p.y as f32),
                )
            };
            let mut path = PathBuilder::stroke(px(width));
            let mut started = false;
            for segment in &segments {
                for &p in segment {
                    if started {
                        path.line_to(position(p));
                    } else {
                        path.move_to(position(p));
                        started = true;
                    }
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(color));
            }
            // Mark the start and each visible leg so retraces retain their order.
            if let Some(start) = segments.first().and_then(|segment| segment.first()) {
                let start = position(*start);
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(start.x - px(start_radius), start.y - px(start_radius)),
                            size(px(start_radius * 2.), px(start_radius * 2.)),
                        ),
                        rgb(color),
                    )
                    .corner_radii(px(start_radius)),
                );
            }
            for (index, segment) in segments.iter().enumerate() {
                let length: f64 = segment
                    .windows(2)
                    .map(|p| (p[1].x - p[0].x).hypot(p[1].y - p[0].y))
                    .sum();
                if length < 8. {
                    continue;
                }
                // Interior arrows describe outgoing legs; the final arrow marks the end.
                let distance = if index + 1 == segments.len() {
                    length
                } else {
                    length * 0.65
                };
                let tip = preview_point_at(segment, distance);
                let anchor =
                    preview_point_at(segment, (distance - f64::from(arrow_length)).max(0.));
                let dx = (tip.x - anchor.x) as f32;
                let dy = (tip.y - anchor.y) as f32;
                let norm = dx.hypot(dy);
                if norm <= f32::EPSILON {
                    continue;
                }
                let (dx, dy) = (dx / norm, dy / norm);
                let tip = position(tip);
                let mut arrow = PathBuilder::stroke(px(width));
                arrow.move_to(point(
                    tip.x - px(dx * arrow_length + dy * arrow_half_width),
                    tip.y - px(dy * arrow_length - dx * arrow_half_width),
                ));
                arrow.line_to(tip);
                arrow.line_to(point(
                    tip.x - px(dx * arrow_length - dy * arrow_half_width),
                    tip.y - px(dy * arrow_length + dx * arrow_half_width),
                ));
                if let Ok(arrow) = arrow.build() {
                    window.paint_path(arrow, rgb(color));
                }
            }
        },
    )
    .w_full()
    .h_full()
}
#[cfg(windows)]
unsafe extern "system" fn settings_window_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    subclass_id: usize,
    _data: usize,
) -> isize {
    use windows_sys::Win32::UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass},
        WindowsAndMessaging::{
            HTCAPTION, HTMAXBUTTON, SC_MAXIMIZE, WM_NCDESTROY, WM_NCLBUTTONDBLCLK,
            WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_SYSCOMMAND,
        },
    };

    // GPUI handles caption button clicks itself without checking WS_MAXIMIZEBOX.
    if (matches!(
        message,
        WM_NCLBUTTONDOWN | WM_NCLBUTTONUP | WM_NCLBUTTONDBLCLK
    ) && wparam == HTMAXBUTTON as usize)
        || (message == WM_NCLBUTTONDBLCLK && wparam == HTCAPTION as usize)
        || (message == WM_SYSCOMMAND && wparam & 0xfff0 == SC_MAXIMIZE as usize)
    {
        return 0;
    }
    // The subclass owns no heap data and is removed with its window.
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(settings_window_proc), subclass_id);
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

#[cfg(windows)]
fn disable_maximize_button(window: &Window) -> anyhow::Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowLongW, SetWindowPos, WS_MAXIMIZEBOX,
    };

    let handle =
        HasWindowHandle::window_handle(window).map_err(|error| anyhow::anyhow!("{error}"))?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        anyhow::bail!("设置窗口没有 Win32 句柄");
    };
    let hwnd = handle.hwnd.get() as _;
    // GPUI couples maximization to resizing; clear only the maximize button style.
    unsafe {
        if windows_sys::Win32::UI::Shell::SetWindowSubclass(hwnd, Some(settings_window_proc), 1, 0)
            == 0
        {
            anyhow::bail!("无法安装设置窗口消息处理器");
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        if style == 0 || SetWindowLongW(hwnd, GWL_STYLE, style & !(WS_MAXIMIZEBOX as i32)) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

fn main() {
    let mut dir = glint_ipc::config_dir();
    let mut args = std::env::args_os().skip(1);
    let mut smoke_test = false;
    while let Some(arg) = args.next() {
        if arg == "--config-dir" {
            if let Some(path) = args.next() {
                dir = path.into();
            }
        } else if arg == "--smoke-test" {
            smoke_test = true;
        }
    }
    if dir.is_relative()
        && let Ok(cwd) = std::env::current_dir()
    {
        dir = cwd.join(dir);
    }
    if let Err(error) = glint_core::logging::init(&dir, "glint-settings.log") {
        eprintln!("无法初始化设置日志：{error}");
    }
    Application::new().with_assets(Assets).run(move |cx| {
        gpui_component::init(cx);
        if smoke_test {
            cx.spawn(async |cx| {
                smol::Timer::after(Duration::from_secs(3)).await;
                let _ = cx.update(|cx| {
                    for handle in cx.windows() {
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    }
                });
            })
            .detach();
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(840.), px(608.)), cx)),
            window_min_size: Some(size(px(680.), px(496.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Glint · 设置".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        match cx.open_window(options, |window, cx| {
            #[cfg(windows)]
            if let Err(error) = disable_maximize_button(window) {
                log::warn!("无法禁用最大化按钮：{error}");
            }
            let view = cx.new(|cx| SettingsView::new(dir, smoke_test, window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        }) {
            Ok(_) => cx.activate(true),
            Err(error) => {
                log::error!("无法打开设置窗口：{error}");
                cx.quit();
            }
        }
    });
}
