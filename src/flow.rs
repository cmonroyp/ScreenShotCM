//! From a command to a finished capture: choose the area (or not), read the
//! pixels, then deliver — clipboard, history, and the save folder when the
//! user asked for it.

use std::cell::Cell;
use std::path::PathBuf;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
    EndPaint, FW_SEMIBOLD, FillRect, HFONT, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, KillTimer,
    LWA_ALPHA, RegisterClassExW, SW_SHOWNOACTIVATE, SetLayeredWindowAttributes, SetTimer,
    ShowWindow, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::capture;
use crate::commands::Command;
use crate::image::{self, Image};
use crate::overlay::{self, Mode};
use crate::settings::Settings;
use crate::util::Rect;
use crate::{agent, clipboard, history, log};

thread_local! {
    static PREVIOUS_AREA: Cell<Option<Rect>> = const { Cell::new(None) };
}

pub fn run(command: Command) {
    if overlay::is_open() || countdown_running() || crate::scroll::is_active() {
        log::info("ignored: a capture is already in progress");
        return;
    }
    let s = agent::settings();
    let result = match command {
        Command::CaptureArea => select(Mode::Area, &s, |rect, frame| {
            finish_from_frame(rect, frame, true)
        }),
        Command::CaptureWindow => select(Mode::Window, &s, |rect, frame| {
            finish_from_frame(rect, frame, false)
        }),
        Command::CaptureFullscreen => {
            let mut p = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut p);
            }
            let monitor = capture::monitor_at(&capture::monitors(), p.x, p.y).rect;
            grab_and_deliver(monitor)
        }
        Command::CapturePreviousArea => match PREVIOUS_AREA.with(Cell::get) {
            Some(r) => grab_and_deliver(r),
            None => select(Mode::Area, &s, |rect, frame| {
                finish_from_frame(rect, frame, true)
            }),
        },
        Command::CaptureScrolling => select(Mode::Area, &s, |rect, _frame| {
            // The page is read live from here on: the frozen frame only
            // served to choose the area.
            if let Some(r) = rect
                && let Err(e) = crate::scroll::start(r)
            {
                log::error(&format!("scrolling capture: {e}"));
                agent::notify(
                    crate::i18n::tr("No se pudo capturar", "Could not capture"),
                    &e,
                    true,
                );
            }
        }),
        Command::SelfTimer => {
            let seconds = s.self_timer_seconds;
            select(Mode::Area, &s, move |rect, _frame| {
                // The point of a timer is a later state of the screen — an
                // open menu, a hover — so the area is read again at the end.
                if let Some(r) = rect {
                    PREVIOUS_AREA.with(|p| p.set(Some(r)));
                    start_countdown(r, seconds);
                }
            })
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        log::error(&format!("{command:?}: {e}"));
        agent::notify(
            crate::i18n::tr("No se pudo capturar", "Could not capture"),
            &e,
            true,
        );
    }
}

fn select(
    mode: Mode,
    s: &Settings,
    then: impl FnOnce(Option<Rect>, Image) + 'static,
) -> Result<(), String> {
    overlay::open(mode, s.show_magnifier, s.show_crosshair, Box::new(then))
}

