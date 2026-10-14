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

/// The folder a drag hands its file over from. A file there is a copy made
/// for the drag, so a target may take it — but Paint opens the dropped
/// file where it is, and saves over it there: files are kept a week, not
/// cleared on the next drag, so an edit saved in Paint is not lost to it.
pub fn staging() -> PathBuf {
    const KEEP: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
    let dir = crate::paths::local_dir().join("drag");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > KEEP);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    dir
}

/// Starts a drag of `path`, a file in `staging`, and returns once it
/// ends; `true` when something took the file. Copy and move are both
/// offered: a program picks one without a modifier key — Word copies,
/// Paint moves — and with copy alone Paint refused the drop unless Ctrl
/// was held.
pub fn drag_file(hwnd: HWND, path: &Path) -> bool {
    let Ok(data) = file_data(path) else {
        return false;
    };
    unsafe {
        SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE)
            .map(|effect: DROPEFFECT| effect != DROPEFFECT(0))
            .unwrap_or(false)
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
}
