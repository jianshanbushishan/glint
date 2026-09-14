use super::wide;
use glint_core::{Appearance, UiSettings};
use std::{
    cell::RefCell,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::Registry::*,
    UI::{Accessibility::*, Controls::*, HiDpi::*, WindowsAndMessaging::*},
};

thread_local! {
    static CONFIG_DIR: RefCell<PathBuf> = const { RefCell::new(PathBuf::new()) };
}

pub(super) fn set_config_dir(dir: PathBuf) {
    CONFIG_DIR.with(|value| *value.borrow_mut() = dir);
}

fn dark_mode(appearance: Appearance, system_light: bool) -> bool {
    match appearance {
        Appearance::Dark => true,
        Appearance::Light => false,
        Appearance::System => !system_light,
    }
}

fn system_light() -> bool {
    let mut value: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut value as *mut _ as _,
            &mut size,
        );
    }
    value != 0
}

fn color(rgb: u32) -> COLORREF {
    ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff)
}

struct Item {
    text: Vec<u16>,
    font: HFONT,
    background: HBRUSH,
    selected: HBRUSH,
    border: HBRUSH,
    foreground: COLORREF,
    scale: f32,
}

impl Item {
    fn px(&self, value: f32) -> i32 {
        (value * self.scale).round() as i32
    }
}

// These items live for the entire native modal menu loop. Windows still handles
// keyboard navigation, dismissal and command selection; we only supply painting.
pub(super) unsafe fn show(window: HWND, point: POINT, pause_label: &str) -> i32 {
    unsafe {
        let appearance = CONFIG_DIR.with(|dir| {
            UiSettings::load(&dir.borrow())
                .unwrap_or_default()
                .appearance
        });
        let dark = dark_mode(appearance, system_light());
        let mut contrast: HIGHCONTRASTW = zeroed();
        contrast.cbSize = size_of::<HIGHCONTRASTW>() as u32;
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            &mut contrast as *mut _ as _,
            0,
        );
        let custom = contrast.dwFlags & HCF_HIGHCONTRASTON == 0;
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let (mut dpi_x, mut dpi_y) = (96, 96);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let mut metrics: NONCLIENTMETRICSW = zeroed();
        metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            &mut metrics as *mut _ as _,
            0,
            dpi_y,
        );
        let font = CreateFontIndirectW(&metrics.lfMenuFont);
        let background = CreateSolidBrush(color(if dark { 0x222222 } else { 0xffffff }));
        let selected = CreateSolidBrush(color(if dark { 0x253e32 } else { 0xe8f4ee }));
        let border = CreateSolidBrush(color(if dark { 0x3a3a3a } else { 0xdcdcdc }));
        let menu = CreatePopupMenu();
        let mut entries = vec![("打开设置", 1), (pause_label, 2), ("重新加载配置", 3)];
        if !crate::elevation::is_elevated().unwrap_or(false) {
            entries.push(("以管理员权限重启", 5));
        }
        entries.extend([("", 0), ("退出 Glint", 4)]);
        let items: Vec<Item> = entries
            .iter()
            .map(|(text, _)| Item {
                text: wide(text),
                font,
                background,
                selected,
                border,
                foreground: color(if dark { 0xebebeb } else { 0x242424 }),
                scale: dpi_y as f32 / 96.0,
            })
            .collect();
        if custom {
            let info = MENUINFO {
                cbSize: size_of::<MENUINFO>() as u32,
                fMask: MIM_BACKGROUND,
                hbrBack: background,
                ..zeroed()
            };
            SetMenuInfo(menu, &info);
        }
        for (item, &(_, id)) in items.iter().zip(&entries) {
            // Keep the string on owner-drawn entries for accessibility clients.
            let info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE | MIIM_ID | MIIM_STRING | MIIM_DATA,
                fType: (if custom { MFT_OWNERDRAW } else { MFT_STRING })
                    | if id == 0 { MFT_SEPARATOR } else { 0 },
                wID: id,
                dwItemData: item as *const Item as usize,
                dwTypeData: item.text.as_ptr() as _,
                ..zeroed()
            };
            InsertMenuItemW(menu, u32::MAX, 1, &info);
        }
        let choice = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            window,
            null(),
        );
        DestroyMenu(menu);
        DeleteObject(font);
        DeleteObject(background);
        DeleteObject(selected);
        DeleteObject(border);
        choice
    }
}

