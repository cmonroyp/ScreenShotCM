//! Pinned captures: a capture floating above every window. Drag to move, wheel to resize, Ctrl+wheel for
//! opacity, arrows to nudge a pixel, Esc or a double click to close. The
//! right-click menu copies, saves, annotates, sets the opacity, locks it
//! (clicks pass through to what is underneath) and closes.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_BRUSH_PROPERTIES, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_EXTEND_MODE_CLAMP, D2D1_ROUNDED_RECT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_DBLCLKS, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, GWL_EXSTYLE, GetCursorPos, GetWindowLongW, GetWindowRect, HTCAPTION, MF_CHECKED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, RegisterClassExW, SW_SHOW, SetForegroundWindow,
    SetWindowLongW, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_CONTEXTMENU,
    WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_MOUSEWHEEL, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCRBUTTONUP,
    WM_RBUTTONUP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_POPUP,
};
use windows::core::{PCWSTR, w};
use windows_numerics::Matrix3x2;

use crate::gfx::{self, rectf, rgba};
use crate::image::Image;
use crate::layered::Surface;
use crate::util::wide;
use crate::{agent, clipboard, log};

const CLASS: PCWSTR = w!("ScreenShotCM.Pin");

struct Pin {
    hwnd: HWND,
    image: Rc<Image>,
    path: Option<PathBuf>,
    /// Display size relative to the capture's own pixels.
    zoom: f32,
    opacity: f32,
    locked: bool,
    s: f32,
    surface: Option<Surface>,
}

thread_local! {
    static PINS: RefCell<Vec<Pin>> = const { RefCell::new(Vec::new()) };
}

fn shadow_margin(s: f32) -> f32 {
    16.0 * s
}

/// Pins `image` where the cursor is, at the capture's own size.
pub fn open(image: Image, path: Option<PathBuf>) {
    if let Err(e) = create(image, path) {
        log::error(&format!("pin: {e}"));
        agent::notify(
            crate::i18n::tr("No se pudo fijar la captura", "Could not pin the capture"),
            &e,
            true,
        );
    }
}

fn create(image: Image, path: Option<PathBuf>) -> Result<(), String> {
    let mut cursor = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut cursor);
    }
    let monitor = crate::capture::monitor_at(&crate::capture::monitors(), cursor.x, cursor.y);
    let s = monitor.scale;
    // Larger than 80 % of the monitor starts scaled down to fit.
    let fit = (monitor.rect.w as f32 * 0.8 / image.width as f32)
        .min(monitor.rect.h as f32 * 0.8 / image.height as f32);
    let zoom = fit.min(1.0);
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: crate::util::arrow_cursor(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            PCWSTR(
                windows::core::HSTRING::from(crate::i18n::tr(
                    "ScreenShotCM — Captura fijada",
                    "ScreenShotCM — Pinned capture",
                ))
                .as_ptr(),
            ),
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
        let pin = Pin {
            hwnd,
            image: Rc::new(image),
            path,
            zoom,
            opacity: 1.0,
            locked: false,
            s,
            surface: None,
        };
        let m = shadow_margin(s) as i32;
        let (w, h) = pin.size();
        let at = (
            (cursor.x - w / 2).clamp(monitor.rect.x - m, monitor.rect.right() - w + m),
            (cursor.y - h / 2).clamp(monitor.rect.y - m, monitor.rect.bottom() - h + m),
        );
        PINS.with(|p| p.borrow_mut().push(pin));
        redraw(hwnd, Some(at));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

impl Pin {
    /// The window, shadow included.
    fn size(&self) -> (i32, i32) {
        let m = 2.0 * shadow_margin(self.s);
        (
            (self.image.width as f32 * self.zoom + m).round() as i32,
            (self.image.height as f32 * self.zoom + m).round() as i32,
        )
    }
}

/// Repaints a pin; `at` moves it, otherwise it keeps its top-left corner.
fn redraw(hwnd: HWND, at: Option<(i32, i32)>) {
    PINS.with(|p| {
        let mut pins = p.borrow_mut();
        let Some(pin) = pins.iter_mut().find(|k| k.hwnd == hwnd) else {
            return;
        };
        let (w, h) = pin.size();
        if pin
            .surface
            .as_ref()
            .is_none_or(|s| s.width != w || s.height != h)
        {
            pin.surface = gfx::d2d().and_then(|f| Surface::new(f, w, h)).ok();
        }
        let Some(surface) = &pin.surface else { return };
        let m = shadow_margin(pin.s);
        let (iw, ih) = (
            pin.image.width as f32 * pin.zoom,
            pin.image.height as f32 * pin.zoom,
        );
        let zoom = pin.zoom;
        let locked = pin.locked;
        let image = pin.image.clone();
        let s = pin.s;
        let _ = surface.draw(|t, brush| unsafe {
            let r = rectf(m, m, iw, ih);
            let radius = 6.0 * s;
            crate::background::shadow(t, brush, r, radius, 0.8, 12.0 * s);
            let Ok(bitmap) = gfx::bitmap(t, &image) else {
                return;
            };
            let interp = if zoom >= 1.0 {
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR
            } else {
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR
            };
            if let Ok(b) = t.CreateBitmapBrush(
                &bitmap,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: interp,
                }),
                None,
            ) {
                b.SetTransform(&Matrix3x2 {
                    M11: zoom,
                    M12: 0.0,
                    M21: 0.0,
                    M22: zoom,
                    M31: m,
                    M32: m,
                });
                t.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: r,
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &b,
                );
            }
            // A locked pin says so with a dashed-blue edge.
            brush.SetColor(&if locked {
                rgba(10, 122, 255, 0.9)
            } else {
                rgba(0, 0, 0, 0.18)
            });
            t.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: r,
                    radiusX: radius,
                    radiusY: radius,
                },
                brush,
                if locked { 2.0 * s } else { 1.0 },
                None,
            );
        });
        let pos = at.unwrap_or_else(|| {
            let mut r = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut r);
            }
            (r.left, r.top)
        });
        let _ = surface.present(hwnd, pos, pin.opacity);
    });
}

