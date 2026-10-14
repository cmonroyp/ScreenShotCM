//! Scrolling capture: after an area is chosen, the screen under it is read
//! again and again while its content scrolls — by the user's wheel, or by
//! the app with "Desplazar solo" — and every strip that appears is stitched
//! below the last (see `stitch`). A border marks the area, a bar under it
//! offers Auto, Done and Cancel, and a preview beside it grows with the
//! image. All three are excluded from capture, so none of them can end up
//! in the picture, and the border and the preview let clicks and the wheel
//! through to the page.

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ROUNDED_RECT, ID2D1Factory, ID2D1RenderTarget,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT, RegisterHotKey,
    ReleaseCapture, SendInput, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    UnregisterHotKey, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, KillTimer, MA_NOACTIVATE,
    RegisterClassExW, SW_SHOWNOACTIVATE, SetCursorPos, SetTimer, ShowWindow, WM_HOTKEY,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::gfx::{self, ACCENT, rectf, rgba};
use crate::layered::{self, Surface};
use crate::stitch::{Step, Stitcher};
use crate::util::{Rect, lparam_point};
use crate::{capture, log};

const BAR_CLASS: PCWSTR = w!("ScreenShotCM.Scroll");
const PANE_CLASS: PCWSTR = w!("ScreenShotCM.ScrollPane");
const TICK: usize = 1;
/// How often the area is read: fast enough that a wheel notch moves the
/// content less than the area's height between two reads.
const TICK_MS: u32 = 60;
/// Auto-scroll sends one wheel notch every this many reads, so a smooth
/// scroll has settled before the next.
const NOTCH_EVERY: u32 = 3;
/// Auto-scroll stops at the end of the page: this many reads with nothing new.
const END_AFTER: u32 = 20;
const HK_DONE: i32 = 0x5C01;
const HK_CANCEL: i32 = 0x5C02;
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Auto,
    Done,
    Cancel,
}

struct Pane {
    hwnd: HWND,
    at: (i32, i32),
    surface: Surface,
}

struct Session {
    area: Rect,
    s: f32,
    bar: Pane,
    frame: Pane,
    preview: Option<Pane>,
    stitcher: Stitcher,
    auto: bool,
    idle: u32,
    reads: u32,
    /// Reads that added rows, that showed nothing new, that did not fit.
    counts: (u32, u32, u32),
    lost: bool,
    hot: Option<Btn>,
    pressed: Option<Btn>,
    hovering: bool,
    /// Where the pointer was before auto-scroll moved it over the area.
    home: Option<POINT>,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

pub fn is_active() -> bool {
    SESSION.with(|s| s.try_borrow().map(|s| s.is_some()).unwrap_or(true))
}

fn with<R>(f: impl FnOnce(&mut Session) -> R) -> Option<R> {
    SESSION.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

/// Starts a scrolling capture of `area` (virtual-screen pixels).
pub fn start(area: Rect) -> Result<(), String> {
    if is_active() {
        return Ok(());
    }
    let centre = (area.x + area.w / 2, area.y + area.h / 2);
    let s = capture::monitor_at(&capture::monitors(), centre.0, centre.1).scale;
    let work = unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: centre.0,
                y: centre.1,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut mi);
        mi.rcWork
    };
    let first = capture::grab(area)?;
    let px = |v: f32| (v * s).round() as i32;

    // The border: just outside the area, so it never covers a pixel of it.
    let m = px(4.0);
    let frame_rect = Rect::new(area.x - m, area.y - m, area.w + 2 * m, area.h + 2 * m);
    let frame = pane(frame_rect, true)?;

    // The bar: under the area, or inside its bottom edge when there is no
    // room below (it is excluded from capture either way).
    let (bw, bh) = (px(420.0), px(44.0));
    let by = if area.bottom() + px(12.0) + bh <= work.bottom {
        area.bottom() + px(12.0)
    } else {
        area.bottom() - bh - px(16.0)
    };
    let bx = (area.x + area.w / 2 - bw / 2).clamp(work.left, (work.right - bw).max(work.left));
    let bar = pane(Rect::new(bx, by, bw, bh), false)?;

    // The preview: beside the area, on whichever side has room.
    let (pw, ph) = (px(132.0), area.h.min(px(520.0)).max(px(120.0)));
    let px_right = area.right() + m + px(10.0);
    let px_left = area.x - m - px(10.0) - pw;
    let preview_x = if px_right + pw <= work.right {
        Some(px_right)
    } else if px_left >= work.left {
        Some(px_left)
    } else {
        None
    };
    let preview = match preview_x {
        Some(x) => Some(pane(Rect::new(x, area.y, pw, ph), true)?),
        None => None,
    };

