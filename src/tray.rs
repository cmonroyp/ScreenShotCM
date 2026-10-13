//! The notification-area icon and its menu.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, HICON, HMENU, MF_CHECKED, MF_SEPARATOR,
    MF_STRING, PostMessageW, SM_CXSMICON, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_APP, WM_NULL,
};
use windows::core::PCWSTR;

use crate::commands::Command;
use crate::settings::Settings;
use crate::util::{copy_into, wide};
use crate::{autostart, icon};

pub const WM_TRAY: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.max(16) as u32;
        let icon = icon::create(size).unwrap_or_default();
        let tray = Self { hwnd, icon };
        tray.add();
        tray
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..Default::default()
        };
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        nid
    }

    /// Adds the icon; also called when Explorer restarts and the taskbar
    /// comes back empty (`TaskbarCreated`).
    pub fn add(&self) {
        let mut nid = self.data();
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = self.icon;
        copy_into(&mut nid.szTip, "ScreenShotCM");
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
    }

    /// A balloon notification, for what the operator must know about.
    pub fn notify(&self, title: &str, text: &str, warning: bool) {
        let mut nid = self.data();
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        copy_into(&mut nid.szInfoTitle, title);
        copy_into(&mut nid.szInfo, text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }
}

/// Shows the menu at `at` and returns the command chosen, if any. A free
/// function, so no borrow of the agent is held across the menu's own
/// message loop.
pub fn menu(hwnd: HWND, at: POINT, settings: &Settings) -> Option<Command> {
    let items: &[(Option<Command>, &str, &str)] = &[
        (
            Some(Command::CaptureArea),
            "Capturar área",
            &settings.hotkey_area,
        ),
        (
            Some(Command::CapturePreviousArea),
            "Capturar el área anterior",
            &settings.hotkey_previous_area,
        ),
        (
            Some(Command::CaptureWindow),
            "Capturar ventana",
            &settings.hotkey_window,
        ),
        (
            Some(Command::CaptureFullscreen),
            "Capturar pantalla completa",
            &settings.hotkey_fullscreen,
        ),
        (Some(Command::SelfTimer), "Temporizador", ""),
        (None, "", ""),
        (
            Some(Command::OpenFromClipboard),
            "Abrir imagen del portapapeles",
            "",
        ),
        (Some(Command::OpenFile), "Anotar una imagen…", ""),
        (Some(Command::OpenHistory), "Historial de capturas", ""),
        (
            Some(Command::RestoreRecentlyClosed),
            "Restaurar la última captura cerrada",
            "",
        ),
        (None, "", ""),
        (
            Some(Command::UnlockPins),
            "Desbloquear capturas fijadas",
            "",
        ),
        (Some(Command::ClosePins), "Cerrar capturas fijadas", ""),
        (None, "", ""),
        (
            Some(Command::OpenSaveFolder),
            "Abrir carpeta de capturas",
            "",
        ),
        (None, "", ""),
        (Some(Command::OpenSettings), "Ajustes…", ""),
        (Some(Command::ReloadSettings), "Recargar ajustes", ""),
        (Some(Command::ToggleAutostart), "Iniciar con Windows", ""),
        (None, "", ""),
        (Some(Command::Quit), "Salir", ""),
    ];
    unsafe {
        let menu: HMENU = CreatePopupMenu().ok()?;
        for (i, (cmd, label, key)) in items.iter().enumerate() {
            match cmd {
                None => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                }
                Some(c) => {
                    let text = if key.is_empty() {
                        label.to_string()
                    } else {
                        format!("{label}\t{}", display_key(key))
                    };
                    let mut flags = MF_STRING;
                    if *c == Command::ToggleAutostart && autostart::is_enabled() {
                        flags |= MF_CHECKED;
                    }
                    let w = wide(&text);
                    let _ = AppendMenuW(menu, flags, i + 1, PCWSTR(w.as_ptr()));
                }
            }
        }
        // Without the foreground the menu never closes when the user
        // clicks elsewhere; the WM_NULL afterwards is the documented
        // companion of that call.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0,
            at.x,
            at.y,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, Default::default(), Default::default());
        let _ = DestroyMenu(menu);
        let index = chosen.0 as usize;
        if index == 0 {
            None
        } else {
            items.get(index - 1).and_then(|i| i.0)
        }
    }
}

/// `PrintScreen` reads better as the key's printed name.
fn display_key(text: &str) -> String {
    text.replace("PrintScreen", "Impr Pant")
}

impl Drop for Tray {
    fn drop(&mut self) {
        let nid = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            if !self.icon.is_invalid() {
                let _ = DestroyIcon(self.icon);
            }
        }
    }
}
