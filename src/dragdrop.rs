//! Dragging a capture out to another program, as a file. The shell builds the
//! data object and drives the drag (`SHDoDragDrop` with no drop source of our
//! own), so whatever accepts a file from Explorer accepts a capture: mail,
//! chat, a browser upload, a folder.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE};
use windows::Win32::UI::Shell::{
    BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop,
};
use windows::core::PCWSTR;

use crate::util::wide;

/// How long a dragged file outlives its drag. Explorer may copy a dropped
/// file after the drop has returned; a minute covers it.
const AFTER_DROP: std::time::Duration = std::time::Duration::from_secs(60);

fn staging_dir() -> PathBuf {
    crate::paths::local_dir().join("drag")
}

/// The folder a drag hands its file over from. Nothing the user did not
/// save is kept: each new drag removes the files of drags over a minute
/// old, and the folder is emptied when the agent starts and when it quits
/// (`clear_staging`). No thread waits on a file: one each, a burst of drags
/// held a hundred and more.
pub fn staging() -> PathBuf {
    let dir = staging_dir();
    let _ = std::fs::create_dir_all(&dir);
    remove_files(&dir, Some(AFTER_DROP));
    dir
}

/// Empties the drag folder.
pub fn clear_staging() {
    remove_files(&staging_dir(), None);
}

/// Removes the files in `dir`, or only those older than `older_than`.
fn remove_files(dir: &Path, older_than: Option<std::time::Duration>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let old = older_than.is_none_or(|limit| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > limit)
        });
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Starts a drag of `path`, a copy in `staging`, and returns once it ends;
/// `true` when something took the file. Copy and move are both offered: a
/// program picks one without a modifier key — Word copies, Paint moves —
/// and with copy alone Paint refused the drop unless Ctrl was held.
pub fn drag_file(hwnd: HWND, path: &Path) -> bool {
    match file_data(path) {
        Ok(data) => unsafe {
            SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE)
                .map(|effect: DROPEFFECT| effect != DROPEFFECT(0))
                .unwrap_or(false)
        },
        Err(_) => false,
    }
}

/// The data object Explorer itself drags for `path`: the file list
/// (CF_HDROP), the shell's item list and the file's contents. The one
/// `SHCreateDataObject` made from a lone absolute item list carried only
/// the drag image, and every program refused the drop.
fn file_data(path: &Path) -> windows::core::Result<IDataObject> {
    let p = wide(path);
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(p.as_ptr()), None)?;
        item.BindToHandler(None, &BHID_DataObject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};

    #[test]
    fn a_dragged_capture_carries_the_file() {
        crate::image::tests::com_for_test();
        let path = std::env::temp_dir().join(format!("scm-drag-{}.png", std::process::id()));
        std::fs::write(&path, b"not really a png").unwrap();
        let data = file_data(&path).unwrap();
        let hdrop = FORMATETC {
            cfFormat: 15, // CF_HDROP
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        let offered = unsafe { data.QueryGetData(&hdrop) };
        let _ = std::fs::remove_file(&path);
        assert!(offered.is_ok(), "no file list in the drag: {offered:?}");
    }

    /// Everything a drag does but the drag itself, many times over, for a
    /// memory measurement taken from outside (`--ignored`): the process
    /// must level off, not grow with the count.
    #[test]
    #[ignore]
    fn many_drags_level_off() {
        crate::image::tests::com_for_test();
        let image = crate::image::Image {
            width: 640,
            height: 400,
            pixels: [40u8, 90, 160, 255].repeat(640 * 400),
        };
        let dir = std::env::temp_dir().join(format!("scm-drag-mem-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for n in 0..4000 {
            let path = dir.join(format!("{}.png", n % 50));
            crate::image::save(&image, &path, crate::settings::ImageFormat::Png, 100).unwrap();
            let data = file_data(&path).unwrap();
            drop(data);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