    // As tall as the editor can show and a saved file can be opened again.
    let limit = bar.surface.max_bitmap_size() as usize;
    let by_pixels = 100_000_000 / area.w.max(1) as usize;
    let max_rows = limit.min(20_000).min(by_pixels);

    let session = Session {
        area,
        s,
        bar,
        frame,
        preview,
        stitcher: Stitcher::new(&first, max_rows),
        auto: false,
        idle: 0,
        reads: 0,
        counts: (0, 0, 0),
        lost: false,
        hot: None,
        pressed: None,
        hovering: false,
        home: None,
    };
    let bar_hwnd = session.bar.hwnd;
    SESSION.with(|c| *c.borrow_mut() = Some(session));
    unsafe {
        // Enter finishes and Esc cancels while the session lasts; the page
        // keeps the focus, so the keys are taken as hotkeys and given back.
        let _ = RegisterHotKey(
            Some(bar_hwnd),
            HK_DONE,
            HOT_KEY_MODIFIERS(0),
            VK_RETURN.0 as u32,
        );
        let _ = RegisterHotKey(
            Some(bar_hwnd),
            HK_CANCEL,
            HOT_KEY_MODIFIERS(0),
            VK_ESCAPE.0 as u32,
        );
        SetTimer(Some(bar_hwnd), TICK, TICK_MS, None);
    }
    with(|s| {
        render_frame(s);
        render_bar(s);
        render_preview(s);
        for p in [Some(&s.frame), Some(&s.bar), s.preview.as_ref()]
            .into_iter()
            .flatten()
        {
            unsafe {
                let _ = ShowWindow(p.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    });
    log::info(&format!("scrolling capture {}x{}", area.w, area.h));
    Ok(())
}

/// A layered, topmost window that never takes the focus; `through` lets the
/// mouse pass to whatever is under it.
fn pane(r: Rect, through: bool) -> Result<Pane, String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: if through { PANE_CLASS } else { BAR_CLASS },
            ..Default::default()
        };
        RegisterClassExW(&class);
        let mut ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        if through {
            ex |= WS_EX_TRANSPARENT;
        }
        let hwnd = CreateWindowExW(
            ex,
            if through { PANE_CLASS } else { BAR_CLASS },
            w!("ScreenShotCM"),
            WS_POPUP,
            r.x,
            r.y,
            r.w,
            r.h,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        capture::exclude_from_capture(hwnd);
        let surface = match gfx::d2d().and_then(|f: ID2D1Factory| Surface::new(f, r.w, r.h)) {
            Ok(s) => s,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(e.to_string());
            }
        };
        Ok(Pane {
            hwnd,
            at: (r.x, r.y),
            surface,
        })
    }
}

fn render_frame(s: &Session) {
    let p = &s.frame;
    let (w, h) = (p.surface.width as f32, p.surface.height as f32);
    let stroke = 2.0 * s.s;
    let _ = p.surface.draw(|t, brush| unsafe {
        brush.SetColor(&rgba(ACCENT.0, ACCENT.1, ACCENT.2, 1.0));
        t.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rectf(stroke / 2.0, stroke / 2.0, w - stroke, h - stroke),
                radiusX: 4.0 * s.s,
                radiusY: 4.0 * s.s,
            },
            brush,
            stroke,
            None,
        );
    });
    let _ = p.surface.present(p.hwnd, p.at, 1.0);
}

/// The bar's buttons, left to right, in surface pixels.
fn buttons(s: &Session) -> [(Btn, f32, f32, f32, f32); 3] {
    let k = s.s;
    let (pad, gap, bh) = (6.0 * k, 6.0 * k, 32.0 * k);
    let y = (s.bar.surface.height as f32 - bh) / 2.0;
    let total = s.bar.surface.width as f32;
    let cancel_w = 92.0 * k;
    let done_w = 84.0 * k;
    let auto_w = 150.0 * k;
    let cancel_x = total - pad - cancel_w;
    let done_x = cancel_x - gap - done_w;
    let auto_x = done_x - gap - auto_w;
    [
        (Btn::Auto, auto_x, y, auto_w, bh),
        (Btn::Done, done_x, y, done_w, bh),
        (Btn::Cancel, cancel_x, y, cancel_w, bh),
    ]
}

fn button_at(s: &Session, p: (f32, f32)) -> Option<Btn> {
    buttons(s)
        .into_iter()
        .find(|&(_, x, y, w, h)| p.0 >= x && p.0 < x + w && p.1 >= y && p.1 < y + h)
        .map(|(b, ..)| b)
}

fn status(s: &Session) -> String {
    let rows = s.stitcher.rows();
    if s.stitcher.is_full() {
        crate::i18n::tr("Altura máxima", "Height limit").into()
    } else if s.lost {
        crate::i18n::tr("Más despacio", "Scroll slower").into()
    } else {
        format!(
            "{} px",
            group(rows, if crate::i18n::english() { ',' } else { '.' })
        )
    }
}

