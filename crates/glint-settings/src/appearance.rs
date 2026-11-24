pub use glint_core::{Appearance, UiSettings};
use gpui::{App, Window, WindowAppearance, px, rgb};
use gpui_component::{Theme, ThemeMode};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: u32,
    pub sidebar: u32,
    pub panel: u32,
    pub elevated: u32,
    pub border: u32,
    pub border_subtle: u32,
    pub accent: u32,
    pub accent_border: u32,
    pub text: u32,
    pub muted: u32,
    pub selected: u32,
    pub hover: u32,
    pub error: u32,
    pub error_bg: u32,
    pub error_border: u32,
    pub warning: u32,
    pub is_dark: bool,
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
                bg: 0x111318,
                sidebar: 0x151820,
                panel: 0x1A1E28,
                elevated: 0x222836,
                border: 0x2D3548,
                border_subtle: 0x232A39,
                accent: 0x748FFC,
                accent_border: 0x3B4D8C,
                text: 0xECEFF6,
                muted: 0x8E98AE,
                selected: 0x202945,
                hover: 0x222836,
                error: 0xFF8787,
                error_bg: 0x2D1B1E,
                error_border: 0x612F35,
                warning: 0xF5B841,
                is_dark: true,
            }
        } else {
            Palette {
                bg: 0xF5F6F9,
                sidebar: 0xFAFBFD,
                panel: 0xFFFFFF,
                elevated: 0xEEF1F6,
                border: 0xDDE2EC,
                border_subtle: 0xEAEEF5,
                accent: 0x3B5BDB,
                accent_border: 0xB1C3F8,
                text: 0x171C28,
                muted: 0x636E83,
                selected: 0xEBF0FE,
                hover: 0xF1F4F9,
                error: 0xD9383A,
                error_bg: 0xFEF2F2,
                error_border: 0xF8CACA,
                warning: 0xC97A10,
                is_dark: false,
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
        theme.primary_hover = rgb(if dark { 0x91A7FF } else { 0x324EC2 }).into();
        theme.primary_active = rgb(if dark { 0x5C7CFA } else { 0x2940A3 }).into();
        theme.primary_foreground = rgb(if dark { 0x0F1426 } else { 0xFFFFFF }).into();
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
        theme.switch = rgb(if dark { 0x465066 } else { 0xC2CAD8 }).into();
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
