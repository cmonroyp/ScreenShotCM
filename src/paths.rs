//! Where the agent keeps things. Everything is per user: the agent runs as
//! the signed-in user and never needs elevation.

use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_Pictures, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT,
    SHGetKnownFolderPath,
};
use windows::core::GUID;

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

const APP: &str = "ScreenShotCM";

/// Settings travel with a roaming profile.
pub fn config_dir() -> PathBuf {
    known_folder(&FOLDERID_RoamingAppData)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP)
}

/// History and logs are this machine's.
pub fn local_dir() -> PathBuf {
    known_folder(&FOLDERID_LocalAppData)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP)
}

pub fn history_dir() -> PathBuf {
    local_dir().join("history")
}

pub fn log_dir() -> PathBuf {
    local_dir().join("logs")
}

/// The default for "Save": the user's Pictures\ScreenShotCM.
pub fn default_save_dir() -> PathBuf {
    known_folder(&FOLDERID_Pictures)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP)
}
