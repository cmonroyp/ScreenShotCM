//! Reading the screen. GDI `BitBlt` with `CAPTUREBLT` is what freezes the
//! screen: it needs no consent, draws no yellow border, includes layered
//! windows, and honours `WDA_EXCLUDEFROMCAPTURE`, so our own toolbars never
//! end up in a picture. Coordinates are physical pixels of the virtual
//! screen; the process is per-monitor DPI aware.

use std::mem::size_of;

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetMonitorInfoW, HDC,
    HMONITOR, MONITORINFO, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetSystemMetrics, GetWindowLongW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    WS_EX_TRANSPARENT,
};
use windows::core::BOOL;

use crate::image::Image;
use crate::util::Rect;

pub fn virtual_screen() -> Rect {
    unsafe {
        Rect::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Copies `area` of the screen.
pub fn grab(area: Rect) -> Result<Image, String> {
    if area.is_empty() {
        return Err(crate::i18n::tr("área vacía", "empty area").into());
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err(crate::i18n::tr("GetDC falló", "GetDC failed").into());
        }
        let result = grab_from(screen, area);
        ReleaseDC(None, screen);
        result
    }
}

unsafe fn grab_from(screen: HDC, area: Rect) -> Result<Image, String> {
    unsafe {
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: area.w,
                biHeight: -area.h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let dib = match CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = DeleteDC(mem);
                return Err(format!(
                    "no hay memoria para una captura de {}×{}",
                    area.w, area.h
                ));
            }
        };
        let old = SelectObject(mem, dib.into());
        let copied = BitBlt(
            mem,
            0,
            0,
            area.w,
            area.h,
            Some(screen),
            area.x,
            area.y,
            SRCCOPY | CAPTUREBLT,
        );
        let len = area.w as usize * area.h as usize * 4;
        let mut pixels = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
        SelectObject(mem, old);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(mem);
        copied.map_err(|e| format!("BitBlt: {e}"))?;
        // GDI leaves alpha undefined (usually 0); a screenshot is opaque.
        for a in pixels.iter_mut().skip(3).step_by(4) {
            *a = 255;
        }
        Ok(Image {
            width: area.w as u32,
            height: area.h as u32,
            pixels,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub rect: Rect,
    /// The monitor less the taskbar and docked bars.
    pub work: RECT,
    /// 1.0 at 96 DPI.
    pub scale: f32,
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        unsafe {
            let list = &mut *(data.0 as *mut Vec<Monitor>);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(m, &mut info).as_bool() {
                let r = info.rcMonitor;
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                list.push(Monitor {
                    rect: Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top),
                    work: info.rcWork,
                    scale: dx as f32 / 96.0,
                });
            }
            true.into()
        }
    }
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

/// The monitor under a screen point, or the nearest one: its area, work
/// area and scale in one look.
pub fn monitor_under(x: i32, y: i32) -> Monitor {
    monitor_at(&monitors(), x, y)
}

/// The monitor holding the point, or the nearest one to it.
pub fn monitor_at(list: &[Monitor], x: i32, y: i32) -> Monitor {
    list.iter()
        .find(|m| m.rect.contains(x, y))
        .or_else(|| {
            list.iter().min_by_key(|m| {
                let cx = x.clamp(m.rect.x, m.rect.right() - 1);
                let cy = y.clamp(m.rect.y, m.rect.bottom() - 1);
                (cx - x).abs() + (cy - y).abs()
            })
        })
        .copied()
        .unwrap_or_else(|| {
            let v = virtual_screen();
            Monitor {
                rect: v,
                work: RECT {
                    left: v.x,
                    top: v.y,
                    right: v.right(),
                    bottom: v.bottom(),
                },
                scale: 1.0,
            }
        })
}

/// The visible top-level windows, front to back, as the user sees them —
/// taken before the overlay appears, so hovering highlights what was there.
pub fn windows_front_to_back() -> Vec<Rect> {
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> BOOL {
        unsafe {
            let list = &mut *(data.0 as *mut Vec<Rect>);
            if let Some(r) = visible_bounds(hwnd) {
                list.push(r);
            }
            true.into()
        }
    }
    let mut list: Vec<Rect> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

unsafe fn visible_bounds(hwnd: HWND) -> Option<Rect> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        // Click-through windows are not what the user points at.
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT.0 != 0 {
            return None;
        }
        // Our own windows are never a target.
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == std::process::id() {
            return None;
        }
        // Cloaked: suspended UWP apps and windows on other virtual desktops
        // report visible but are not on screen.
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
        if cloaked != 0 {
            return None;
        }
        // The frame without its invisible resize borders.
        let mut r = RECT::default();
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut _,
            size_of::<RECT>() as u32,
        )
        .ok()?;
        let rect = Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top);
        (rect.w > 1 && rect.h > 1).then_some(rect)
    }
}

/// Keeps our own windows (the overlay, the cards, the countdown) out of
/// every capture, ours and other programs'. A debug build started with
/// `SCM_SHOW_IN_CAPTURE` leaves them in, so tests can photograph them.
pub fn exclude_from_capture(hwnd: HWND) {
    if cfg!(debug_assertions) && std::env::var_os("SCM_SHOW_IN_CAPTURE").is_some() {
        return;
    }
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowDisplayAffinity(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::WDA_EXCLUDEFROMCAPTURE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grabbing_the_primary_screen_gives_opaque_pixels() {
        let m = monitors();
        assert!(!m.is_empty());
        let r = m[0].rect;
        let img = grab(Rect::new(r.x, r.y, 64, 48)).unwrap();
        assert_eq!((img.width, img.height), (64, 48));
        assert!(img.pixels.iter().skip(3).step_by(4).all(|a| *a == 255));
    }

    #[test]
    fn monitor_at_falls_back_to_the_nearest() {
        let list = [
            Monitor {
                rect: Rect::new(0, 0, 100, 100),
                work: RECT::default(),
                scale: 1.0,
            },
            Monitor {
                rect: Rect::new(100, 0, 100, 100),
                work: RECT::default(),
                scale: 2.0,
            },
        ];
        assert_eq!(monitor_at(&list, 150, 50).scale, 2.0);
        assert_eq!(monitor_at(&list, 500, 50).scale, 2.0);
        assert_eq!(monitor_at(&list, -50, 50).scale, 1.0);
    }
}
