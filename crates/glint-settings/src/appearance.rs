pub use glint_core::{Appearance, UiSettings};
use gpui::{App, Window, WindowAppearance, px, rgb};
use gpui_component::{Theme, ThemeMode};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: u32,
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
                bg: 0x181B20,
                panel: 0x22262C,
                border: 0x353B43,
                accent: 0x58D4AC,
                text: 0xE5E9ED,
                muted: 0xA2ADB8,
                selected: 0x223E36,
                hover: 0x2D343C,
                error: 0xFFB4A9,
            }
        } else {
            Palette {
                bg: 0xF6F7F9,
                panel: 0xFFFFFF,
                border: 0xE4E8EC,
                accent: 0x00835E,
                text: 0x202A31,
                muted: 0x697780,
                selected: 0xE0EEE8,
                hover: 0xE5E9ED,
                error: 0xB42318,
            }
        };
        let theme = Theme::global_mut(cx);
        theme.font_family = "Microsoft YaHei UI".into();
        theme.font_size = px(14.);
        theme.mono_font_size = px(13.);
        theme.radius = px(7.);
        theme.shadow = false;
        theme.background = rgb(palette.panel).into();
        theme.foreground = rgb(palette.text).into();
        theme.border = rgb(palette.border).into();
        theme.input = rgb(palette.border).into();
        theme.primary = rgb(palette.accent).into();
        theme.primary_hover = rgb(if dark { 0xA1F0CF } else { 0x066A47 }).into();
        theme.primary_active = rgb(if dark { 0x64CBA3 } else { 0x055C3D }).into();
        theme.primary_foreground = rgb(if dark { 0x11231A } else { 0xFFFFFF }).into();
        theme.secondary = rgb(if dark { 0x303030 } else { 0xF0F0F0 }).into();
        theme.secondary_foreground = rgb(palette.text).into();
        theme.secondary_hover = rgb(palette.hover).into();
        theme.secondary_active = rgb(palette.selected).into();
        theme.muted = rgb(palette.hover).into();
        theme.muted_foreground = rgb(palette.muted).into();
        theme.accent = rgb(palette.selected).into();
        theme.accent_foreground = rgb(palette.text).into();
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