/// 12345 → "12.345" (Spanish) or "12,345" (English).
fn group(n: usize, sep: char) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

fn render_bar(s: &Session) {
    let p = &s.bar;
    let k = s.s;
    let (w, h) = (p.surface.width as f32, p.surface.height as f32);
    let format = gfx::text_format(13.0 * k, DWRITE_FONT_WEIGHT_SEMI_BOLD).ok();
    let _ = p.surface.draw(|t, brush| unsafe {
        let r = rectf(0.0, 0.0, w, h);
        brush.SetColor(&rgba(32, 32, 36, 0.96));
        t.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: h / 2.0,
                radiusY: h / 2.0,
            },
            brush,
        );
        brush.SetColor(&rgba(255, 255, 255, 0.12));
        t.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rectf(0.5, 0.5, w - 1.0, h - 1.0),
                radiusX: h / 2.0,
                radiusY: h / 2.0,
            },
            brush,
            1.0,
            None,
        );
        let Some(f) = &format else { return };
        let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
        let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        let buttons = buttons(s);
        // The size so far, left of the buttons.
        let text_w = buttons[0].1 - 8.0 * k;
        brush.SetColor(&if s.lost {
            rgba(251, 191, 36, 1.0)
        } else {
            rgba(255, 255, 255, 0.85)
        });
        draw_text(t, brush, f, &status(s), rectf(8.0 * k, 0.0, text_w, h));
        for (btn, x, y, bw, bh) in buttons {
            let hot = s.hot == Some(btn);
            let (fill, ink) = match btn {
                Btn::Done => (
                    rgba(ACCENT.0, ACCENT.1, ACCENT.2, if hot { 1.0 } else { 0.92 }),
                    rgba(255, 255, 255, 1.0),
                ),
                Btn::Auto if s.auto => (rgba(255, 255, 255, 0.95), rgba(20, 20, 22, 1.0)),
                _ => (
                    rgba(255, 255, 255, if hot { 0.2 } else { 0.1 }),
                    rgba(255, 255, 255, 1.0),
                ),
            };
            brush.SetColor(&fill);
            t.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rectf(x, y, bw, bh),
                    radiusX: bh / 2.0,
                    radiusY: bh / 2.0,
                },
                brush,
            );
            let label = match btn {
                Btn::Auto if s.auto => crate::i18n::tr("Pausar", "Pause"),
                Btn::Auto => crate::i18n::tr("↓ Desplazar solo", "↓ Auto-scroll"),
                Btn::Done => crate::i18n::tr("Listo", "Done"),
                Btn::Cancel => crate::i18n::tr("Cancelar", "Cancel"),
            };
            brush.SetColor(&ink);
            draw_text(t, brush, f, label, rectf(x, y, bw, bh));
        }
    });
    let _ = p.surface.present(p.hwnd, p.at, 1.0);
}

fn draw_text(
    t: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    format: &windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    text: &str,
    r: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F,
) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        t.DrawText(
            &wide,
            format,
            &r,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            Default::default(),
        );
    }
}

/// The latest part of the image, scaled to the preview's width, with its
/// bottom — what just arrived — at the bottom of the pane.
fn render_preview(s: &Session) {
    let Some(p) = &s.preview else { return };
    let k = s.s;
    let (w, h) = (p.surface.width as f32, p.surface.height as f32);
    let inset = 6.0 * k;
    let (iw, ih) = (w - 2.0 * inset, h - 2.0 * inset);
    let source_rows = (ih * s.area.w as f32 / iw).ceil() as usize;
    let thumb = layered::thumbnail(&s.stitcher.tail(source_rows), iw as u32, ih as u32);
    let _ = p.surface.draw(|t, brush| unsafe {
        let card = rectf(0.0, 0.0, w, h);
        brush.SetColor(&rgba(32, 32, 36, 0.92));
        t.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: card,
                radiusX: 8.0 * k,
                radiusY: 8.0 * k,
            },
            brush,
        );
        if let Ok(bitmap) = gfx::bitmap(t, &thumb) {
            let (tw, th) = (thumb.width as f32, thumb.height as f32);
            let x = (w - tw) / 2.0;
            let y = h - inset - th;
            t.DrawBitmap(
                &bitmap,
                Some(&rectf(x, y, tw, th)),
                1.0,
                windows::Win32::Graphics::Direct2D::D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }
    });
    let _ = p.surface.present(p.hwnd, p.at, 1.0);
}

