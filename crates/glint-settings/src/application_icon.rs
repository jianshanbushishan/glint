use gpui::RenderImage;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

/// Cache successes and failures so repainting never repeatedly reads executables.
pub(crate) fn load(path: &str) -> Option<Arc<RenderImage>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<RenderImage>>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    cache
        .entry(path.to_owned())
        .or_insert_with(|| extract(path))
        .clone()
}

#[cfg(not(windows))]
fn extract(_: &str) -> Option<Arc<RenderImage>> {
    None
}

#[cfg(windows)]
fn extract(path: &str) -> Option<Arc<RenderImage>> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, SelectObject,
        },
        UI::{
            Shell::ExtractIconExW,
            WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx},
        },
    };
    if path.is_empty() || path.contains('\0') || !std::path::Path::new(path).is_file() {
        return None;
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Render on black and white to recover alpha for both modern and legacy masked icons.
    unsafe {
        let mut icon = ptr::null_mut();
        if ExtractIconExW(wide.as_ptr(), 0, &mut icon, ptr::null_mut(), 1) == 0 || icon.is_null() {
            return None;
        }
        let dc = CreateCompatibleDC(ptr::null_mut());
        if dc.is_null() {
            DestroyIcon(icon);
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 32,
                biHeight: -32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(dc);
            DestroyIcon(icon);
            return None;
        }
        let previous = SelectObject(dc, bitmap);
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), 32 * 32 * 4);
        pixels.fill(0);
        let black_ok = DrawIconEx(dc, 0, 0, icon, 32, 32, 0, ptr::null_mut(), DI_NORMAL) != 0;
        GdiFlush();
        let mut black = pixels.to_vec();
        pixels.fill(255);
        let white_ok = DrawIconEx(dc, 0, 0, icon, 32, 32, 0, ptr::null_mut(), DI_NORMAL) != 0;
        GdiFlush();
        for (dark, light) in black
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(pixels.as_chunks::<4>().0)
        {
            let alpha = 255 - light[0].saturating_sub(dark[0]);
            for channel in &mut dark[..3] {
                *channel = if alpha == 0 {
                    0
                } else {
                    (u32::from(*channel) * 255 / u32::from(alpha)).min(255) as u8
                };
            }
            dark[3] = alpha;
        }
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        DestroyIcon(icon);
        if !black_ok || !white_ok {
            return None;
        }
        // RenderImage consumes BGRA bytes, matching the Windows DIB channel order.
        let frame = image::Frame::new(image::RgbaImage::from_raw(32, 32, black)?);
        Some(Arc::new(RenderImage::new(vec![frame])))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn extracts_native_icon_and_reuses_image() {
        let path = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("shell32.dll");
        let path = path.to_str().unwrap();
        let icon = load(path).expect("Windows shell library contains native icons");
        assert!(
            icon.as_bytes(0)
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0)
        );
        assert!(Arc::ptr_eq(&icon, &load(path).unwrap()));
    }

    #[test]
    fn missing_or_invalid_program_uses_fallback() {
        assert!(load("").is_none());
        assert!(load("C:\\glint-nonexistent-icon-test\\missing.exe").is_none());
        assert!(load("invalid\0path").is_none());
    }
}
