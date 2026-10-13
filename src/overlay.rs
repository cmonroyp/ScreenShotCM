//! The selection overlay: the screen is frozen
//! first and painted back unchanged; a thin crosshair spans the screen with
//! the coordinates beside the cursor and a magnifier below-right of it;
//! dragging dims everything outside the selection and shows its size.
//! Space switches to window mode, where the window under the cursor is
//! tinted and a click takes it.
//!
//! One borderless topmost window covers the whole virtual screen, drawn by
//! Direct2D in physical pixels, so mixed-DPI monitors need no stitching.

use std::cell::RefCell;

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_BRUSH_PROPERTIES, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_EXTEND_MODE_CLAMP, D2D1_ROUNDED_RECT, ID2D1Bitmap,
    ID2D1BitmapBrush, ID2D1HwndRenderTarget, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_TEXT_METRICS, IDWriteTextFormat,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN,
    VK_RIGHT, VK_SHIFT, VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, IDC_CROSS, LoadCursorW,
    RegisterClassExW, SW_SHOW, SetCursor, SetCursorPos, SetForegroundWindow, ShowWindow,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};
use windows_numerics::Matrix3x2;

use crate::capture::{self, Monitor};
use crate::gfx::{self, ACCENT, point, rect, rectf, rgba};
use crate::image::Image;
use crate::util::{Rect, lparam_point};

const CLASS: PCWSTR = w!("ScreenShotCM.Overlay");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Area,
    Window,
}

/// What the overlay hands back: the chosen rectangle in virtual-screen
/// coordinates, and the frozen screen it was chosen on.
pub type Done = Box<dyn FnOnce(Option<Rect>, Image)>;

struct Gfx {
    target: ID2D1HwndRenderTarget,
    frozen: ID2D1Bitmap,
    zoom: ID2D1BitmapBrush,
    brush: ID2D1SolidColorBrush,
    small: IDWriteTextFormat,
    label: IDWriteTextFormat,
}

struct Overlay {
    hwnd: HWND,
    /// The virtual screen; client (0, 0) is its top-left corner.
    screen: Rect,
    frame: Image,
    windows: Vec<Rect>,
    monitors: Vec<Monitor>,
    mode: Mode,
    cursor: (i32, i32),
    anchor: Option<(i32, i32)>,
    magnifier: bool,
    crosshair: bool,
    gfx: Option<Gfx>,
    done: Option<Done>,
    /// Frames painted, their total and slowest time in microseconds: logged
    /// when the overlay closes, to keep the drawing honest.
    frames: (u32, u64, u64),
}

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

pub fn is_open() -> bool {
    OVERLAY.with(|o| o.try_borrow().map(|o| o.is_some()).unwrap_or(true))
}

fn with<R>(f: impl FnOnce(&mut Overlay) -> R) -> Option<R> {
    OVERLAY.with(|o| o.try_borrow_mut().ok().and_then(|mut o| o.as_mut().map(f)))
}

/// Freezes the screen and opens the overlay. `done` runs once, after the
/// overlay is gone, with the selection or `None` when it was cancelled.
pub fn open(mode: Mode, magnifier: bool, crosshair: bool, done: Done) -> Result<(), String> {
    if is_open() {
        return Err(crate::i18n::tr(
            "ya hay una selección abierta",
            "a selection is already open",
        )
        .into());
    }
    let screen = capture::virtual_screen();
    // Windows first: they are what the user sees right now.
    let windows = capture::windows_front_to_back();
    let frame = capture::grab(screen)?;
    let monitors = capture::monitors();
    let mut cursor = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut cursor);
    }

    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        // A second registration fails harmlessly.
        RegisterClassExW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            w!("ScreenShotCM"),
            WS_POPUP,
            screen.x,
            screen.y,
            screen.w,
            screen.h,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        // Never in a picture, should anything capture while it is up. A
        // debug build can be told otherwise, to photograph the overlay itself.
        crate::capture::exclude_from_capture(hwnd);

        let mut overlay = Overlay {
            hwnd,
            screen,
            frame,
            windows: Vec::new(),
            monitors,
            mode,
            cursor: (cursor.x - screen.x, cursor.y - screen.y),
            anchor: None,
            magnifier,
            crosshair,
            gfx: None,
            done: Some(done),
            frames: (0, 0, 0),
        };
        overlay.windows = windows
            .iter()
            .map(|r| r.offset(-screen.x, -screen.y))
            .collect();
        overlay.gfx = make_gfx(&overlay).ok();
        if overlay.gfx.is_none() {
            let _ = DestroyWindow(hwnd);
            return Err(crate::i18n::tr(
                "Direct2D no pudo crear la superficie de dibujo",
                "Direct2D could not create the drawing surface",
            )
            .into());
        }
        OVERLAY.with(|o| *o.borrow_mut() = Some(overlay));

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
    Ok(())
}

