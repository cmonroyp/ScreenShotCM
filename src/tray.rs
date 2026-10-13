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
            crate::i18n::tr("Capturar área", "Capture area"),
            &settings.hotkey_area,
        ),
        (
            Some(Command::CapturePreviousArea),
            crate::i18n::tr("Capturar el área anterior", "Capture previous area"),
            &settings.hotkey_previous_area,
        ),
        (
            Some(Command::CaptureWindow),
            crate::i18n::tr("Capturar ventana", "Capture window"),
            &settings.hotkey_window,
        ),
        (
            Some(Command::CaptureFullscreen),
            crate::i18n::tr("Capturar pantalla completa", "Capture full screen"),
            &settings.hotkey_fullscreen,
        ),
        (
            Some(Command::SelfTimer),
            crate::i18n::tr("Temporizador", "Timer"),
            "",
        ),
        (None, "", ""),
        (
            Some(Command::OpenFromClipboard),
            crate::i18n::tr("Abrir imagen del portapapeles", "Open image from clipboard"),
            "",
        ),
        (
            Some(Command::OpenFile),
            crate::i18n::tr("Anotar una imagen…", "Annotate an image…"),
            "",
        ),
        (
            Some(Command::OpenHistory),
            crate::i18n::tr("Historial de capturas", "Capture history"),
            "",
        ),
        (
            Some(Command::RestoreRecentlyClosed),
            crate::i18n::tr(
                "Restaurar la última captura cerrada",
                "Restore the last closed capture",
            ),
            "",
        ),
        (None, "", ""),
        (
            Some(Command::UnlockPins),
            crate::i18n::tr("Desbloquear capturas fijadas", "Unlock pinned captures"),
            "",
        ),
        (
            Some(Command::ClosePins),
            crate::i18n::tr("Cerrar capturas fijadas", "Close pinned captures"),
            "",
        ),
        (None, "", ""),
        (
            Some(Command::OpenSaveFolder),
            crate::i18n::tr("Abrir carpeta de capturas", "Open the captures folder"),
            "",
        ),
        (None, "", ""),
        (
            Some(Command::OpenSettings),
            crate::i18n::tr("Ajustes…", "Settings…"),
            "",
        ),
        (
            Some(Command::ReloadSettings),
            crate::i18n::tr("Recargar ajustes", "Reload settings"),
            "",
        ),
        (
            Some(Command::ToggleAutostart),
            crate::i18n::tr("Iniciar con Windows", "Start with Windows"),
            "",
        ),
        (None, "", ""),
        (Some(Command::Quit), crate::i18n::tr("Salir", "Quit"), ""),
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
    text.replace("PrintScreen", crate::i18n::tr("Impr Pant", "PrtScn"))
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
