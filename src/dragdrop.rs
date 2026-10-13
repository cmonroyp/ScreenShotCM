//! Dragging a capture out to another program, as a file. The shell builds the
//! data object and drives the drag (`SHDoDragDrop` with no drop source of our
//! own), so whatever accepts a file from Explorer accepts a capture: mail,
//! chat, a browser upload, a folder.

use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{SHCreateDataObject, SHDoDragDrop, SHParseDisplayName};
use windows::core::PCWSTR;

use crate::util::wide;

/// Starts a drag of `path` and returns once it ends; `true` when something
/// took the file.
pub fn drag_file(hwnd: HWND, path: &Path) -> bool {
    let p = wide(path);
    unsafe {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(PCWSTR(p.as_ptr()), None, &mut pidl, 0, None).is_err()
            || pidl.is_null()
        {
            return false;
        }
        let items = [pidl as *const ITEMIDLIST];
        let data: windows::core::Result<IDataObject> = SHCreateDataObject(None, Some(&items), None);
        let dropped = match data {
            Ok(data) => SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY)
                .map(|effect: DROPEFFECT| effect != DROPEFFECT(0))
                .unwrap_or(false),
            Err(_) => false,
        };
        CoTaskMemFree(Some(pidl as *const _));
        dropped
    }
}