fn make_gfx(o: &Overlay) -> windows::core::Result<Gfx> {
    let factory = gfx::d2d()?;
    let target = gfx::window_target(&factory, o.hwnd, o.screen.w as u32, o.screen.h as u32)?;
    let frozen = gfx::bitmap(&target, &o.frame)?;
    let brush = unsafe { target.CreateSolidColorBrush(&rgba(0, 0, 0, 1.0), None)? };
    let zoom = unsafe {
        target.CreateBitmapBrush(
            &frozen,
            Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            }),
            None,
        )?
    };
    let scale = capture::monitor_at(
        &o.monitors,
        o.cursor.0 + o.screen.x,
        o.cursor.1 + o.screen.y,
    )
    .scale;
    Ok(Gfx {
        target,
        frozen,
        zoom,
        brush,
        small: gfx::text_format(10.0 * scale, DWRITE_FONT_WEIGHT_MEDIUM)?,
        label: gfx::text_format(12.0 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
    })
}

/// Ends the overlay. The state is taken out before the window goes, so the
/// messages `DestroyWindow` sends find nothing to borrow.
fn finish(selection: Option<Rect>) {
    let Some(mut o) = OVERLAY.with(|o| o.borrow_mut().take()) else {
        return;
    };
    unsafe {
        let _ = ReleaseCapture();
        let _ = DestroyWindow(o.hwnd);
    }
    o.gfx = None;
    let (n, total, max) = o.frames;
    if n > 0 {
        crate::log::info(&format!(
            "overlay: {n} frames, {:.1} ms average, {:.1} ms slowest",
            total as f64 / n as f64 / 1000.0,
            max as f64 / 1000.0
        ));
    }
    let screen = o.screen;
    let chosen = selection
        .map(|r| r.intersect(&Rect::new(0, 0, screen.w, screen.h)))
        .filter(|r| !r.is_empty())
        .map(|r| r.offset(screen.x, screen.y));
    if let Some(done) = o.done.take() {
        done(chosen, o.frame);
    }
}

impl Overlay {
    fn selection(&self) -> Option<Rect> {
        let a = self.anchor?;
        let mut b = self.cursor;
        // Shift keeps it square, measured from the anchor.
        if unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0 {
            let side = (b.0 - a.0).abs().max((b.1 - a.1).abs());
            b = (
                a.0 + side * (b.0 - a.0).signum(),
                a.1 + side * (b.1 - a.1).signum(),
            );
        }
        // The pixel under the cursor is included.
        let r = Rect::from_points(a, b);
        Some(Rect::new(r.x, r.y, r.w + 1, r.h + 1))
    }

    fn hovered_window(&self) -> Rect {
        let (x, y) = self.cursor;
        self.windows
            .iter()
            .find(|r| r.contains(x, y))
            .copied()
            .unwrap_or_else(|| {
                capture::monitor_at(&self.monitors, x + self.screen.x, y + self.screen.y)
                    .rect
                    .offset(-self.screen.x, -self.screen.y)
            })
    }

    fn scale(&self) -> f32 {
        capture::monitor_at(
            &self.monitors,
            self.cursor.0 + self.screen.x,
            self.cursor.1 + self.screen.y,
        )
        .scale
    }

