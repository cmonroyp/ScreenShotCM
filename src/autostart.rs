//! Start with Windows: the per-user `Run` key, which needs no elevation and is
//! what Task Manager's "Startup apps" lists and lets the user switch off.

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

use crate::util::wide;

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("ScreenShotCM");

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
