//! The quick-access overlay: after a capture, a card with its thumbnail
//! appears in the bottom-left corner of the screen it was taken on.
//! Hovering shows Copiar and Guardar in the middle and close, pin,
//! annotate and delete in the corners; dragging the card drops the capture
//! as a file into another program; a double click opens the editor. New
//! cards stack upwards.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_BRUSH_PROPERTIES, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_ROUNDED_RECT,
    ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, KillTimer,
    MA_NOACTIVATE, RegisterClassExW, SM_CXDRAG, SM_CYDRAG, SW_SHOWNOACTIVATE, SetTimer, ShowWindow,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};
use windows_numerics::Matrix3x2;

use crate::gfx::{self, point, rectf, rgba};
use crate::icons::{Icon, Icons};
use crate::image::{self, Image};
use crate::layered::{self, Surface};
use crate::settings::ImageFormat;
use crate::util::lparam_point;
use crate::{agent, clipboard, dragdrop, flow, log};

const CLASS: PCWSTR = w!("ScreenShotCM.Quick");
const CLOSE_TIMER: usize = 3;
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Copy,
    Save,
    Close,
    Pin,
    Edit,
    Delete,
}

struct Card {
    hwnd: HWND,
    image: Rc<Image>,
    thumb: Image,
    path: PathBuf,
    /// The monitor's work area the card lives on.
    work: RECT,
    s: f32,
    /// The card itself, without the shadow margin.
    w: f32,
    h: f32,
    surface: Option<Surface>,
    hover: bool,
    hot: Option<Btn>,
    press: Option<(i32, i32)>,
    /// The button the mouse went down on; it acts if released over it.
    pressed_btn: Option<Btn>,
}

thread_local! {
    static CARDS: RefCell<Vec<Card>> = const { RefCell::new(Vec::new()) };
    static LAST_CLOSED: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn margin(s: f32) -> f32 {
    20.0 * s
}

/// Shows a card for a capture saved (or being saved) at `path`.
pub fn show(image: Image, path: PathBuf, near: (i32, i32)) {
    if let Err(e) = create(Rc::new(image), path, near) {
        log::error(&format!("quick overlay: {e}"));
    }
}

fn create(image: Rc<Image>, path: PathBuf, near: (i32, i32)) -> Result<(), String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
        let monitor = MonitorFromPoint(
            POINT {
                x: near.0,
                y: near.1,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut mi);
        let s = crate::capture::monitor_at(&crate::capture::monitors(), near.0, near.1).scale;

        let (max_w, max_h) = ((236.0 * s) as u32, (160.0 * s) as u32);
        let thumb = layered::thumbnail(&image, max_w, max_h);
        // A very thin capture still gets a card a person can aim at.
        let w = (thumb.width as f32).max(150.0 * s);
        let h = (thumb.height as f32).max(96.0 * s);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS,
            w!("ScreenShotCM"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        crate::capture::exclude_from_capture(hwnd);
        let card = Card {
            hwnd,
            image,
            thumb,
            path,
            work: mi.rcWork,
            s,
            w,
            h,
            surface: None,
            hover: false,
            hot: None,
            press: None,
            pressed_btn: None,
        };
        CARDS.with(|c| c.borrow_mut().push(card));
        let seconds = agent::settings().overlay_close_seconds;
        if seconds > 0 {
            SetTimer(Some(hwnd), CLOSE_TIMER, seconds * 1000, None);
        }
        reflow();
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(())
}

/// The editor finished a capture: refresh its card, or show one.
pub fn replaced(path: &Path, image: Image) {
    let found = CARDS.with(|c| {
        let mut cards = c.borrow_mut();
        let card = cards.iter_mut().find(|k| k.path == path)?;
        card.thumb = layered::thumbnail(&image, (236.0 * card.s) as u32, (160.0 * card.s) as u32);
        card.w = (card.thumb.width as f32).max(150.0 * card.s);
        card.h = (card.thumb.height as f32).max(96.0 * card.s);
        card.image = Rc::new(image.clone());
        card.surface = None;
        Some(())
    });
    if found.is_some() {
        reflow();
    } else {
        let mut p = POINT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p);
        }
        show(image, path.to_path_buf(), (p.x, p.y));
    }
}

