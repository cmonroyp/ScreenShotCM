//! The settings window: native, themed controls (Common Controls 6 through
//! the manifest), laid out in sections. A
//! shortcut field records the keys pressed into it — PrintScreen included,
//! which is why the global hotkeys are released while the window is open.

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, COLOR_GRAYTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, CreateFontW,
    CreateSolidBrush, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
    FW_NORMAL, FW_SEMIBOLD, FillRect, FrameRect, GetSysColor, GetSysColorBrush, HDC, HFONT,
    InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VK_BACK, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN,
    VK_SHIFT, VK_SNAPSHOT, VK_TAB,
};
use windows::Win32::UI::Shell::{
    FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_DEFPUSHBUTTON, BS_PUSHBUTTON,
    CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBS_DROPDOWNLIST, CW_USEDEFAULT, CreateWindowExW,
    DLGC_WANTALLKEYS, DefWindowProcW, DestroyWindow, ES_AUTOHSCROLL, ES_NUMBER, ES_READONLY,
    GetWindowTextW, HMENU, IDC_ARROW, IsIconic, LoadCursorW, MB_ICONWARNING, MB_OK, MessageBoxW,
    RegisterClassExW, SW_RESTORE, SW_SHOW, SWP_NOZORDER, SendMessageW, SetForegroundWindow,
    SetWindowPos, SetWindowTextW, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DESTROY, WM_ERASEBKGND, WM_GETDLGCODE, WM_KEYDOWN,
    WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_PAINT, WM_SETFOCUS, WM_SETFONT, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_APPWINDOW, WS_EX_CLIENTEDGE,
    WS_GROUP, WS_MINIMIZEBOX, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{PCWSTR, w};

use windows::Win32::UI::Controls::BST_CHECKED;

use crate::settings::{ImageFormat, Settings};
use crate::util::wide;
use crate::{agent, autostart, hotkeys, log};

const CLASS: PCWSTR = w!("ScreenShotCM.Settings");
const HOTKEY_CLASS: PCWSTR = w!("ScreenShotCM.HotkeyField");

// Control ids.
const ID_AUTOSTART: i32 = 101;
const ID_COPY: i32 = 102;
const ID_OVERLAY: i32 = 103;
const ID_EDITOR: i32 = 104;
const ID_SAVE: i32 = 105;
const ID_CLOSE_AFTER: i32 = 106;
const ID_FOLDER: i32 = 107;
const ID_FOLDER_PICK: i32 = 108;
const ID_PNG: i32 = 109;
const ID_JPEG: i32 = 110;
const ID_QUALITY: i32 = 111;
const ID_HISTORY_DAYS: i32 = 112;
const ID_HISTORY_OPEN: i32 = 113;
const ID_MAGNIFIER: i32 = 114;
const ID_CROSSHAIR: i32 = 115;
const ID_TIMER: i32 = 116;
const ID_LANGUAGE: i32 = 117;
const ID_HK_AREA: i32 = 120;
const ID_HK_WINDOW: i32 = 121;
const ID_HK_FULL: i32 = 122;
const ID_HK_PREVIOUS: i32 = 123;
const ID_RESET: i32 = 130;
const ID_CANCEL: i32 = 131;
const ID_OK: i32 = 132;

const CLOSE_CHOICES: [(u32, &str, &str); 6] = [
    (0, "Nunca", "Never"),
    (5, "A los 5 segundos", "After 5 seconds"),
    (10, "A los 10 segundos", "After 10 seconds"),
    (20, "A los 20 segundos", "After 20 seconds"),
    (30, "A los 30 segundos", "After 30 seconds"),
    (60, "Al minuto", "After a minute"),
];
const HISTORY_CHOICES: [(u32, &str, &str); 5] = [
    (7, "1 semana", "1 week"),
    (14, "2 semanas", "2 weeks"),
    (30, "1 mes", "1 month"),
    (90, "3 meses", "3 months"),
    (0, "Siempre", "Forever"),
];
const LANGUAGE_CHOICES: [(crate::settings::Language, &str, &str); 3] = [
    (
        crate::settings::Language::Auto,
        "Automático (el de Windows)",
        "Automatic (Windows' own)",
    ),
    (crate::settings::Language::Spanish, "Español", "Español"),
    (crate::settings::Language::English, "English", "English"),
];
const TIMER_CHOICES: [u32; 4] = [3, 5, 10, 15];

struct Window {
    hwnd: HWND,
    font: HFONT,
    bold: HFONT,
    s: f32,
    /// Section headings: text and top in client pixels.
    headings: Vec<(String, i32, i32)>,
}

thread_local! {
    static WINDOW: RefCell<Option<Window>> = const { RefCell::new(None) };
}

pub fn open() {
    if let Some(hwnd) = WINDOW.with(|w| w.borrow().as_ref().map(|w| w.hwnd)) {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    if let Err(e) = create() {
        log::error(&format!("settings window: {e}"));
        agent::notify(
            crate::i18n::tr(
                "No se pudieron abrir los ajustes",
                "Could not open the settings",
            ),
            &e,
            true,
        );
    }
}

fn create() -> Result<(), String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            hIcon: crate::icon::create(32).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let field = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(hotkey_proc),
            hInstance: instance.into(),
            lpszClassName: HOTKEY_CLASS,
            hCursor: LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_IBEAM)
                .unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&field);

        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            CLASS,
            PCWSTR(
                windows::core::HSTRING::from(crate::i18n::tr(
                    "Ajustes — ScreenShotCM",
                    "Settings — ScreenShotCM",
                ))
                .as_ptr(),
            ),
            WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            100,
            100,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        let s = GetDpiForWindow(hwnd) as f32 / 96.0;
        let px = |v: f32| (v * s).round() as i32;
        let font = CreateFontW(
            -px(15.0),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let bold = CreateFontW(
            -px(15.0),
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );

        // While the window is open, keys go to the shortcut fields.
        hotkeys::unregister_all(agent::hwnd());

        let settings = agent::settings();
        let mut headings = Vec::new();
        let mut left = px(24.0);
        let width = px(520.0);
        let row = px(30.0);
        let mut y = px(16.0);
        let mut heading = |text: &str, x: i32, y: &mut i32| {
            headings.push((text.to_string(), x, *y));
            *y += px(30.0);
        };
        let ctl = |class: PCWSTR,
                   text: &str,
                   style: u32,
                   x: i32,
                   y: i32,
                   w: i32,
                   h: i32,
                   id: i32|
         -> HWND {
            let t = wide(text);
            let h = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR(t.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | style),
                x,
                y,
                w,
                h,
                Some(hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(instance.into()),
                None,
            )
            .unwrap_or_default();
            SendMessageW(
                h,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            h
        };
        let check = |text: &str, x: i32, y: i32, w: i32, id: i32, on: bool| {
            let h = ctl(
                w!("BUTTON"),
                text,
                BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
                x,
                y,
                w,
                px(24.0),
                id,
            );
            SendMessageW(
                h,
                BM_SETCHECK,
                Some(WPARAM(if on { BST_CHECKED.0 as usize } else { 0 })),
                None,
            );
        };
        let label = |text: &str, x: i32, y: i32, w: i32| {
            ctl(w!("STATIC"), text, 0, x, y + px(3.0), w, px(22.0), 0);
        };
        let combo = |x: i32, y: i32, w: i32, id: i32, items: &[&str], selected: usize| {
            let h = ctl(
                w!("COMBOBOX"),
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP.0 | WS_VSCROLL.0,
                x,
                y,
                w,
                px(240.0),
                id,
            );
            for item in items {
                let t = wide(item);
                SendMessageW(h, CB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
            }
            SendMessageW(h, CB_SETCURSEL, Some(WPARAM(selected)), None);
        };

        heading(crate::i18n::tr("General", "General"), left, &mut y);
        check(
            crate::i18n::tr(
                "Iniciar ScreenShotCM con Windows",
                "Start ScreenShotCM with Windows",
            ),
            left,
            y,
            width,
            ID_AUTOSTART,
            autostart::is_enabled(),
        );
        y += row;
        label(crate::i18n::tr("Idioma:", "Language:"), left, y, px(170.0));
        let language_idx = LANGUAGE_CHOICES
            .iter()
            .position(|c| c.0 == settings.language)
            .unwrap_or(0);
        combo(
            left + px(180.0),
            y,
            px(260.0),
            ID_LANGUAGE,
            &LANGUAGE_CHOICES.map(|c| crate::i18n::tr(c.1, c.2)),
            language_idx,
        );
        y += row + px(6.0);

        heading(
            crate::i18n::tr("Después de capturar", "After capturing"),
            left,
            &mut y,
        );
        check(
            crate::i18n::tr(
                "Copiar la captura al portapapeles",
                "Copy the capture to the clipboard",
            ),
            left,
            y,
            width,
            ID_COPY,
            settings.copy_after_capture,
        );
        y += row;
        check(
            crate::i18n::tr(
                "Mostrar la miniatura de acceso rápido",
                "Show the quick-access thumbnail",
            ),
            left,
            y,
            width,
            ID_OVERLAY,
            settings.show_overlay,
        );
        y += row;
        check(
            crate::i18n::tr(
                "Abrir el editor directamente",
                "Open the editor straight away",
            ),
            left,
            y,
            width,
            ID_EDITOR,
            settings.open_editor_after_capture,
        );
        y += row;
        check(
            crate::i18n::tr(
                "Guardar también en la carpeta de capturas",
                "Also save to the captures folder",
            ),
            left,
            y,
            width,
            ID_SAVE,
            settings.save_after_capture,
        );
        y += row;
        label(
            crate::i18n::tr("Cerrar la miniatura:", "Close the thumbnail:"),
            left,
            y,
            px(170.0),
        );
        let close_idx = CLOSE_CHOICES
            .iter()
            .position(|c| c.0 == settings.overlay_close_seconds)
            .unwrap_or(0);
        combo(
            left + px(180.0),
            y,
            px(200.0),
            ID_CLOSE_AFTER,
            &CLOSE_CHOICES.map(|c| crate::i18n::tr(c.1, c.2)),
            close_idx,
        );
        y += row + px(12.0);

        heading(crate::i18n::tr("Archivos", "Files"), left, &mut y);
        label(crate::i18n::tr("Carpeta:", "Folder:"), left, y, px(80.0));
        let folder = ctl(
            w!("EDIT"),
            &settings.save_dir.to_string_lossy(),
            ES_AUTOHSCROLL as u32 | ES_READONLY as u32 | WS_BORDER.0,
            left + px(90.0),
            y,
            px(320.0),
            px(26.0),
            ID_FOLDER,
        );
        let _ = folder;
        ctl(
            w!("BUTTON"),
            crate::i18n::tr("Cambiar…", "Change…"),
            BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
            left + px(420.0),
            y - px(1.0),
            px(100.0),
            px(28.0),
            ID_FOLDER_PICK,
        );
        y += row + px(4.0);
        label(crate::i18n::tr("Formato:", "Format:"), left, y, px(80.0));
        let png = ctl(
            w!("BUTTON"),
            "PNG",
            BS_AUTORADIOBUTTON as u32 | WS_GROUP.0 | WS_TABSTOP.0,
            left + px(90.0),
            y,
            px(70.0),
            px(24.0),
            ID_PNG,
        );
        let jpg = ctl(
            w!("BUTTON"),
            "JPEG",
            BS_AUTORADIOBUTTON as u32,
            left + px(170.0),
            y,
            px(80.0),
            px(24.0),
            ID_JPEG,
        );
        SendMessageW(
            if settings.image_format == ImageFormat::Png {
                png
            } else {
                jpg
            },
            BM_SETCHECK,
            Some(WPARAM(BST_CHECKED.0 as usize)),
            None,
        );
        label(
            crate::i18n::tr("Calidad JPEG:", "JPEG quality:"),
            left + px(270.0),
            y,
            px(110.0),
        );
        ctl(
            w!("EDIT"),
            &settings.jpeg_quality.to_string(),
            ES_NUMBER as u32 | WS_TABSTOP.0 | WS_BORDER.0,
            left + px(385.0),
            y,
            px(60.0),
            px(26.0),
            ID_QUALITY,
        );
        y += row + px(4.0);
        label(crate::i18n::tr("Historial:", "History:"), left, y, px(80.0));
        let days_idx = HISTORY_CHOICES
            .iter()
            .position(|c| c.0 == settings.history_days)
            .unwrap_or(2);
        combo(
            left + px(90.0),
            y,
            px(160.0),
            ID_HISTORY_DAYS,
            &HISTORY_CHOICES.map(|c| crate::i18n::tr(c.1, c.2)),
            days_idx,
        );
        ctl(
            w!("BUTTON"),
            crate::i18n::tr("Abrir historial", "Open history"),
            BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
            left + px(270.0),
            y - px(1.0),
            px(140.0),
            px(28.0),
            ID_HISTORY_OPEN,
        );
        y += row + px(12.0);

        // Two columns: in one, the window was taller than a 1080p screen at
        // 125 %, and Guardar fell behind the taskbar.
        let left_bottom = y;
        left += width + px(40.0);
        y = px(16.0);
        heading(crate::i18n::tr("Selección", "Selection"), left, &mut y);
        check(
            crate::i18n::tr("Mostrar la lupa", "Show the magnifier"),
            left,
            y,
            px(200.0),
            ID_MAGNIFIER,
            settings.show_magnifier,
        );
        check(
            crate::i18n::tr("Mostrar la cruz", "Show the crosshair"),
            left + px(220.0),
            y,
            px(200.0),
            ID_CROSSHAIR,
            settings.show_crosshair,
        );
        y += row;
        label(
            crate::i18n::tr("Temporizador:", "Timer:"),
            left,
            y,
            px(120.0),
        );
        let timer_labels: Vec<String> = TIMER_CHOICES
            .iter()
            .map(|t| crate::i18n::trs(format!("{t} segundos"), format!("{t} seconds")))
            .collect();
        let timer_refs: Vec<&str> = timer_labels.iter().map(String::as_str).collect();
        let timer_idx = TIMER_CHOICES
            .iter()
            .position(|t| *t == settings.self_timer_seconds)
            .unwrap_or(1);
        combo(
            left + px(130.0),
            y,
            px(160.0),
            ID_TIMER,
            &timer_refs,
            timer_idx,
        );
        y += row + px(12.0);

        heading(
            crate::i18n::tr("Atajos de teclado", "Keyboard shortcuts"),
            left,
            &mut y,
        );
        for (id, text, value) in [
            (
                ID_HK_AREA,
                crate::i18n::tr("Capturar área", "Capture area"),
                &settings.hotkey_area,
            ),
            (
                ID_HK_WINDOW,
                crate::i18n::tr("Capturar ventana", "Capture window"),
                &settings.hotkey_window,
            ),
            (
                ID_HK_FULL,
                crate::i18n::tr("Pantalla completa", "Full screen"),
                &settings.hotkey_fullscreen,
            ),
            (
                ID_HK_PREVIOUS,
                crate::i18n::tr("Área anterior", "Previous area"),
                &settings.hotkey_previous_area,
            ),
        ] {
            label(text, left, y, px(170.0));
            let t = wide(value);
            let field = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                HOTKEY_CLASS,
                PCWSTR(t.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0),
                left + px(180.0),
                y,
                px(260.0),
                px(26.0),
                Some(hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(instance.into()),
                None,
            )
            .unwrap_or_default();
            SendMessageW(
                field,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            y += row + px(2.0);
        }
        label(
            crate::i18n::tr(
                "Haz clic en un campo y pulsa la combinación. Retroceso lo vacía.",
                "Click a field and press the keys. Backspace clears it.",
            ),
            left,
            y,
            width,
        );
        y += row + px(12.0);

        y = y.max(left_bottom);
        left = px(24.0);
        let width = width * 2 + px(40.0);
        let bw = px(110.0);
        ctl(
            w!("BUTTON"),
            crate::i18n::tr("Restablecer", "Reset"),
            BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
            left,
            y,
            bw + px(10.0),
            px(32.0),
            ID_RESET,
        );
        ctl(
            w!("BUTTON"),
            crate::i18n::tr("Cancelar", "Cancel"),
            BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
            left + width - 2 * bw - px(10.0),
            y,
            bw,
            px(32.0),
            ID_CANCEL,
        );
        ctl(
            w!("BUTTON"),
            crate::i18n::tr("Guardar", "Save"),
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP.0,
            left + width - bw,
            y,
            bw,
            px(32.0),
            ID_OK,
        );
        y += px(32.0) + px(20.0);

        // Size the window around its content.
        let mut r = RECT {
            left: 0,
            top: 0,
            right: left * 2 + width,
            bottom: y,
        };
        let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(
            &mut r,
            WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            false,
            WS_EX_APPWINDOW,
            GetDpiForWindow(hwnd),
        );
        // Centred on the monitor it opened on, inside its work area.
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let monitor = windows::Win32::Graphics::Gdi::MonitorFromWindow(
            hwnd,
            windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
        );
        let mut mi = windows::Win32::Graphics::Gdi::MONITORINFO {
            cbSize: size_of::<windows::Win32::Graphics::Gdi::MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = windows::Win32::Graphics::Gdi::GetMonitorInfoW(monitor, &mut mi);
        let work = mi.rcWork;
        let x = work.left + ((work.right - work.left - ww) / 2).max(0);
        let y = work.top + ((work.bottom - work.top - wh) / 2).max(0);
        let _ = SetWindowPos(hwnd, None, x, y, ww, wh, SWP_NOZORDER);

        WINDOW.with(|w| {
            *w.borrow_mut() = Some(Window {
                hwnd,
                font,
                bold,
                s,
                headings,
            })
        });
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

fn item(hwnd: HWND, id: i32) -> HWND {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(hwnd), id).unwrap_or_default()
    }
}

fn checked(hwnd: HWND, id: i32) -> bool {
    unsafe { SendMessageW(item(hwnd, id), BM_GETCHECK, None, None).0 as u32 == BST_CHECKED.0 }
}

fn text(hwnd: HWND, id: i32) -> String {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetWindowTextW(item(hwnd, id), &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn selection(hwnd: HWND, id: i32) -> usize {
    unsafe {
        SendMessageW(item(hwnd, id), CB_GETCURSEL, None, None)
            .0
            .max(0) as usize
    }
}

/// Reads every control into settings, or says what is wrong.
fn read(hwnd: HWND) -> Result<Settings, String> {
    let mut s = agent::settings();
    s.copy_after_capture = checked(hwnd, ID_COPY);
    s.show_overlay = checked(hwnd, ID_OVERLAY);
    s.open_editor_after_capture = checked(hwnd, ID_EDITOR);
    s.save_after_capture = checked(hwnd, ID_SAVE);
    s.overlay_close_seconds =
        CLOSE_CHOICES[selection(hwnd, ID_CLOSE_AFTER).min(CLOSE_CHOICES.len() - 1)].0;
    s.save_dir = text(hwnd, ID_FOLDER).into();
    s.image_format = if checked(hwnd, ID_JPEG) {
        ImageFormat::Jpeg
    } else {
        ImageFormat::Png
    };
    s.jpeg_quality = match text(hwnd, ID_QUALITY).trim().parse::<u8>() {
        Ok(q) if (1..=100).contains(&q) => q,
        _ => {
            return Err(crate::i18n::tr(
                "La calidad JPEG va de 1 a 100.",
                "JPEG quality goes from 1 to 100.",
            )
            .into());
        }
    };
    s.history_days =
        HISTORY_CHOICES[selection(hwnd, ID_HISTORY_DAYS).min(HISTORY_CHOICES.len() - 1)].0;
    s.show_magnifier = checked(hwnd, ID_MAGNIFIER);
    s.show_crosshair = checked(hwnd, ID_CROSSHAIR);
    s.self_timer_seconds = TIMER_CHOICES[selection(hwnd, ID_TIMER).min(TIMER_CHOICES.len() - 1)];
    s.hotkey_area = text(hwnd, ID_HK_AREA);
    s.hotkey_window = text(hwnd, ID_HK_WINDOW);
    s.hotkey_fullscreen = text(hwnd, ID_HK_FULL);
    s.hotkey_previous_area = text(hwnd, ID_HK_PREVIOUS);
    s.language = LANGUAGE_CHOICES[selection(hwnd, ID_LANGUAGE).min(LANGUAGE_CHOICES.len() - 1)].0;
    let keys = [
        &s.hotkey_area,
        &s.hotkey_window,
        &s.hotkey_fullscreen,
        &s.hotkey_previous_area,
    ];
    for (i, a) in keys.iter().enumerate() {
        if !a.is_empty() && keys[i + 1..].iter().any(|b| b.eq_ignore_ascii_case(a)) {
            return Err(crate::i18n::trs(
                format!("El atajo «{a}» está repetido."),
                format!("The shortcut «{a}» is repeated."),
            ));
        }
        hotkeys::parse(a)?;
    }
    Ok(s)
}

fn apply(hwnd: HWND) -> bool {
    let s = match read(hwnd) {
        Ok(s) => s,
        Err(e) => {
            let t = wide(&e);
            unsafe {
                MessageBoxW(
                    Some(hwnd),
                    PCWSTR(t.as_ptr()),
                    w!("ScreenShotCM"),
                    MB_OK | MB_ICONWARNING,
                );
            }
            return false;
        }
    };
    if let Err(e) = autostart::set(checked(hwnd, ID_AUTOSTART)) {
        agent::notify(
            crate::i18n::tr(
                "No se pudo cambiar el inicio con Windows",
                "Could not change starting with Windows",
            ),
            &e,
            true,
        );
    }
    if let Err(e) = s.save() {
        agent::notify(
            crate::i18n::tr(
                "No se pudieron guardar los ajustes",
                "Could not save the settings",
            ),
            &e.to_string(),
            true,
        );
        return false;
    }
    agent::replace_settings(s);
    true
}

fn pick_folder(hwnd: HWND) {
    unsafe {
        let Ok(dialog) =
            CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
        else {
            return;
        };
        let _ = dialog.SetOptions(FOS_PICKFOLDERS);
        let _ = dialog.SetTitle(&windows::core::HSTRING::from(crate::i18n::tr(
            "Carpeta para las capturas",
            "Folder for captures",
        )));
        if dialog.Show(Some(hwnd)).is_err() {
            return;
        }
        let Ok(item_) = dialog.GetResult() else {
            return;
        };
        let Ok(path) = item_.GetDisplayName(SIGDN_FILESYSPATH) else {
            return;
        };
        let _ = SetWindowTextW(item(hwnd, ID_FOLDER), PCWSTR(path.0));
        CoTaskMemFree(Some(path.0 as *const _));
    }
}

fn reset(hwnd: HWND) {
    let d = Settings::default();
    let set_check = |id, on: bool| unsafe {
        SendMessageW(
            item(hwnd, id),
            BM_SETCHECK,
            Some(WPARAM(if on { BST_CHECKED.0 as usize } else { 0 })),
            None,
        );
    };
    set_check(ID_COPY, d.copy_after_capture);
    set_check(ID_OVERLAY, d.show_overlay);
    set_check(ID_EDITOR, d.open_editor_after_capture);
    set_check(ID_SAVE, d.save_after_capture);
    set_check(ID_PNG, d.image_format == ImageFormat::Png);
    set_check(ID_JPEG, d.image_format == ImageFormat::Jpeg);
    set_check(ID_MAGNIFIER, d.show_magnifier);
    set_check(ID_CROSSHAIR, d.show_crosshair);
    let set_text = |id, v: &str| unsafe {
        let t = wide(v);
        let _ = SetWindowTextW(item(hwnd, id), PCWSTR(t.as_ptr()));
        let _ = InvalidateRect(Some(item(hwnd, id)), None, true);
    };
    set_text(ID_FOLDER, &d.save_dir.to_string_lossy());
    set_text(ID_QUALITY, &d.jpeg_quality.to_string());
    set_text(ID_HK_AREA, &d.hotkey_area);
    set_text(ID_HK_WINDOW, &d.hotkey_window);
    set_text(ID_HK_FULL, &d.hotkey_fullscreen);
    set_text(ID_HK_PREVIOUS, &d.hotkey_previous_area);
    let sel = |id, i: usize| unsafe {
        SendMessageW(item(hwnd, id), CB_SETCURSEL, Some(WPARAM(i)), None);
    };
    sel(
        ID_CLOSE_AFTER,
        CLOSE_CHOICES
            .iter()
            .position(|c| c.0 == d.overlay_close_seconds)
            .unwrap_or(0),
    );
    sel(
        ID_HISTORY_DAYS,
        HISTORY_CHOICES
            .iter()
            .position(|c| c.0 == d.history_days)
            .unwrap_or(2),
    );
    sel(ID_LANGUAGE, 0);
    sel(
        ID_TIMER,
        TIMER_CHOICES
            .iter()
            .position(|t| *t == d.self_timer_seconds)
            .unwrap_or(1),
    );
}

fn close(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                WINDOW.with(|w| {
                    if let Some(w) = w.borrow().as_ref() {
                        paint_headings(dc, w);
                    }
                });
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            // Labels and checkboxes on the window's own white.
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                let dc = HDC(wparam.0 as *mut _);
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
                LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                match id {
                    ID_FOLDER_PICK => pick_folder(hwnd),
                    ID_HISTORY_OPEN => crate::historywin::open(),
                    ID_RESET => reset(hwnd),
                    ID_CANCEL => close(hwnd),
                    ID_OK if apply(hwnd) => {
                        close(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                close(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let state = WINDOW.with(|w| w.borrow_mut().take());
                if let Some(w) = state {
                    let _ = DeleteObject(w.font.into());
                    let _ = DeleteObject(w.bold.into());
                }
                // The hotkeys come back, with whatever was saved.
                agent::reregister_hotkeys();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn paint_headings(dc: HDC, w: &Window) {
    unsafe {
        let old = SelectObject(dc, w.bold.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
        for (text, x, y) in &w.headings {
            let left = *x;
            let mut t: Vec<u16> = text.encode_utf16().collect();
            let mut r = RECT {
                left,
                top: *y,
                right: left + (520.0 * w.s) as i32,
                bottom: *y + (26.0 * w.s) as i32,
            };
            DrawTextW(dc, &mut t, &mut r, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
            // A hairline from the end of the heading to the column's edge.
            let mut size = windows::Win32::Foundation::SIZE::default();
            let _ = windows::Win32::Graphics::Gdi::GetTextExtentPoint32W(dc, &t, &mut size);
            let line = RECT {
                left: left + size.cx + (12.0 * w.s) as i32,
                top: *y + (13.0 * w.s) as i32,
                right: left + (520.0 * w.s) as i32,
                bottom: *y + (14.0 * w.s) as i32,
            };
            let brush = CreateSolidBrush(COLORREF(0x00E5E5E5));
            FillRect(dc, &line, brush);
            let _ = DeleteObject(brush.into());
        }
        SelectObject(dc, old);
    }
}

// --- The shortcut field ---------------------------------------------------------

/// The keys held, as the text the settings file uses: "Ctrl+Shift+PrintScreen".
fn chord_text(vk: u32) -> Option<String> {
    let key = match vk {
        v if v == VK_SNAPSHOT.0 as u32 => "PrintScreen".to_string(),
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => return None,
    };
    let down = |k: u16| unsafe { GetKeyState(k as i32) < 0 };
    let mut parts = Vec::new();
    if down(VK_CONTROL.0) {
        parts.push("Ctrl".to_string());
    }
    if down(VK_MENU.0) {
        parts.push("Alt".to_string());
    }
    if down(VK_SHIFT.0) {
        parts.push("Shift".to_string());
    }
    if down(VK_LWIN.0) || down(VK_RWIN.0) {
        parts.push("Win".to_string());
    }
    parts.push(key);
    Some(parts.join("+"))
}

extern "system" fn hotkey_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_GETDLGCODE => LRESULT(DLGC_WANTALLKEYS as isize),
            WM_LBUTTONDOWN => {
                let _ = SetFocus(Some(hwnd));
                LRESULT(0)
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                let _ = InvalidateRect(Some(hwnd), None, true);
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
                let vk = wparam.0 as u32;
                let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
                // PrintScreen arrives only as a key-up.
                if up && vk != VK_SNAPSHOT.0 as u32 {
                    return LRESULT(0);
                }
                if !up && (vk == VK_BACK.0 as u32 || vk == VK_DELETE.0 as u32) {
                    let _ = SetWindowTextW(hwnd, w!(""));
                } else if !up && (vk == VK_TAB.0 as u32 || vk == VK_ESCAPE.0 as u32) {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                } else if let Some(text) = chord_text(vk) {
                    let t = wide(&text);
                    let _ = SetWindowTextW(hwnd, PCWSTR(t.as_ptr()));
                }
                let _ = InvalidateRect(Some(hwnd), None, true);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);
                let focused = windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() == hwnd;
                let bg = CreateSolidBrush(COLORREF(if focused { 0x00FFF4E6 } else { 0x00FFFFFF }));
                FillRect(dc, &rc, bg);
                let _ = DeleteObject(bg.into());
                if focused {
                    let frame = CreateSolidBrush(COLORREF(0x00FF7A0A));
                    FrameRect(dc, &rc, frame);
                    let _ = DeleteObject(frame.into());
                }
                let font = WINDOW
                    .with(|w| w.borrow().as_ref().map(|w| w.font))
                    .unwrap_or_default();
                let old = SelectObject(dc, font.into());
                SetBkMode(dc, TRANSPARENT);
                let mut buf = [0u16; 128];
                let n = GetWindowTextW(hwnd, &mut buf) as usize;
                let shown = if n == 0 {
                    SetTextColor(dc, COLORREF(GetSysColor(COLOR_GRAYTEXT)));
                    if focused {
                        crate::i18n::tr("Pulsa la combinación…", "Press the keys…").to_string()
                    } else {
                        crate::i18n::tr("Sin atajo", "No shortcut").to_string()
                    }
                } else {
                    SetTextColor(dc, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
                    String::from_utf16_lossy(&buf[..n])
                        .replace("PrintScreen", crate::i18n::tr("Impr Pant", "PrtScn"))
                };
                let mut t: Vec<u16> = shown.encode_utf16().collect();
                let mut r = RECT {
                    left: rc.left + 8,
                    ..rc
                };
                DrawTextW(dc, &mut t, &mut r, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                SelectObject(dc, old);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