fn close(hwnd: HWND) {
    let removed = PINS.with(|p| {
        let mut pins = p.borrow_mut();
        let i = pins.iter().position(|k| k.hwnd == hwnd)?;
        Some(pins.remove(i))
    });
    if removed.is_some() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

pub fn close_all() {
    let all: Vec<HWND> = PINS.with(|p| p.borrow().iter().map(|k| k.hwnd).collect());
    for h in all {
        close(h);
    }
}

/// Clicks reach every locked pin again.
pub fn unlock_all() {
    let all: Vec<HWND> = PINS.with(|p| {
        p.borrow()
            .iter()
            .filter(|k| k.locked)
            .map(|k| k.hwnd)
            .collect()
    });
    for h in all {
        set_locked(h, false);
    }
}

fn set_locked(hwnd: HWND, locked: bool) {
    PINS.with(|p| {
        if let Some(pin) = p.borrow_mut().iter_mut().find(|k| k.hwnd == hwnd) {
            pin.locked = locked;
        }
    });
    unsafe {
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let ex = if locked {
            ex | WS_EX_TRANSPARENT.0
        } else {
            ex & !WS_EX_TRANSPARENT.0
        };
        SetWindowLongW(hwnd, GWL_EXSTYLE, ex as i32);
    }
    redraw(hwnd, None);
}

fn menu(hwnd: HWND) {
    let Some((image, path, opacity, zoom)) = PINS.with(|p| {
        p.borrow()
            .iter()
            .find(|k| k.hwnd == hwnd)
            .map(|k| (k.image.clone(), k.path.clone(), k.opacity, k.zoom))
    }) else {
        return;
    };
    const COPY: usize = 1;
    const SAVE: usize = 2;
    const EDIT: usize = 3;
    const ZOOM_1: usize = 4;
    const LOCK: usize = 5;
    const CLOSE: usize = 6;
    const CLOSE_ALL: usize = 7;
    const OPACITY: usize = 100;
    let chosen = unsafe {
        let Ok(m) = CreatePopupMenu() else { return };
        let add = |menu, flags, id: usize, text: &str| {
            let t = wide(text);
            let _ = AppendMenuW(menu, flags, id, PCWSTR(t.as_ptr()));
        };
        add(m, MF_STRING, COPY, crate::i18n::tr("Copiar", "Copy"));
        add(
            m,
            MF_STRING,
            SAVE,
            crate::i18n::tr(
                "Guardar en la carpeta de capturas",
                "Save to the captures folder",
            ),
        );
        add(m, MF_STRING, EDIT, crate::i18n::tr("Anotar…", "Annotate…"));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        add(
            m,
            if (zoom - 1.0).abs() < 0.001 {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            },
            ZOOM_1,
            crate::i18n::tr("Tamaño real (100 %)", "Actual size (100 %)"),
        );
        if let Ok(sub) = CreatePopupMenu() {
            for pct in [100usize, 80, 60, 40, 20] {
                let checked = ((opacity * 100.0).round() as usize) == pct;
                add(
                    sub,
                    if checked {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    },
                    OPACITY + pct,
                    &format!("{pct} %"),
                );
            }
            let t = wide(crate::i18n::tr("Opacidad", "Opacity"));
            let _ = AppendMenuW(m, MF_POPUP, sub.0 as usize, PCWSTR(t.as_ptr()));
        }
        add(
            m,
            MF_STRING,
            LOCK,
            crate::i18n::tr(
                "Bloquear (los clics la atraviesan)",
                "Lock (clicks pass through)",
            ),
        );
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        add(
            m,
            MF_STRING,
            CLOSE,
            crate::i18n::tr("Cerrar\tEsc", "Close\tEsc"),
        );
        add(
            m,
            MF_STRING,
            CLOSE_ALL,
            crate::i18n::tr("Cerrar todas las fijadas", "Close all pinned"),
        );
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        let _ = SetForegroundWindow(hwnd);
        let r = TrackPopupMenuEx(
            m,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            at.x,
            at.y,
            hwnd,
            None,
        );
        let _ = DestroyMenu(m);
        r.0 as usize
    };
    match chosen {
        COPY => {
            if let Err(e) = clipboard::copy_image(agent::hwnd(), &image) {
                agent::notify(
                    crate::i18n::tr("No se pudo copiar", "Could not copy"),
                    &e,
                    true,
                );
            }
        }
        SAVE => {
            if let Err(e) = crate::flow::save_to_folder(&image, &agent::settings()) {
                agent::notify(
                    crate::i18n::tr("No se pudo guardar", "Could not save"),
                    &e,
                    true,
                );
            }
        }
        EDIT => crate::editor::open((*image).clone(), path),
        ZOOM_1 => {
            PINS.with(|p| {
                if let Some(pin) = p.borrow_mut().iter_mut().find(|k| k.hwnd == hwnd) {
                    pin.zoom = 1.0;
                }
            });
            redraw(hwnd, None);
        }
        LOCK => set_locked(hwnd, true),
        CLOSE => close(hwnd),
        CLOSE_ALL => close_all(),
        n if n > OPACITY => {
            PINS.with(|p| {
                if let Some(pin) = p.borrow_mut().iter_mut().find(|k| k.hwnd == hwnd) {
                    pin.opacity = (n - OPACITY) as f32 / 100.0;
                }
            });
            redraw(hwnd, None);
        }
        _ => {}
    }
}

fn adjust(hwnd: HWND, f: impl FnOnce(&mut Pin)) {
    PINS.with(|p| {
        if let Some(pin) = p.borrow_mut().iter_mut().find(|k| k.hwnd == hwnd) {
            f(pin);
        }
    });
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // The whole pin moves the window, as a title bar would.
            WM_NCHITTEST => LRESULT(HTCAPTION as isize),
            WM_NCLBUTTONDBLCLK | WM_LBUTTONDBLCLK => {
                close(hwnd);
                LRESULT(0)
            }
            WM_NCRBUTTONUP | WM_RBUTTONUP | WM_CONTEXTMENU => {
                menu(hwnd);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
                let up = delta > 0;
                if GetKeyState(VK_CONTROL.0 as i32) < 0 {
                    adjust(hwnd, |p| {
                        p.opacity = (p.opacity + if up { 0.1 } else { -0.1 }).clamp(0.2, 1.0)
                    });
                    redraw(hwnd, None);
                } else {
                    // Resize about the window's centre.
                    let mut r = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut r);
                    let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
                    let mut size = (0, 0);
                    adjust(hwnd, |p| {
                        // As small as 24 px on its short side: a tall
                        // scrolling capture starts below 10 %, and a fixed
                        // 10 % floor made the wheel *grow* it.
                        let short = p.image.width.min(p.image.height).max(1) as f32;
                        let floor = (24.0 / short).min(1.0);
                        p.zoom = (p.zoom * if up { 1.1 } else { 1.0 / 1.1 })
                            .clamp(floor.min(p.zoom), 5.0);
                        size = p.size();
                    });
                    redraw(hwnd, Some((cx - size.0 / 2, cy - size.1 / 2)));
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let key = wparam.0 as u16;
                if key == VK_ESCAPE.0 {
                    close(hwnd);
                } else if [VK_LEFT.0, VK_RIGHT.0, VK_UP.0, VK_DOWN.0].contains(&key) {
                    let step = if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                        10
                    } else {
                        1
                    };
                    let mut r = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut r);
                    let (dx, dy) = match key {
                        k if k == VK_LEFT.0 => (-step, 0),
                        k if k == VK_RIGHT.0 => (step, 0),
                        k if k == VK_UP.0 => (0, -step),
                        _ => (0, step),
                    };
                    redraw(hwnd, Some((r.left + dx, r.top + dy)));
                } else if key == 0x43 && GetKeyState(VK_CONTROL.0 as i32) < 0 {
                    let image = PINS.with(|p| {
                        p.borrow()
                            .iter()
                            .find(|k| k.hwnd == hwnd)
                            .map(|k| k.image.clone())
                    });
                    if let Some(img) = image {
                        let _ = clipboard::copy_image(agent::hwnd(), &img);
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