/// One read of the area, and auto-scroll's next notch.
fn tick() {
    let mut end = false;
    with(|s| {
        s.reads += 1;
        let frame = match capture::grab(s.area) {
            Ok(f) => f,
            Err(e) => {
                log::error(&format!("scrolling capture read: {e}"));
                return;
            }
        };
        let was_lost = s.lost;
        match s.stitcher.push(&frame) {
            Step::Added(_) => {
                s.counts.0 += 1;
                s.idle = 0;
                s.lost = false;
                render_preview(s);
                render_bar(s);
            }
            Step::Same => {
                s.counts.1 += 1;
                s.idle += 1;
                if was_lost {
                    s.lost = false;
                    render_bar(s);
                }
            }
            Step::Lost => {
                s.counts.2 += 1;
                s.idle += 1;
                s.lost = true;
                if !was_lost {
                    render_bar(s);
                }
            }
            Step::Full => {
                render_bar(s);
                end = true;
                return;
            }
        }
        if s.auto {
            let mut cursor = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut cursor);
            }
            // The wheel goes to the window under the pointer: send it only
            // while the pointer is over the area.
            if s.area.contains(cursor.x, cursor.y) {
                if s.reads % NOTCH_EVERY == 0 {
                    wheel_down();
                }
                if s.idle >= END_AFTER {
                    end = true;
                }
            } else {
                s.idle = 0;
            }
        }
    });
    if end {
        finish(true);
    }
}

fn wheel_down() {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                mouseData: (-120i32) as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                ..Default::default()
            },
        },
    };
    unsafe {
        SendInput(&[input], size_of::<INPUT>() as i32);
    }
}

fn toggle_auto() {
    with(|s| {
        s.auto = !s.auto;
        s.idle = 0;
        if s.auto {
            let mut cursor = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut cursor);
                if !s.area.contains(cursor.x, cursor.y) {
                    if s.home.is_none() {
                        s.home = Some(cursor);
                    }
                    let _ = SetCursorPos(s.area.x + s.area.w / 2, s.area.y + s.area.h / 2);
                }
            }
        }
        render_bar(s);
    });
}

/// Ends the session; with `keep`, what was stitched becomes the capture.
fn finish(keep: bool) {
    let Some(s) = SESSION.with(|c| c.try_borrow_mut().ok().and_then(|mut c| c.take())) else {
        return;
    };
    unsafe {
        let _ = KillTimer(Some(s.bar.hwnd), TICK);
        let _ = UnregisterHotKey(Some(s.bar.hwnd), HK_DONE);
        let _ = UnregisterHotKey(Some(s.bar.hwnd), HK_CANCEL);
        for p in [Some(&s.bar), Some(&s.frame), s.preview.as_ref()]
            .into_iter()
            .flatten()
        {
            let _ = DestroyWindow(p.hwnd);
        }
        if let Some(home) = s.home {
            let _ = SetCursorPos(home.x, home.y);
        }
    }
    if !keep {
        log::info("scrolling capture cancelled");
        return;
    }
    let image = s.stitcher.image();
    log::info(&format!(
        "scrolling capture: {}x{} from {} reads ({} added, {} same, {} lost)",
        image.width, image.height, s.reads, s.counts.0, s.counts.1, s.counts.2
    ));
    crate::flow::deliver(image, (s.area.x + s.area.w / 2, s.area.y + s.area.h / 2));
}

fn press(btn: Btn) {
    match btn {
        Btn::Auto => toggle_auto(),
        Btn::Done => finish(true),
        Btn::Cancel => finish(false),
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_TIMER if wparam.0 == TICK => {
                tick();
                LRESULT(0)
            }
            WM_HOTKEY => {
                match wparam.0 as i32 {
                    HK_DONE => finish(true),
                    HK_CANCEL => finish(false),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_point(lparam.0);
                with(|s| {
                    if !s.hovering {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut tme);
                        s.hovering = true;
                    }
                    let hot = button_at(s, (x as f32, y as f32));
                    if hot != s.hot {
                        s.hot = hot;
                        render_bar(s);
                    }
                });
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with(|s| {
                    s.hovering = false;
                    if s.hot.take().is_some() {
                        render_bar(s);
                    }
                });
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lparam_point(lparam.0);
                with(|s| s.pressed = button_at(s, (x as f32, y as f32)));
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = lparam_point(lparam.0);
                let _ = ReleaseCapture();
                // A button acts when pressed and released on itself.
                let btn = with(|s| {
                    let pressed = s.pressed.take()?;
                    button_at(s, (x as f32, y as f32)).filter(|b| *b == pressed)
                })
                .flatten();
                if let Some(b) = btn {
                    press(b);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::group;

    #[test]
    fn sizes_are_grouped_by_thousands() {
        assert_eq!(group(7, '.'), "7");
        assert_eq!(group(1234, '.'), "1.234");
        assert_eq!(group(12345678, ','), "12,345,678");
    }
}
