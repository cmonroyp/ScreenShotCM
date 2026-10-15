//! Capture history: a grid of the last month's
//! captures, newest first. Hovering a card offers copy, annotate, pin and
//! delete; a double click opens the editor. Thumbnails are decoded on a
//! worker thread and arrive as they are ready.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_ELLIPSE, D2D1_ROUNDED_RECT, ID2D1Bitmap,
    ID2D1Factory, ID2D1HwndRenderTarget, ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, IDNO,
    IsIconic, MB_ICONWARNING, MB_YESNO, MessageBoxW, PostMessageW, RegisterClassExW, SW_RESTORE,
    SW_SHOW, SetForegroundWindow, ShowWindow, WM_APP, WM_CLOSE, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_PAINT, WM_SIZE, WNDCLASSEXW, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCWSTR, w};

use crate::gfx::{self, point, rectf, rgba};
use crate::icons::{Icon, Icons};
use crate::image::{self, Image};
use crate::theme::{self, Palette};
use crate::util::lparam_point;
use crate::{agent, clipboard, log, paths};

const CLASS: PCWSTR = w!("ScreenShotCM.History");
const WM_THUMB: u32 = WM_APP + 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Copy,
    Edit,
    Pin,
    Delete,
}

struct Entry {
    path: PathBuf,
    label: String,
}

struct Gpu {
    factory: ID2D1Factory,
    target: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    dwrite: IDWriteFactory,
    title: IDWriteTextFormat,
    small: IDWriteTextFormat,
    bitmaps: HashMap<PathBuf, ID2D1Bitmap>,
}

struct History {
    hwnd: HWND,
    pal: Palette,
    s: f32,
    entries: Vec<Entry>,
    thumbs: HashMap<PathBuf, Image>,
    incoming: Receiver<(PathBuf, Image)>,
    scroll: f32,
    hover: Option<usize>,
    hot: Option<Btn>,
    /// The button pressed, which acts only if released over itself.
    pressed: Option<(usize, Btn)>,
    gpu: Option<Gpu>,
}

thread_local! {
    static WINDOW: RefCell<Option<History>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut History) -> R) -> Option<R> {
    WINDOW.with(|w| w.try_borrow_mut().ok().and_then(|mut w| w.as_mut().map(f)))
}

/// Opens the history, or brings the open one forward.
pub fn open() {
    if let Some(hwnd) = with(|h| h.hwnd) {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        reload();
        return;
    }
    if let Err(e) = create() {
        log::error(&format!("history window: {e}"));
        agent::notify(
            crate::i18n::tr(
                "No se pudo abrir el historial",
                "Could not open the history",
            ),
            &e,
            true,
        );
    }
}

/// The captures in the history folder, newest first.
fn scan() -> Vec<Entry> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(paths::history_dir())
        .map(|d| {
            d.flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x == "png" || x == "jpg")
                })
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files
        .into_iter()
        .map(|(_, path)| {
            // Names are "2026-10-13 12.30.45.123.png": show "13/10 12:30".
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let label = match stem.split_once(' ') {
                Some((date, time)) => {
                    let d: Vec<&str> = date.split('-').collect();
                    let t: Vec<&str> = time.split('.').collect();
                    if d.len() == 3 && t.len() >= 2 {
                        format!("{}/{} {}:{}", d[2], d[1], t[0], t[1])
                    } else {
                        stem.clone()
                    }
                }
                None => stem.clone(),
            };
            Entry { path, label }
        })
        .collect()
}

fn start_loader(hwnd: HWND, paths: Vec<PathBuf>, size: (u32, u32)) -> Receiver<(PathBuf, Image)> {
    let (tx, rx) = channel();
    let target = hwnd.0 as isize;
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        for p in paths {
            if let Ok(img) = image::load_thumbnail(&p, size.0, size.1) {
                if tx.send((p, img)).is_err() {
                    break;
                }
                unsafe {
                    let _ =
                        PostMessageW(Some(HWND(target as *mut _)), WM_THUMB, WPARAM(0), LPARAM(0));
                }
            }
        }
        unsafe { CoUninitialize() };
    });
    rx
}