fn finish_from_frame(rect: Option<Rect>, frame: Image, remember: bool) {
    let Some(r) = rect else { return };
    if remember {
        PREVIOUS_AREA.with(|p| p.set(Some(r)));
    }
    let screen = capture::virtual_screen();
    deliver(frame.crop(r.offset(-screen.x, -screen.y)), centre(r));
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

fn grab_and_deliver(r: Rect) -> Result<(), String> {
    let image = capture::grab(r)?;
    deliver(image, centre(r));
    Ok(())
}

/// The workers still writing a capture's files.
static WORKERS: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> =
    std::sync::Mutex::new(Vec::new());

/// Waits for every capture still being written: the process ends with
/// `exit`, which would otherwise cut a PNG in half — a Quit (or the
/// installer's) just after a capture lost it from the save folder.
pub fn finish_writing() {
    let pending = WORKERS.lock().map(|mut w| std::mem::take(&mut *w));
    for h in pending.unwrap_or_default() {
        let _ = h.join();
    }
}

/// Hands a finished capture to the user: the clipboard (set here, on the UI
/// thread that owns it), the history when it is kept and the save folder
/// when that is the setting (encoded on a worker, so the next capture never
/// waits for this one's PNG), then the quick-access card — or the editor
/// straight away. Nothing else is written. `near` is a point of the screen
/// it was taken on.
pub fn deliver(image: Image, near: (i32, i32)) {
    let s = agent::settings();
    if s.copy_after_capture
        && let Err(e) = clipboard::copy_image(agent::hwnd(), &image)
    {
        log::error(&format!("clipboard: {e}"));
        agent::notify(
            crate::i18n::tr("No se pudo copiar la captura", "Could not copy the capture"),
            &e,
            true,
        );
    }
    let (w, h) = (image.width, image.height);
    let save_to = s.save_after_capture.then(|| s.clone());
    let history_days = s.history_days;
    let hist = s.keep_history.then(history::new_path);
    let for_worker = image.clone();
    let hist_worker = hist.clone();
    let worker = std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        if let Some(hist_worker) = hist_worker {
            if let Some(dir) = hist_worker.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // The card or the editor may have written it meanwhile, or the
            // user deleted it: either way it is not written over or back.
            match image::save_new(&for_worker, &hist_worker) {
                Ok(_) if history::was_forgotten(&hist_worker) => {
                    let _ = std::fs::remove_file(&hist_worker);
                }
                Ok(_) => {}
                Err(e) => log::error(&format!("history {}: {e}", hist_worker.display())),
            }
        }
        if let Some(settings) = save_to
            && let Err(e) = save_to_folder(&for_worker, &settings)
        {
            log::error(&format!("save: {e}"));
        }
        history::prune(history_days);
        unsafe { CoUninitialize() };
    });
    if let Ok(mut w) = WORKERS.lock() {
        w.retain(|h| !h.is_finished());
        w.push(worker);
    }
    log::info(&format!("captured {w}x{h}"));
    if s.open_editor_after_capture {
        crate::editor::open_capture(image, hist);
    } else if s.show_overlay {
        crate::quick::show(image, hist, near);
    }
}

/// A free name in the save folder: "Captura 2026-10-13 a las 12.30.45.png",
/// like a camera roll, with " (2)" and up when it is taken. A suggestion
/// only; `reserve_save_path` holds it.
pub fn save_path(s: &Settings) -> PathBuf {
    candidates(s).find(|p| !p.exists()).unwrap_or_default()
}

fn candidates(s: &Settings) -> impl Iterator<Item = PathBuf> + '_ {
    let t = crate::log::timestamp();
    let (date, time) = t.split_once(' ').unwrap_or((&t, ""));
    let time = time.split('.').next().unwrap_or("").replace(':', ".");
    let base = crate::i18n::trs(
        format!("Captura {date} a las {time}"),
        format!("Screenshot {date} at {time}"),
    );
    let ext = s.image_format.extension();
    (1..).map(move |n| {
        let name = if n == 1 {
            format!("{base}.{ext}")
        } else {
            format!("{base} ({n}).{ext}")
        };
        s.save_dir.join(name)
    })
}

/// Takes a name in the save folder by creating it, so two saves in the same
/// second — two captures, or a capture and a card's Save — never both pick
/// "(no number)" and overwrite each other. The writer replaces the empty
/// file; a writer that fails removes it.
fn reserve_save_path(s: &Settings) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&s.save_dir).map_err(|e| e.to_string())?;
    for path in candidates(s).take(1000) {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Err(crate::i18n::tr("no hay un nombre libre", "no free name").into())
}