    /// The monitor under the cursor, in client coordinates.
    fn monitor_rect(&self) -> Rect {
        capture::monitor_at(
            &self.monitors,
            self.cursor.0 + self.screen.x,
            self.cursor.1 + self.screen.y,
        )
        .rect
        .offset(-self.screen.x, -self.screen.y)
    }

    fn paint(&mut self) {
        let started = std::time::Instant::now();
        self.draw();
        let us = started.elapsed().as_micros() as u64;
        self.frames = (self.frames.0 + 1, self.frames.1 + us, self.frames.2.max(us));
    }

    fn draw(&mut self) {
        let Some(g) = &self.gfx else { return };
        let t: &ID2D1RenderTarget = &g.target;
        let s = self.scale();
        unsafe {
            t.BeginDraw();
            let full = rect(Rect::new(0, 0, self.screen.w, self.screen.h));
            t.DrawBitmap(
                &g.frozen,
                Some(&full),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
            );

            match (self.mode, self.selection()) {
                (Mode::Area, Some(sel)) => {
                    self.dim_outside(t, &g.brush, sel);
                    g.brush.SetColor(&rgba(255, 255, 255, 0.9));
                    t.DrawRectangle(
                        &rectf(
                            sel.x as f32 + 0.5,
                            sel.y as f32 + 0.5,
                            sel.w as f32 - 1.0,
                            sel.h as f32 - 1.0,
                        ),
                        &g.brush,
                        1.0,
                        None,
                    );
                    let text = format!("{}\n{}", sel.w, sel.h);
                    self.halo_text(t, g, &text, s);
                }
                (Mode::Area, None) => {
                    if self.crosshair {
                        self.crosshair_lines(t, &g.brush);
                    }
                    // The monitor's own coordinates.
                    let mon = self.monitor_rect();
                    let text = format!("{}\n{}", self.cursor.0 - mon.x, self.cursor.1 - mon.y);
                    self.halo_text(t, g, &text, s);
                }
                (Mode::Window, _) => {
                    let win = self.hovered_window();
                    g.brush.SetColor(&rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.28));
                    t.FillRectangle(&rect(win), &g.brush);
                    g.brush.SetColor(&rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.9));
                    t.DrawRectangle(
                        &rectf(
                            win.x as f32 + 1.0,
                            win.y as f32 + 1.0,
                            win.w as f32 - 2.0,
                            win.h as f32 - 2.0,
                        ),
                        &g.brush,
                        2.0,
                        None,
                    );
                    self.size_pill(t, g, win, s);
                }
            }
            if self.magnifier && self.mode == Mode::Area {
                self.magnifier_box(t, g, s);
            }
            if let Err(e) = t.EndDraw(None, None)
                && e.code() == D2DERR_RECREATE_TARGET
            {
                self.gfx = None;
                self.gfx = make_gfx(self).ok();
            }
        }
    }

    unsafe fn dim_outside(&self, t: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush, sel: Rect) {
        unsafe {
            brush.SetColor(&rgba(0, 0, 0, 0.35));
            let (w, h) = (self.screen.w, self.screen.h);
            for r in [
                Rect::new(0, 0, w, sel.y),
                Rect::new(0, sel.bottom(), w, h - sel.bottom()),
                Rect::new(0, sel.y, sel.x, sel.h),
                Rect::new(sel.right(), sel.y, w - sel.right(), sel.h),
            ] {
                if !r.is_empty() {
                    t.FillRectangle(&rect(r), brush);
                }
            }
        }
    }

    /// A light and a dark line side by side: visible on any background.
    unsafe fn crosshair_lines(&self, t: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush) {
        unsafe {
            let (x, y) = (self.cursor.0 as f32 + 0.5, self.cursor.1 as f32 + 0.5);
            let (w, h) = (self.screen.w as f32, self.screen.h as f32);
            brush.SetColor(&rgba(0, 0, 0, 0.45));
            t.DrawLine(point(x, 0.0), point(x, h), brush, 1.0, None);
            t.DrawLine(point(0.0, y), point(w, y), brush, 1.0, None);
            brush.SetColor(&rgba(255, 255, 255, 0.35));
            t.DrawLine(point(x + 1.0, 0.0), point(x + 1.0, h), brush, 1.0, None);
            t.DrawLine(point(0.0, y + 1.0), point(w, y + 1.0), brush, 1.0, None);
        }
    }

    /// Small dark text with a white halo, just below-right of the cursor.
    unsafe fn halo_text(&self, t: &ID2D1RenderTarget, g: &Gfx, text: &str, s: f32) {
        let x = self.cursor.0 as f32 + 6.0 * s;
        let y = self.cursor.1 as f32 + 4.0 * s;
        let units: Vec<u16> = text.encode_utf16().collect();
        let draw = |dx: f32, dy: f32, color: D2D1_COLOR_F| unsafe {
            g.brush.SetColor(&color);
            t.DrawText(
                &units,
                &g.small,
                &rectf(x + dx, y + dy, 80.0 * s, 40.0 * s),
                &g.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        };
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
            draw(dx, dy, rgba(255, 255, 255, 0.9));
        }
        draw(0.0, 0.0, rgba(20, 20, 20, 1.0));
    }

    unsafe fn size_pill(&self, t: &ID2D1RenderTarget, g: &Gfx, r: Rect, s: f32) {
        unsafe {
            let text = format!("{} × {}", r.w, r.h);
            let units: Vec<u16> = text.encode_utf16().collect();
            // Measured, not estimated: the pill hugs the text it holds.
            let Ok(layout) =
                gfx::dwrite().and_then(|f| f.CreateTextLayout(&units, &g.label, 4000.0, 400.0))
            else {
                return;
            };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            let (pad_x, pad_y) = (10.0 * s, 4.0 * s);
            let (pw, ph) = (m.width + 2.0 * pad_x, m.height + 2.0 * pad_y);
            let x = (r.x as f32 + (r.w as f32 - pw) / 2.0).max(0.0);
            let y = (r.y as f32 + (r.h as f32 - ph) / 2.0).max(0.0);
            g.brush.SetColor(&rgba(20, 20, 20, 0.78));
            t.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rectf(x, y, pw, ph),
                    radiusX: ph / 2.0,
                    radiusY: ph / 2.0,
                },
                &g.brush,
            );
            g.brush.SetColor(&rgba(255, 255, 255, 1.0));
            t.DrawTextLayout(
                point(x + pad_x - m.left, y + pad_y - m.top),
                &layout,
                &g.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    /// A rounded square of enlarged pixels, below-right of the cursor (or
    /// wherever it fits on this monitor), with the centre row and column
    /// marked by translucent bands.
    unsafe fn magnifier_box(&self, t: &ID2D1RenderTarget, g: &Gfx, s: f32) {
        const CELLS: i32 = 11;
        let cell = (9.0 * s).round().max(6.0);
        let size = cell * CELLS as f32;
        // Far enough that the coordinates beside the cursor stay clear of it.
        let gap = 30.0 * s;
        let mon = self.monitor_rect();
        let (cx, cy) = (self.cursor.0 as f32, self.cursor.1 as f32);
        let mut x = cx + gap;
        let mut y = cy + gap;
        if x + size > mon.right() as f32 {
            x = cx - gap - size;
        }
        if y + size > mon.bottom() as f32 {
            y = cy - gap - size;
        }
        let radius = 10.0 * s;
        let boxr = rectf(x, y, size, size);
        let rounded = D2D1_ROUNDED_RECT {
            rect: boxr,
            radiusX: radius,
            radiusY: radius,
        };
        let half = CELLS / 2;
        unsafe {
            // The frozen screen as a brush, scaled so one pixel is one cell
            // and placed so the cursor's pixel is the centre cell: filling
            // the rounded square with it clips the pixels to the rounding.
            // Past the screen's edge the brush repeats the edge pixel.
            let ox = x - (self.cursor.0 - half) as f32 * cell;
            let oy = y - (self.cursor.1 - half) as f32 * cell;
            g.zoom.SetTransform(&Matrix3x2 {
                M11: cell,
                M12: 0.0,
                M21: 0.0,
                M22: cell,
                M31: ox,
                M32: oy,
            });
            t.FillRoundedRectangle(&rounded, &g.zoom);

            g.brush.SetColor(&rgba(0, 0, 0, 0.35));
            let mid = x + half as f32 * cell;
            let midy = y + half as f32 * cell;
            t.FillRectangle(&rectf(mid, y, cell, size), &g.brush);
            t.FillRectangle(&rectf(x, midy, size, cell), &g.brush);
            g.brush.SetColor(&rgba(255, 255, 255, 0.9));
            t.DrawRectangle(&rectf(mid, midy, cell, cell), &g.brush, 1.0, None);
            // A dark edge with a light rim outside it: visible on any background.
            let rim = D2D1_ROUNDED_RECT {
                rect: rectf(x - 1.5 * s, y - 1.5 * s, size + 3.0 * s, size + 3.0 * s),
                radiusX: radius + 1.5 * s,
                radiusY: radius + 1.5 * s,
            };
            g.brush.SetColor(&rgba(255, 255, 255, 0.55));
            t.DrawRoundedRectangle(&rim, &g.brush, 1.0 * s, None);
            g.brush.SetColor(&rgba(40, 40, 40, 0.85));
            t.DrawRoundedRectangle(&rounded, &g.brush, 1.5 * s, None);
        }
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(hwnd, &mut ps);
            }
            with(|o| o.paint());
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SETCURSOR => {
            unsafe {
                SetCursor(LoadCursorW(None, IDC_CROSS).ok());
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let p = lparam_point(lparam.0);
            with(|o| o.cursor = p);
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let p = lparam_point(lparam.0);
            let area = with(|o| {
                o.cursor = p;
                if o.mode == Mode::Area {
                    o.anchor = Some(p);
                }
                o.mode == Mode::Area
            });
            if area == Some(true) {
                unsafe {
                    SetCapture(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let p = lparam_point(lparam.0);
            let outcome = with(|o| {
                o.cursor = p;
                match o.mode {
                    Mode::Window => Some(Some(o.hovered_window())),
                    Mode::Area => {
                        let sel = o.selection();
                        o.anchor = None;
                        // A click that did not drag is not a selection.
                        match sel {
                            Some(r) if r.w > 2 && r.h > 2 => Some(Some(r)),
                            _ => None,
                        }
                    }
                }
            })
            .flatten();
            unsafe {
                let _ = ReleaseCapture();
            }
            match outcome {
                Some(sel) => finish(sel),
                None => invalidate(hwnd),
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            // First a drag in progress is dropped; then the overlay.
            let dragging = with(|o| o.anchor.take().is_some()).unwrap_or(false);
            if dragging {
                unsafe {
                    let _ = ReleaseCapture();
                }
                invalidate(hwnd);
            } else {
                finish(None);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u16;
            if key == VK_ESCAPE.0 {
                finish(None);
            } else if key == VK_SPACE.0 {
                with(|o| {
                    o.mode = if o.mode == Mode::Area {
                        Mode::Window
                    } else {
                        Mode::Area
                    };
                    o.anchor = None;
                });
                invalidate(hwnd);
            } else if key == VK_RETURN.0 {
                let sel = with(|o| match o.mode {
                    Mode::Window => Some(o.hovered_window()),
                    Mode::Area => o.selection(),
                })
                .flatten();
                if sel.is_some() {
                    finish(sel);
                }
            } else if [VK_LEFT.0, VK_RIGHT.0, VK_UP.0, VK_DOWN.0].contains(&key) {
                // Arrows move the pointer a pixel at a time (ten with Shift),
                // for an exact edge.
                let step = if unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0 {
                    10
                } else {
                    1
                };
                let (dx, dy) = match key {
                    k if k == VK_LEFT.0 => (-step, 0),
                    k if k == VK_RIGHT.0 => (step, 0),
                    k if k == VK_UP.0 => (0, -step),
                    _ => (0, step),
                };
                let mut p = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut p);
                    let _ = SetCursorPos(p.x + dx, p.y + dy);
                }
            }
            LRESULT(0)
        }
        // The overlay spans monitors on purpose; a DPI change must not
        // resize it.
        WM_DPICHANGED => LRESULT(0),
        WM_DISPLAYCHANGE => {
            finish(None);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
