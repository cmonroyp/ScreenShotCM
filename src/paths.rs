//! Where the agent keeps things. Everything is per user: the agent runs as
//! the signed-in user and never needs elevation.

use std::path::{Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_RoamingAppData,
    KF_FLAG_DEFAULT, SHGetKnownFolderPath,
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

/// The default for "Save": the user's Pictures\ScreenShotCM, on this PC.
/// When OneDrive backs up Pictures, Windows' Pictures is a folder OneDrive
/// uploads, and every capture would leave the PC unasked; the profile's own
/// Pictures stays local.
pub fn default_save_dir() -> PathBuf {
    let pictures = known_folder(&FOLDERID_Pictures).unwrap_or_else(std::env::temp_dir);
    if !in_onedrive(&pictures) {
        return pictures.join(APP);
    }
    known_folder(&FOLDERID_Profile)
        .map(|p| p.join("Pictures"))
        .unwrap_or(pictures)
        .join(APP)
}

/// The default every version before 0.4.13 wrote into the settings: moved to
/// the local one, since nobody chose a folder OneDrive uploads.
pub fn is_synced_default(dir: &Path) -> bool {
    known_folder(&FOLDERID_Pictures).is_some_and(|p| in_onedrive(&p) && dir == p.join(APP))
}

/// Whether `path` lies in a folder the OneDrive client keeps in sync. The
/// client publishes its roots as these variables.
fn in_onedrive(path: &Path) -> bool {
    ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(std::env::var_os)
        .any(|root| under(path, Path::new(&root)))
}

/// `path` is `root` or inside it, ignoring case as Windows does.
fn under(path: &Path, root: &Path) -> bool {
    let (p, r) = (
        path.to_string_lossy().to_lowercase(),
        root.to_string_lossy().trim_end_matches('\\').to_lowercase(),
    );
    !r.is_empty() && (p == r || p.starts_with(&(r + "\\")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_inside_a_onedrive_root_is_under_it_and_a_neighbour_is_not() {
        let root = Path::new(r"C:\Users\Ana\OneDrive");
        assert!(under(
            Path::new(r"C:\Users\Ana\OneDrive\Imágenes\ScreenShotCM"),
            root
        ));
        assert!(under(Path::new(r"c:\users\ana\onedrive"), root));
        assert!(under(
            Path::new(r"C:\Users\Ana\OneDrive\x"),
            Path::new(r"C:\Users\Ana\OneDrive\")
        ));
        assert!(!under(
            Path::new(r"C:\Users\Ana\OneDrive - Empresa\x"),
            root
        ));
        assert!(!under(
            Path::new(r"C:\Users\Ana\Pictures\ScreenShotCM"),
            root
        ));
        assert!(!under(Path::new(r"C:\x"), Path::new("")));
    }
}
