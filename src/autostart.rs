//! Start with Windows. Installed by the MSI it is the per-user `Run` key,
//! which needs no elevation and is what Task Manager's "Startup apps" lists.
//! Installed from the Store (an MSIX package) the registry is the package's
//! own, so the `Run` key would never be read: there it is the package's
//! startup task, declared in its manifest and switched through the
//! `StartupTask` API.

use windows::ApplicationModel::{StartupTask, StartupTaskState};
use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::util::wide;

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("ScreenShotCM");
/// The `TaskId` of the startup task in the MSIX manifest.
const TASK_ID: &str = "ScreenShotCMStartup";

/// Whether this process runs from an MSIX package.
pub fn packaged() -> bool {
    let mut len = 0u32;
    let rc = unsafe { GetCurrentPackageFullName(&mut len, None) };
    rc != APPMODEL_ERROR_NO_PACKAGE
}

fn task() -> Result<StartupTask, String> {
    StartupTask::GetAsync(&HSTRING::from(TASK_ID))
        .and_then(|op| op.join())
        .map_err(|e| e.message())
}

fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let mut key = HKEY::default();
    unsafe {
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN, None, access, &mut key)
            .ok()
            .ok()?
    };
    Some(key)
}

pub fn is_enabled() -> bool {
    if packaged() {
        return task()
            .and_then(|t| t.State().map_err(|e| e.message()))
            .is_ok_and(|s| {
                s == StartupTaskState::Enabled || s == StartupTaskState::EnabledByPolicy
            });
    }
    let Some(key) = open(KEY_READ) else {
        return false;
    };
    let found = unsafe { RegQueryValueExW(key, NAME, None, None, None, None).is_ok() };
    unsafe {
        let _ = RegCloseKey(key);
    }
    found
}

pub fn set(enabled: bool) -> Result<(), String> {
    if packaged() {
        let t = task()?;
        if !enabled {
            return t.Disable().map_err(|e| e.message());
        }
        let state = t
            .RequestEnableAsync()
            .and_then(|op| op.join())
            .map_err(|e| e.message())?;
        return match state {
            StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy => Ok(()),
            // Switched off by the user in Task Manager or Settings: only
            // they can switch it back on, there.
            StartupTaskState::DisabledByUser => Err(crate::i18n::tr(
                "Está desactivado en Configuración → Aplicaciones → Inicio; actívalo allí.",
                "It is turned off in Settings → Apps → Startup; turn it on there.",
            )
            .into()),
            _ => Err(crate::i18n::tr(
                "Una directiva del equipo no permite iniciarlo con Windows.",
                "A policy on this PC does not allow it to start with Windows.",
            )
            .into()),
        };
    }
    let key = open(KEY_SET_VALUE).ok_or("no se pudo abrir la clave Run del registro")?;
    let result = unsafe {
        if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            // Quoted, because the install folder may contain spaces.
            let value = wide(format!("\"{}\"", exe.display()));
            let bytes = std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2);
            RegSetValueExW(key, NAME, None, REG_SZ, Some(bytes))
        } else {
            RegDeleteValueW(key, NAME)
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    result.ok().map_err(|e| e.message())
}