/// Brings back the last card closed.
pub fn restore_last() {
    let Some(path) = LAST_CLOSED.with(|l| l.borrow_mut().take()) else {
        agent::notify(
            "Nada que restaurar",
            "No hay ninguna captura cerrada recientemente.",
            false,
        );
        return;
    };
    match image::load(&path) {
        Ok(img) => {
            let mut p = POINT::default();
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p);
            }
            show(img, path, (p.x, p.y));
        }
        Err(e) => agent::notify("No se pudo restaurar", &e.to_string(), true),
    }
}

/// Places every card: newest at the bottom, older ones above, per monitor.
fn reflow() {
    CARDS.with(|c| {
        let mut cards = c.borrow_mut();
        let mut stacks: Vec<(RECT, f32)> = Vec::new();
        for card in cards.iter_mut().rev() {
            let m = margin(card.s);
            let offset = match stacks.iter_mut().find(|(r, _)| *r == card.work) {
                Some((_, used)) => used,
                None => {
                    stacks.push((card.work, 0.0));
                    &mut stacks.last_mut().unwrap().1
                }
            };
            let x = card.work.left as f32 + 20.0 * card.s - m;
            let y = card.work.bottom as f32 - 20.0 * card.s - card.h - *offset - m;
            *offset += card.h + 12.0 * card.s;
            render(card, (x.round() as i32, y.round() as i32));
        }
    });
}

fn render(card: &mut Card, at: (i32, i32)) {
    let m = margin(card.s);
    let (sw, sh) = (
        (card.w + 2.0 * m).ceil() as i32,
        (card.h + 2.0 * m).ceil() as i32,
    );
    if card
        .surface
        .as_ref()
        .is_none_or(|s| s.width != sw || s.height != sh)
    {
        card.surface = gfx::d2d().and_then(|f| Surface::new(f, sw, sh)).ok();
    }
    let Some(surface) = &card.surface else { return };
    let _ = surface.draw(|t, brush| paint(card, surface, t, brush));
    let _ = surface.present(card.hwnd, at, 1.0);
}

fn rerender(hwnd: HWND) {
    CARDS.with(|c| {
        let mut cards = c.borrow_mut();
        if let Some(card) = cards.iter_mut().find(|k| k.hwnd == hwnd) {
            let mut r = RECT::default();
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut r);
            }
            render(card, (r.left, r.top));
        }
    });
}

fn buttons(card: &Card) -> Vec<(Btn, f32, f32, f32, f32)> {
    let s = card.s;
    let m = margin(s);
    let (x, y, w, h) = (m, m, card.w, card.h);
    let pill_w = 92.0 * s;
    let pill_h = 30.0 * s;
    let gap = 8.0 * s;
    let cy = y + h / 2.0;
    let d = 26.0 * s;
    let inset = 8.0 * s;
    vec![
        (
            Btn::Copy,
            x + (w - pill_w) / 2.0,
            cy - pill_h - gap / 2.0,
            pill_w,
            pill_h,
        ),
        (
            Btn::Save,
            x + (w - pill_w) / 2.0,
            cy + gap / 2.0,
            pill_w,
            pill_h,
        ),
        (Btn::Close, x + inset, y + inset, d, d),
        (Btn::Pin, x + w - inset - d, y + inset, d, d),
        (Btn::Edit, x + inset, y + h - inset - d, d, d),
        (Btn::Delete, x + w - inset - d, y + h - inset - d, d, d),
    ]
}

fn button_at(card: &Card, p: (f32, f32)) -> Option<Btn> {
    buttons(card)
        .into_iter()
        .find(|(_, x, y, w, h)| p.0 >= *x && p.0 < x + w && p.1 >= *y && p.1 < y + h)
        .map(|b| b.0)
}