fn create() -> Result<(), String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hIcon: crate::icon::class_icon(),
            hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                None,
                windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
            )
            .unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            CLASS,
            PCWSTR(
                windows::core::HSTRING::from(crate::i18n::tr(
                    "Historial de capturas — ScreenShotCM",
                    "Capture history — ScreenShotCM",
                ))
                .as_ptr(),
            ),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000,
            700,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        crate::theme::frame_window(hwnd);
        let s = GetDpiForWindow(hwnd) as f32 / 96.0;
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
            hwnd,
            None,
            0,
            0,
            (1000.0 * s) as i32,
            (700.0 * s) as i32,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
        );
        let entries = scan();
        let incoming = start_loader(
            hwnd,
            entries.iter().map(|e| e.path.clone()).collect(),
            thumb_size(s),
        );
        WINDOW.with(|w| {
            *w.borrow_mut() = Some(History {
                hwnd,
                pal: theme::current(),
                s,
                entries,
                thumbs: HashMap::new(),
                incoming,
                scroll: 0.0,
                hover: None,
                hot: None,
                pressed: None,
                gpu: None,
            })
        });
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

fn thumb_size(s: f32) -> (u32, u32) {
    ((216.0 * s) as u32, (136.0 * s) as u32)
}

/// Rescans the folder: a capture taken while the window is open appears.
fn reload() {
    with(|h| {
        let entries = scan();
        let missing: Vec<PathBuf> = entries
            .iter()
            .filter(|e| !h.thumbs.contains_key(&e.path))
            .map(|e| e.path.clone())
            .collect();
        h.entries = entries;
        if !missing.is_empty() {
            h.incoming = start_loader(h.hwnd, missing, thumb_size(h.s));
        }
        invalidate(h.hwnd);
    });
}

impl History {
    fn client(&self) -> (f32, f32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        (rc.right as f32, rc.bottom as f32)
    }

    fn header_h(&self) -> f32 {
        64.0 * self.s
    }

    /// Card size and column count for the current width.
    fn grid(&self) -> (f32, f32, usize, f32) {
        let (w, _) = self.client();
        let (cw, ch) = (232.0 * self.s, 180.0 * self.s);
        let gap = 16.0 * self.s;
        let cols = (((w - gap) / (cw + gap)).floor() as usize).max(1);
        let left = ((w - cols as f32 * (cw + gap) + gap) / 2.0).max(gap);
        (cw, ch, cols, left)
    }

    fn card_rect(&self, i: usize) -> (f32, f32, f32, f32) {
        let (cw, ch, cols, left) = self.grid();
        let gap = 16.0 * self.s;
        let (col, row) = ((i % cols) as f32, (i / cols) as f32);
        (
            left + col * (cw + gap),
            self.header_h() + gap + row * (ch + gap) - self.scroll,
            cw,
            ch,
        )
    }

    fn content_h(&self) -> f32 {
        let (_, ch, cols, _) = self.grid();
        let rows = self.entries.len().div_ceil(cols) as f32;
        self.header_h() + 16.0 * self.s + rows * (ch + 16.0 * self.s)
    }

    fn buttons(&self, i: usize) -> Vec<(Btn, f32, f32, f32)> {
        let (x, y, cw, _) = self.card_rect(i);
        let d = 30.0 * self.s;
        let gap = 8.0 * self.s;
        let total = 4.0 * d + 3.0 * gap;
        let start = x + (cw - total) / 2.0;
        let cy = y + 12.0 * self.s + thumb_size(self.s).1 as f32 / 2.0;
        [Btn::Copy, Btn::Edit, Btn::Pin, Btn::Delete]
            .iter()
            .enumerate()
            .map(|(k, b)| (*b, start + k as f32 * (d + gap) + d / 2.0, cy, d / 2.0))
            .collect()
    }

    fn hit(&self, p: (f32, f32)) -> (Option<usize>, Option<Btn>) {
        if p.1 < self.header_h() {
            return (None, None);
        }
        for i in 0..self.entries.len() {
            let (x, y, w, h) = self.card_rect(i);
            if p.0 >= x && p.0 < x + w && p.1 >= y && p.1 < y + h {
                let btn = self
                    .buttons(i)
                    .into_iter()
                    .find(|(_, cx, cy, r)| (p.0 - cx).powi(2) + (p.1 - cy).powi(2) <= r * r)
                    .map(|b| b.0);
                return (Some(i), btn);
            }
        }
        (None, None)
    }

