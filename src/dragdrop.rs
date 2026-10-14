//! Dragging a capture out to another program, as a file. The shell builds the
//! data object and drives the drag (`SHDoDragDrop` with no drop source of our
//! own), so whatever accepts a file from Explorer accepts a capture: mail,
//! chat, a browser upload, a folder.

use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY};
use windows::Win32::UI::Shell::{
    BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop,
};
use windows::core::PCWSTR;

use crate::util::wide;

/// Starts a drag of `path` and returns once it ends; `true` when something
/// took the file.
pub fn drag_file(hwnd: HWND, path: &Path) -> bool {
    let Ok(data) = file_data(path) else {
        return false;
    };
    unsafe {
        SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY)
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