fn paint(card: &Card, surface: &Surface, t: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush) {
    let s = card.s;
    let m = margin(s);
    let r = rectf(m, m, card.w, card.h);
    let radius = 10.0 * s;
    unsafe {
        crate::background::shadow(t, brush, r, radius, 0.9, 12.0 * s);
        brush.SetColor(&rgba(40, 40, 44, 1.0));
        t.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            brush,
        );
        // The thumbnail, centred, clipped to the card's rounding.
        if let Ok(bitmap) = gfx::bitmap(t, &card.thumb) {
            let (tw, th) = (card.thumb.width as f32, card.thumb.height as f32);
            let (tx, ty) = (m + (card.w - tw) / 2.0, m + (card.h - th) / 2.0);
            if let Ok(b) = t.CreateBitmapBrush(
                &bitmap,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                }),
                None,
            ) {
                b.SetTransform(&Matrix3x2 {
                    M11: 1.0,
                    M12: 0.0,
                    M21: 0.0,
                    M22: 1.0,
                    M31: tx,
                    M32: ty,
                });
                let inner = rectf(tx.max(m), ty.max(m), tw.min(card.w), th.min(card.h));
                t.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: inner,
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &b,
                );
            }
        }
        brush.SetColor(&rgba(255, 255, 255, 0.14));
        t.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            brush,
            1.0,
            None,
        );

        if !card.hover {
            return;
        }
        brush.SetColor(&rgba(0, 0, 0, 0.45));
        t.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            brush,
        );
        let round = surface
            .factory
            .CreateStrokeStyle(
                &windows::Win32::Graphics::Direct2D::D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: windows::Win32::Graphics::Direct2D::D2D1_CAP_STYLE_ROUND,
                    endCap: windows::Win32::Graphics::Direct2D::D2D1_CAP_STYLE_ROUND,
                    dashCap: windows::Win32::Graphics::Direct2D::D2D1_CAP_STYLE_ROUND,
                    lineJoin: windows::Win32::Graphics::Direct2D::D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    ..Default::default()
                },
                None,
            )
            .ok();
        let dwrite = gfx::dwrite().ok();
        let format = gfx::text_format(13.0 * s, DWRITE_FONT_WEIGHT_SEMI_BOLD).ok();
        if let Some(f) = &format {
            let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        }
        for (btn, x, y, w, h) in buttons(card) {
            let hot = card.hot == Some(btn);
            match btn {
                Btn::Copy | Btn::Save => {
                    brush.SetColor(&if hot {
                        rgba(255, 255, 255, 1.0)
                    } else {
                        rgba(255, 255, 255, 0.92)
                    });
                    t.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: rectf(x, y, w, h),
                            radiusX: h / 2.0,
                            radiusY: h / 2.0,
                        },
                        brush,
                    );
                    if let Some(f) = &format {
                        let label: Vec<u16> = if btn == Btn::Copy {
                            "Copiar"
                        } else {
                            "Guardar"
                        }
                        .encode_utf16()
                        .collect();
                        brush.SetColor(&rgba(20, 20, 22, 1.0));
                        t.DrawText(
                            &label,
                            f,
                            &rectf(x, y, w, h),
                            brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                            Default::default(),
                        );
                    }
                }
                _ => {
                    let e = D2D1_ELLIPSE {
                        point: point(x + w / 2.0, y + h / 2.0),
                        radiusX: w / 2.0,
                        radiusY: h / 2.0,
                    };
                    brush.SetColor(&if hot {
                        rgba(255, 255, 255, 1.0)
                    } else {
                        rgba(255, 255, 255, 0.9)
                    });
                    t.FillEllipse(&e, brush);
                    if let (Some(round), Some(dw)) = (&round, &dwrite) {
                        let icons = Icons {
                            factory: &surface.factory,
                            dwrite: dw,
                            round,
                        };
                        let icon = match btn {
                            Btn::Close => Icon::Close,
                            Btn::Pin => Icon::Pin,
                            Btn::Edit => Icon::Pen,
                            _ => Icon::Trash,
                        };
                        let color = if btn == Btn::Delete && hot {
                            rgba(220, 38, 38, 1.0)
                        } else {
                            rgba(20, 20, 22, 1.0)
                        };
                        icons.draw(t, brush, icon, x + w / 2.0, y + h / 2.0, 14.0 * s, color);
                    }
                }
            }
        }
    }
}

/// Removes a card and its window; remembered for "restaurar".
fn close(hwnd: HWND) {
    let removed = CARDS.with(|c| {
        let mut cards = c.borrow_mut();
        let i = cards.iter().position(|k| k.hwnd == hwnd)?;
        Some(cards.remove(i))
    });
    if let Some(card) = removed {
        LAST_CLOSED.with(|l| *l.borrow_mut() = Some(card.path.clone()));
        unsafe {
            let _ = KillTimer(Some(hwnd), CLOSE_TIMER);
            let _ = DestroyWindow(hwnd);
        }
        reflow();
    }
}