    fn ensure_gpu(&mut self) -> bool {
        if self.gpu.is_some() {
            return true;
        }
        let (w, h) = self.client();
        let s = self.s;
        let make = || -> windows::core::Result<Gpu> {
            let factory = gfx::d2d()?;
            let target =
                gfx::window_target(&factory, self.hwnd, w.max(1.0) as u32, h.max(1.0) as u32)?;
            let brush = unsafe { target.CreateSolidColorBrush(&rgba(0, 0, 0, 1.0), None)? };
            let round = gfx::round_stroke(&factory, false)?;
            let title = gfx::text_format(20.0 * s, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let small = gfx::text_format(12.5 * s, DWRITE_FONT_WEIGHT_MEDIUM)?;
            unsafe {
                let _ = title.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                let _ = small.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            }
            Ok(Gpu {
                dwrite: gfx::dwrite()?,
                factory,
                target,
                brush,
                round,
                title,
                small,
                bitmaps: HashMap::new(),
            })
        };
        match make() {
            Ok(g) => {
                self.gpu = Some(g);
                true
            }
            Err(e) => {
                log::error(&format!("history surface: {e}"));
                false
            }
        }
    }

    fn paint(&mut self) {
        while let Ok((p, img)) = self.incoming.try_recv() {
            self.thumbs.insert(p, img);
        }
        if !self.ensure_gpu() {
            return;
        }
        // Upload thumbnails that arrived since the last frame.
        {
            let g = self.gpu.as_mut().unwrap();
            for (p, img) in &self.thumbs {
                if !g.bitmaps.contains_key(p)
                    && let Ok(b) = gfx::bitmap(&g.target, img)
                {
                    g.bitmaps.insert(p.clone(), b);
                }
            }
        }
        let g = self.gpu.as_ref().unwrap();
        let t: &ID2D1RenderTarget = &g.target;
        let p = &self.pal;
        let s = self.s;
        let (w, h) = self.client();
        let icons = Icons {
            factory: &g.factory,
            dwrite: &g.dwrite,
            round: &g.round,
        };
        unsafe {
            t.BeginDraw();
            t.Clear(Some(&p.canvas));
            for (i, e) in self.entries.iter().enumerate() {
                let (x, y, cw, ch) = self.card_rect(i);
                if y + ch < self.header_h() || y > h {
                    continue;
                }
                let card = rectf(x, y, cw, ch);
                let hover = self.hover == Some(i);
                g.brush.SetColor(&p.chrome);
                t.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: card,
                        radiusX: 12.0 * s,
                        radiusY: 12.0 * s,
                    },
                    &g.brush,
                );
                g.brush
                    .SetColor(if hover { &p.accent } else { &p.chrome_line });
                t.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: card,
                        radiusX: 12.0 * s,
                        radiusY: 12.0 * s,
                    },
                    &g.brush,
                    if hover { 2.0 * s } else { 1.0 },
                    None,
                );
                let (tw, th) = thumb_size(s);
                let area = rectf(x + 8.0 * s, y + 8.0 * s, cw - 16.0 * s, th as f32 + 8.0 * s);
                g.brush.SetColor(&p.canvas);
                t.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: area,
                        radiusX: 8.0 * s,
                        radiusY: 8.0 * s,
                    },
                    &g.brush,
                );
                if let (Some(b), Some(img)) = (g.bitmaps.get(&e.path), self.thumbs.get(&e.path)) {
                    let (iw, ih) = (img.width as f32, img.height as f32);
                    let k = ((area.right - area.left - 8.0 * s) / iw)
                        .min((area.bottom - area.top - 8.0 * s) / ih)
                        .min(1.0);
                    let (dw, dh) = (iw * k, ih * k);
                    let dx = area.left + (area.right - area.left - dw) / 2.0;
                    let dy = area.top + (area.bottom - area.top - dh) / 2.0;
                    t.DrawBitmap(
                        b,
                        Some(&rectf(dx, dy, dw, dh)),
                        1.0,
                        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                        None,
                    );
                }
                let _ = tw;
                let label: Vec<u16> = e.label.encode_utf16().collect();
                let _ = g.small.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
                g.brush.SetColor(&p.text_dim);
                crate::gfx::draw_text(
                    t,
                    &g.brush,
                    &g.small,
                    &label,
                    &rectf(
                        x + 12.0 * s,
                        area.bottom + 4.0 * s,
                        cw - 24.0 * s,
                        ch - (area.bottom - y) - 8.0 * s,
                    ),
                );

                if hover {
                    g.brush.SetColor(&rgba(0, 0, 0, 0.35));
                    t.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: area,
                            radiusX: 8.0 * s,
                            radiusY: 8.0 * s,
                        },
                        &g.brush,
                    );
                    for (btn, cx, cy, r) in self.buttons(i) {
                        let e = D2D1_ELLIPSE {
                            point: point(cx, cy),
                            radiusX: r,
                            radiusY: r,
                        };
                        g.brush.SetColor(&if self.hot == Some(btn) {
                            rgba(255, 255, 255, 1.0)
                        } else {
                            rgba(255, 255, 255, 0.88)
                        });
                        t.FillEllipse(&e, &g.brush);
                        let icon = match btn {
                            Btn::Copy => Icon::Copy,
                            Btn::Edit => Icon::Pen,
                            Btn::Pin => Icon::Pin,
                            Btn::Delete => Icon::Trash,
                        };
                        let color = if btn == Btn::Delete && self.hot == Some(btn) {
                            rgba(220, 38, 38, 1.0)
                        } else {
                            rgba(20, 20, 22, 1.0)
                        };
                        icons.draw(t, &g.brush, icon, cx, cy, 15.0 * s, color);
                    }
                }
            }

            // Header, over the scrolled grid.
            g.brush.SetColor(&p.chrome);
            t.FillRectangle(&rectf(0.0, 0.0, w, self.header_h()), &g.brush);
            g.brush.SetColor(&p.chrome_line);
            t.FillRectangle(&rectf(0.0, self.header_h() - 1.0, w, 1.0), &g.brush);
            let title: Vec<u16> = crate::i18n::tr("Historial", "History")
                .encode_utf16()
                .collect();
            g.brush.SetColor(&p.text);
            crate::gfx::draw_text(
                t,
                &g.brush,
                &g.title,
                &title,
                &rectf(24.0 * s, 0.0, 300.0 * s, self.header_h()),
            );
            let days = agent::settings().history_days;
            let sub = match self.entries.len() {
                0 => crate::i18n::tr("Aún no hay capturas", "No captures yet").to_string(),
                1 => crate::i18n::trs(
                    format!("1 captura · se guardan {days} días"),
                    format!("1 capture · kept {days} days"),
                ),
                n => crate::i18n::trs(
                    format!("{n} capturas · se guardan {days} días"),
                    format!("{n} captures · kept {days} days"),
                ),
            };
            let units: Vec<u16> = sub.encode_utf16().collect();
            let _ = g.small.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            g.brush.SetColor(&p.text_dim);
            crate::gfx::draw_text(
                t,
                &g.brush,
                &g.small,
                &units,
                &rectf(130.0 * s, 0.0, 400.0 * s, self.header_h()),
            );
            if self.entries.is_empty() {
                let units: Vec<u16> = crate::i18n::tr(
                    "Las capturas aparecerán aquí.",
                    "Your captures will appear here.",
                )
                .encode_utf16()
                .collect();
                let _ = g.small.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                crate::gfx::draw_text(
                    t,
                    &g.brush,
                    &g.small,
                    &units,
                    &rectf(0.0, self.header_h(), w, h - self.header_h()),
                );
            }
            if let Err(e) = t.EndDraw(None, None)
                && e.code() == D2DERR_RECREATE_TARGET
            {
                self.gpu = None;
            }
        }
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn act(i: usize, btn: Btn) {
    let Some((hwnd, path)) = with(|h| (h.hwnd, h.entries.get(i).map(|e| e.path.clone()))) else {
        return;
    };
    let Some(path) = path else { return };
    match btn {
        Btn::Delete => {
            let answer = unsafe {
                MessageBoxW(
                    Some(hwnd),
                    PCWSTR(
                        windows::core::HSTRING::from(crate::i18n::tr(
                            "¿Eliminar esta captura del historial?",
                            "Delete this capture from the history?",
                        ))
                        .as_ptr(),
                    ),
                    w!("ScreenShotCM"),
                    MB_YESNO | MB_ICONWARNING,
                )
            };
            if answer == IDNO {
                return;
            }
            // An entry leaves the list only if its file really went: it
            // came back on the next opening otherwise.
            if let Err(e) = crate::history::forget(&path) {
                crate::agent::notify(
                    crate::i18n::tr("No se pudo eliminar", "Could not delete"),
                    &format!("{}: {e}", path.display()),
                    true,
                );
                return;
            }
            with(|h| {
                h.entries.retain(|e| e.path != path);
                h.thumbs.remove(&path);
                if let Some(g) = &mut h.gpu {
                    g.bitmaps.remove(&path);
                }
                h.hover = None;
            });
            invalidate(hwnd);
        }
        _ => match image::load(&path) {
            Ok(img) => match btn {
                Btn::Copy => {
                    if let Err(e) = clipboard::copy_image(agent::hwnd(), &img) {
                        agent::notify(
                            crate::i18n::tr("No se pudo copiar", "Could not copy"),
                            &e,
                            true,
                        );
                    }
                }
                Btn::Edit => crate::editor::open(img, Some(path)),
                Btn::Pin => crate::pin::open(img, Some(path)),
                Btn::Delete => unreachable!(),
            },
            Err(e) => agent::notify(
                crate::i18n::tr("No se pudo abrir la captura", "Could not open the capture"),
                &e.to_string(),
                true,
            ),
        },
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                with(|h| h.paint());
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_THUMB => {
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_SIZE => {
                with(|h| {
                    let (w, ht) = h.client();
                    if let Some(g) = &h.gpu {
                        let _ = g.target.Resize(
                            &windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U {
                                width: w as u32,
                                height: ht as u32,
                            },
                        );
                    }
                    let max = (h.content_h() - ht).max(0.0);
                    h.scroll = h.scroll.min(max);
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let r = &*(lparam.0 as *const RECT);
                with(|h| {
                    h.s = (wparam.0 & 0xFFFF) as f32 / 96.0;
                    h.gpu = None;
                });
                let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                );
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_point(lparam.0);
                let changed = with(|h| {
                    let (i, b) = h.hit((x as f32, y as f32));
                    let changed = (i, b) != (h.hover, h.hot);
                    h.hover = i;
                    h.hot = b;
                    changed
                });
                crate::util::track_leave(hwnd);
                if changed == Some(true) {
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with(|h| {
                    h.hover = None;
                    h.hot = None;
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
                with(|h| {
                    let (_, ht) = h.client();
                    let max = (h.content_h() - ht).max(0.0);
                    h.scroll = (h.scroll - delta as f32 / 120.0 * 80.0 * h.s).clamp(0.0, max);
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lparam_point(lparam.0);
                with(|h| {
                    h.pressed = match h.hit((x as f32, y as f32)) {
                        (Some(i), Some(b)) => Some((i, b)),
                        _ => None,
                    }
                });
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = lparam_point(lparam.0);
                let _ = ReleaseCapture();
                // A button acts when pressed and released on itself, like
                // every other button in the app.
                let hit = with(|h| {
                    let pressed = h.pressed.take()?;
                    match h.hit((x as f32, y as f32)) {
                        (Some(i), Some(b)) if (i, b) == pressed => Some(pressed),
                        _ => None,
                    }
                })
                .flatten();
                if let Some((i, b)) = hit {
                    act(i, b);
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = lparam_point(lparam.0);
                if let Some((Some(i), None)) = with(|h| h.hit((x as f32, y as f32))) {
                    act(i, Btn::Edit);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                WINDOW.with(|w| w.borrow_mut().take());
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