/// Saves `image` in the save folder, in the format of the settings, under a
/// reserved name.
pub fn save_to_folder(image: &Image, s: &Settings) -> Result<PathBuf, String> {
    let path = reserve_save_path(s)?;
    match image::save(image, &path, s.image_format, s.jpeg_quality) {
        Ok(()) => Ok(path),
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            Err(e.to_string())
        }
    }
}

// --- Self-timer countdown -------------------------------------------------

const COUNTDOWN_CLASS: PCWSTR = w!("ScreenShotCM.Countdown");
const TICK: usize = 1;

struct Countdown {
    area: Rect,
    left: u32,
    font: HFONT,
}

thread_local! {
    static COUNTDOWN: std::cell::RefCell<Option<Countdown>> = const { std::cell::RefCell::new(None) };
}

fn countdown_running() -> bool {
    COUNTDOWN.with(|c| c.try_borrow().map(|c| c.is_some()).unwrap_or(true))
}

/// A translucent number in the middle of the area, counting down; it is
/// click-through, never activated, and excluded from capture.
fn start_countdown(area: Rect, seconds: u32) {
    let scale = capture::monitor_at(
        &capture::monitors(),
        area.x + area.w / 2,
        area.y + area.h / 2,
    )
    .scale;
    let size = (96.0 * scale) as i32;
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(countdown_proc),
            hInstance: instance.into(),
            lpszClassName: COUNTDOWN_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            COUNTDOWN_CLASS,
            w!(""),
            WS_POPUP,
            area.x + area.w / 2 - size / 2,
            area.y + area.h / 2 - size / 2,
            size,
            size,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            // No countdown window: capture now rather than not at all.
            let _ = grab_and_deliver(area);
            return;
        };
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 215, LWA_ALPHA);
        crate::capture::exclude_from_capture(hwnd);
        let font = crate::util::ui_font((56.0 * scale) as i32, FW_SEMIBOLD);
        COUNTDOWN.with(|c| {
            *c.borrow_mut() = Some(Countdown {
                area,
                left: seconds.max(1),
                font,
            })
        });
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), TICK, 1000, None);
    }
}

extern "system" fn countdown_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            let finished = COUNTDOWN.with(|c| {
                let mut c = c.borrow_mut();
                let cd = c.as_mut()?;
                cd.left -= 1;
                if cd.left == 0 { c.take() } else { None }
            });
            match finished {
                Some(cd) => unsafe {
                    let _ = KillTimer(Some(hwnd), TICK);
                    let _ = DestroyWindow(hwnd);
                    let _ = DeleteObject(cd.font.into());
                    if let Err(e) = grab_and_deliver(cd.area) {
                        agent::notify(
                            crate::i18n::tr("No se pudo capturar", "Could not capture"),
                            &e,
                            true,
                        );
                    }
                },
                None => unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, true);
                },
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let bg = CreateSolidBrush(COLORREF(0x00201C1C));
                FillRect(dc, &rc, bg);
                let _ = DeleteObject(bg.into());
                if let Some((left, font)) =
                    COUNTDOWN.with(|c| c.borrow().as_ref().map(|c| (c.left, c.font)))
                {
                    let old = SelectObject(dc, font.into());
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(dc, COLORREF(0x00FFFFFF));
                    let mut text: Vec<u16> = left.to_string().encode_utf16().collect();
                    DrawTextW(
                        dc,
                        &mut text,
                        &mut rc,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );
                    SelectObject(dc, old);
                }
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_saves_in_the_same_second_never_share_a_name() {
        crate::image::tests::com_for_test();
        let dir = std::env::temp_dir().join(format!("scm-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Settings {
            save_dir: dir.clone(),
            ..Settings::default()
        };
        let img = Image {
            width: 4,
            height: 3,
            pixels: vec![200; 4 * 3 * 4],
        };
        let a = save_to_folder(&img, &s).unwrap();
        let b = save_to_folder(&img, &s).unwrap();
        assert_ne!(a, b);
        assert!(std::fs::metadata(&a).unwrap().len() > 0);
        assert!(std::fs::metadata(&b).unwrap().len() > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