/// The file behind a card, written now if the history writer has not yet.
fn ensure_file(image: &Image, path: &Path) -> bool {
    if path.exists() {
        return true;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    image::save(image, path, ImageFormat::Png, 100).is_ok()
}

fn press(hwnd: HWND, btn: Btn) {
    let Some((image, path)) = CARDS.with(|c| {
        c.borrow()
            .iter()
            .find(|k| k.hwnd == hwnd)
            .map(|k| (k.image.clone(), k.path.clone()))
    }) else {
        return;
    };
    match btn {
        Btn::Copy => match clipboard::copy_image(agent::hwnd(), &image) {
            Ok(()) => close(hwnd),
            Err(e) => agent::notify("No se pudo copiar", &e, true),
        },
        Btn::Save => {
            let settings = agent::settings();
            let target = flow::save_path(&settings);
            if let Some(dir) = target.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match image::save(
                &image,
                &target,
                settings.image_format,
                settings.jpeg_quality,
            ) {
                Ok(()) => {
                    log::info(&format!("saved {}", target.display()));
                    close(hwnd);
                }
                Err(e) => agent::notify("No se pudo guardar", &e.to_string(), true),
            }
        }
        Btn::Close => close(hwnd),
        Btn::Pin => {
            crate::pin::open((*image).clone(), Some(path));
            close(hwnd);
        }
        Btn::Edit => {
            ensure_file(&image, &path);
            close(hwnd);
            crate::editor::open((*image).clone(), Some(path));
        }
        Btn::Delete => {
            let _ = std::fs::remove_file(&path);
            close(hwnd);
            LAST_CLOSED.with(|l| *l.borrow_mut() = None);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // Clicking a card never takes the focus from what the user works in.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_MOUSEMOVE => {
                let (x, y) = lparam_point(lparam.0);
                let mut start_drag = None;
                let changed = CARDS.with(|c| {
                    let mut cards = c.borrow_mut();
                    let card = cards.iter_mut().find(|k| k.hwnd == hwnd)?;
                    if let Some((px, py)) = card.press {
                        let (dx, dy) = (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG));
                        if (x - px).abs() > dx || (y - py).abs() > dy {
                            card.press = None;
                            start_drag = Some((card.image.clone(), card.path.clone()));
                        }
                    }
                    let hot = button_at(card, (x as f32, y as f32));
                    let changed = !card.hover || hot != card.hot;
                    if !card.hover {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    card.hover = true;
                    card.hot = hot;
                    Some(changed)
                });
                if let Some((image, path)) = start_drag {
                    let _ = ReleaseCapture();
                    if ensure_file(&image, &path) && dragdrop::drag_file(hwnd, &path) {
                        close(hwnd);
                        return LRESULT(0);
                    }
                }
                if changed == Some(true) {
                    rerender(hwnd);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                CARDS.with(|c| {
                    if let Some(card) = c.borrow_mut().iter_mut().find(|k| k.hwnd == hwnd) {
                        card.hover = false;
                        card.hot = None;
                    }
                });
                rerender(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lparam_point(lparam.0);
                CARDS.with(|c| {
                    if let Some(card) = c.borrow_mut().iter_mut().find(|k| k.hwnd == hwnd) {
                        // Hovering stops the auto-close; so does a press.
                        let _ = KillTimer(Some(hwnd), CLOSE_TIMER);
                        card.pressed_btn = button_at(card, (x as f32, y as f32));
                        if card.pressed_btn.is_none() {
                            card.press = Some((x, y));
                        }
                    }
                });
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = lparam_point(lparam.0);
                let _ = ReleaseCapture();
                let btn = CARDS.with(|c| {
                    let mut cards = c.borrow_mut();
                    let card = cards.iter_mut().find(|k| k.hwnd == hwnd)?;
                    card.press = None;
                    let pressed = card.pressed_btn.take()?;
                    button_at(card, (x as f32, y as f32)).filter(|b| *b == pressed)
                });
                if let Some(b) = btn {
                    press(hwnd, b);
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = lparam_point(lparam.0);
                let on_button = CARDS.with(|c| {
                    c.borrow()
                        .iter()
                        .find(|k| k.hwnd == hwnd)
                        .map(|k| button_at(k, (x as f32, y as f32)).is_some())
                });
                if on_button == Some(false) {
                    press(hwnd, Btn::Edit);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == CLOSE_TIMER => {
                let hovering = CARDS
                    .with(|c| c.borrow().iter().find(|k| k.hwnd == hwnd).map(|k| k.hover))
                    .unwrap_or(false);
                if !hovering {
                    close(hwnd);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
