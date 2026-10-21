pub use glint_core::{Appearance, UiSettings};
use gpui::{App, Window, WindowAppearance, px, rgb};
use gpui_component::{Theme, ThemeMode};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: u32,
    pub sidebar: u32,
    pub panel: u32,
    pub border: u32,
    pub accent: u32,
    pub text: u32,
    pub muted: u32,
    pub selected: u32,
    pub hover: u32,
    pub error: u32,
}

pub trait AppearanceExt {
    fn apply(self, window: &mut Window, cx: &mut App) -> Palette;
}

impl AppearanceExt for Appearance {
    fn apply(self, window: &mut Window, cx: &mut App) -> Palette {
        let mode = match self {
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
            Self::System => match window.appearance() {
                WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
                _ => ThemeMode::Light,
            },
        };
        Theme::change(mode, Some(window), cx);
        let dark = mode.is_dark();
        // Apply after GPUI's system-theme callback so an explicit app choice also
        // controls the native title bar when the Windows theme changes.
        window.defer(cx, move |window, _| update_titlebar(window, dark));
        let palette = if dark {
            Palette {
                bg: 0x141B19,
                sidebar: 0x18211E,
                panel: 0x1D2823,
                border: 0x34443C,
                accent: 0x70DDB1,
                text: 0xE8F0EC,
                muted: 0xA5B8AE,
                selected: 0x253F33,
                hover: 0x29392F,
                error: 0xFFB4A9,
            }
        } else {
            Palette {
                bg: 0xF3F6F4,
                sidebar: 0xF9FBFA,
                panel: 0xFFFFFF,
                border: 0xDFE7E2,
                accent: 0x087F5B,
                text: 0x23382E,
                muted: 0x63786C,
                selected: 0xE2F2E9,
                hover: 0xEAF0EC,
                error: 0xB42318,
            }
        };
        let theme = Theme::global_mut(cx);
        // Segoe UI is available on Windows 10 and 11; the view supplies CJK fallbacks.
        theme.font_family = "Segoe UI".into();
        theme.font_size = px(14.);
        theme.mono_font_family = "Consolas".into();
        theme.mono_font_size = px(13.);
        theme.radius = px(8.);
        theme.radius_lg = px(12.);
        theme.shadow = false;
        theme.background = rgb(palette.panel).into();
        theme.foreground = rgb(palette.text).into();
        theme.border = rgb(palette.border).into();
        theme.input = rgb(palette.border).into();
        theme.primary = rgb(palette.accent).into();
        theme.primary_hover = rgb(if dark { 0x94E8C3 } else { 0x066A4B }).into();
        theme.primary_active = rgb(if dark { 0x52C99A } else { 0x05563D }).into();
        theme.primary_foreground = rgb(if dark { 0x11231A } else { 0xFFFFFF }).into();
        theme.secondary = rgb(palette.bg).into();
        theme.secondary_foreground = rgb(palette.text).into();
        theme.secondary_hover = rgb(palette.hover).into();
        theme.secondary_active = rgb(palette.selected).into();
        theme.muted = rgb(palette.hover).into();
        theme.muted_foreground = rgb(palette.muted).into();
        theme.accent = rgb(palette.selected).into();
        theme.accent_foreground = rgb(palette.text).into();
        theme.list_hover = rgb(palette.hover).into();
        theme.list_active = rgb(palette.selected).into();
        theme.list_active_border = rgb(palette.accent).into();
        theme.popover = rgb(palette.panel).into();
        theme.popover_foreground = rgb(palette.text).into();
        theme.switch = rgb(if dark { 0x4B6053 } else { 0xB6C8BC }).into();
        theme.switch_thumb = rgb(0xFFFFFF).into();
        theme.slider_bar = rgb(palette.accent).into();
        theme.slider_thumb = rgb(palette.panel).into();
        theme.caret = rgb(palette.accent).into();
        theme.ring = rgb(palette.accent).into();
        theme.selection = rgb(palette.selected).into();
        theme.danger = rgb(palette.error).into();
        palette
    }
}

#[cfg(windows)]
fn update_titlebar(window: &Window, dark: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        let enabled = i32::from(dark);
        // Older DWM versions may not expose this cosmetic attribute.
        unsafe {
            DwmSetWindowAttribute(
                handle.hwnd.get() as _,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &enabled as *const _ as _,
                std::mem::size_of_val(&enabled) as u32,
            );
        }
    }
}

#[cfg(not(windows))]
fn update_titlebar(_window: &Window, _dark: bool) {}
