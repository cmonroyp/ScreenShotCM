//! Small conversions every module needs.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

/// A NUL-terminated UTF-16 copy of `s`, for the W entry points.
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// Copies `s` into a fixed UTF-16 field, truncated and always terminated.
pub fn copy_into(field: &mut [u16], s: &str) {
    let mut n = 0;
    for unit in s.encode_utf16() {
        if n + 1 >= field.len() {
            break;
        }
        field[n] = unit;
        n += 1;
    }
    field[n] = 0;
}

/// The low and high signed words of an LPARAM, as mouse messages pack them.
pub fn lparam_point(lparam: isize) -> (i32, i32) {
    let x = (lparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// The rectangle spanned by two corners, in either order.
    pub fn from_points(a: (i32, i32), b: (i32, i32)) -> Self {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        Self::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 {
            Rect::default()
        } else {
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }

    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

/// Asks Windows for one `WM_MOUSELEAVE` when the pointer leaves `hwnd`: how
/// a hovered button or card knows to go back to rest.
pub fn track_leave(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    };
    let mut tme = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe {
        let _ = TrackMouseEvent(&mut tme);
    }
}

/// Segoe UI for a GDI window, `height` in device units as `CreateFontW`
/// takes it (negative: the character height). The caller deletes it.
pub fn ui_font(
    height: i32,
    weight: windows::Win32::Graphics::Gdi::FONT_WEIGHT,
) -> windows::Win32::Graphics::Gdi::HFONT {
    use windows::Win32::Graphics::Gdi::{CLEARTYPE_QUALITY, CreateFontW};
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight.0 as i32,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            CLEARTYPE_QUALITY,
            0,
            windows::core::w!("Segoe UI"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_from_points_normalises_any_corner_order() {
        assert_eq!(Rect::from_points((10, 20), (4, 2)), Rect::new(4, 2, 6, 18));
    }

    #[test]
    fn rect_intersection_of_disjoint_rects_is_empty() {
        let a = Rect::new(0, 0, 10, 10);
        assert!(a.intersect(&Rect::new(10, 0, 5, 5)).is_empty());
        assert_eq!(a.intersect(&Rect::new(5, 5, 10, 10)), Rect::new(5, 5, 5, 5));
    }

    #[test]
    fn lparam_point_keeps_negative_coordinates() {
        // A monitor left of the primary one has negative x.
        let lp = ((-5i32 as u16 as isize) << 16) | (-300i32 as u16 as isize);
        assert_eq!(lparam_point(lp), (-300, -5));
    }

    #[test]
    fn copy_into_truncates_and_terminates() {
        let mut f = [1u16; 4];
        copy_into(&mut f, "abcdef");
        assert_eq!(f, [b'a' as u16, b'b' as u16, b'c' as u16, 0]);
    }
}