pub(super) unsafe fn handle_message(message: u32, lparam: LPARAM) -> Option<LRESULT> {
    unsafe {
        match message {
            WM_MEASUREITEM => {
                let measure = &mut *(lparam as *mut MEASUREITEMSTRUCT);
                if measure.CtlType != ODT_MENU || measure.itemData == 0 {
                    return None;
                }
                let item = &*(measure.itemData as *const Item);
                let dc = GetDC(null_mut());
                let old = SelectObject(dc, item.font);
                let mut size: SIZE = zeroed();
                GetTextExtentPoint32W(
                    dc,
                    item.text.as_ptr(),
                    (item.text.len() - 1) as i32,
                    &mut size,
                );
                SelectObject(dc, old);
                ReleaseDC(null_mut(), dc);
                measure.itemWidth = (size.cx + item.px(32.0)) as u32;
                measure.itemHeight = if item.text.len() == 1 {
                    item.px(9.0)
                } else {
                    size.cy + item.px(14.0)
                } as u32;
                Some(1)
            }
            WM_DRAWITEM => {
                let draw = &*(lparam as *const DRAWITEMSTRUCT);
                if draw.CtlType != ODT_MENU || draw.itemData == 0 {
                    return None;
                }
                let item = &*(draw.itemData as *const Item);
                FillRect(
                    draw.hDC,
                    &draw.rcItem,
                    if draw.itemState & ODS_SELECTED != 0 {
                        item.selected
                    } else {
                        item.background
                    },
                );
                let mut rect = draw.rcItem;
                rect.left += item.px(16.0);
                rect.right -= item.px(16.0);
                if item.text.len() == 1 {
                    rect.top = (rect.top + rect.bottom) / 2;
                    rect.bottom = rect.top + 1;
                    FillRect(draw.hDC, &rect, item.border);
                } else {
                    let saved = SaveDC(draw.hDC);
                    SelectObject(draw.hDC, item.font);
                    SetBkMode(draw.hDC, TRANSPARENT as i32);
                    SetTextColor(draw.hDC, item.foreground);
                    DrawTextW(
                        draw.hDC,
                        item.text.as_ptr(),
                        (item.text.len() - 1) as i32,
                        &mut rect,
                        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                    );
                    RestoreDC(draw.hDC, saved);
                }
                Some(1)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_theme_overrides_system_and_system_tracks_changes() {
        for system_light in [true, false] {
            assert!(dark_mode(Appearance::Dark, system_light));
            assert!(!dark_mode(Appearance::Light, system_light));
            assert_eq!(dark_mode(Appearance::System, system_light), !system_light);
        }
    }

    #[test]
    fn native_measure_and_draw_use_item_font_and_theme_colors() {
        unsafe {
            let screen = GetDC(null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 300, 60);
            let old_bitmap = SelectObject(dc, bitmap);
            let background = CreateSolidBrush(color(0x222222));
            let selected = CreateSolidBrush(color(0x253e32));
            let item = Item {
                text: wide("重新加载配置"),
                font: GetStockObject(DEFAULT_GUI_FONT) as _,
                background,
                selected,
                border: background,
                foreground: color(0xebebeb),
                scale: 1.0,
            };
            let data = &item as *const Item as usize;
            let mut measure = MEASUREITEMSTRUCT {
                CtlType: ODT_MENU,
                itemData: data,
                ..zeroed()
            };
            assert_eq!(
                handle_message(WM_MEASUREITEM, &mut measure as *mut _ as LPARAM),
                Some(1)
            );
            assert!(measure.itemWidth > 32);
            assert!(measure.itemHeight > 14);
            let mut draw = DRAWITEMSTRUCT {
                CtlType: ODT_MENU,
                itemData: data,
                hDC: dc,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 300,
                    bottom: 60,
                },
                ..zeroed()
            };
            for (state, expected) in [(0, 0x222222), (ODS_SELECTED, 0x253e32)] {
                draw.itemState = state;
                assert_eq!(
                    handle_message(WM_DRAWITEM, &draw as *const _ as LPARAM),
                    Some(1)
                );
                assert_eq!(GetPixel(dc, 1, 1), color(expected));
            }
            SelectObject(dc, old_bitmap);
            DeleteObject(bitmap);
            DeleteObject(background);
            DeleteObject(selected);
            DeleteDC(dc);
            ReleaseDC(null_mut(), screen);
        }
    }
}
