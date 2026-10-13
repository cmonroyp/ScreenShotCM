//! The clipboard: an image goes as `CF_DIB`, which every Windows program
//! reads, and text as `CF_UNICODETEXT`.

use std::cell::RefCell;
use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFOHEADER};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetOpenClipboardWindow,
    IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, KillTimer, SetTimer};
use windows::core::PWSTR;

use crate::image::Image;

/// The clipboard is shared: another program may have it open. A short try
/// here; a longer wait never blocks the UI thread (see `copy_image`).
fn open(owner: HWND) -> bool {
    for _ in 0..5 {
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

/// The program that has the clipboard open, by its executable's name.
pub fn holder() -> Option<String> {
    unsafe {
        let hwnd = GetOpenClipboardWindow().ok().filter(|h| !h.is_invalid())?;
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(process);
        ok.then(|| {
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit('\\').next().unwrap_or(&path).to_string()
        })
    }
}

enum PutError {
    Busy,
    Failed(String),
}

fn put(owner: HWND, format: u16, bytes: &[u8]) -> Result<(), PutError> {
    unsafe {
        let mem =
            GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| PutError::Failed(e.to_string()))?;
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(mem));
            return Err(PutError::Failed("GlobalLock falló".into()));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem);
        if !open(owner) {
            let _ = GlobalFree(Some(mem));
            return Err(PutError::Busy);
        }
        let _ = EmptyClipboard();
        // On success the system owns the memory; on failure we still do.
        let set = SetClipboardData(format as u32, Some(HANDLE(mem.0)));
        let _ = CloseClipboard();
        if set.is_err() {
            let _ = GlobalFree(Some(mem));
            return Err(PutError::Failed("SetClipboardData falló".into()));
        }
    }
    Ok(())
}

/// An image waiting for the clipboard to be free, and since when.
struct Pending {
    owner: HWND,
    dib: Vec<u8>,
    since: Instant,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

/// The agent's timer that retries a pending copy.
pub const RETRY_TIMER: usize = 42;
const RETRY_FOR: Duration = Duration::from_secs(5);

/// Copies `image` now, or — when another program has the clipboard open, as
/// a clipboard manager does after every copy — keeps trying every 100 ms for
/// five seconds without blocking anything. A clipboard manager held it for over a
/// second on 2026-10-13. A later copy replaces one still waiting.
pub fn copy_image(owner: HWND, image: &Image) -> Result<(), String> {
    let dib = dib_bytes(image);
    match put(owner, CF_DIB.0, &dib) {
        Ok(()) => {
            PENDING.with(|p| p.borrow_mut().take());
            Ok(())
        }
        Err(PutError::Failed(e)) => Err(e),
        Err(PutError::Busy) => {
            PENDING.with(|p| {
                *p.borrow_mut() = Some(Pending {
                    owner,
                    dib,
                    since: Instant::now(),
                })
            });
            unsafe {
                SetTimer(Some(owner), RETRY_TIMER, 100, None);
            }
            crate::log::info(&format!(
                "clipboard busy ({}), retrying",
                holder().unwrap_or_else(|| "?".into())
            ));
            Ok(())
        }
    }
}

/// Called by the owner's `RETRY_TIMER`.
pub fn retry() {
    let Some((owner, outcome, waited)) = PENDING.with(|p| {
        let p = p.borrow();
        let pending = p.as_ref()?;
        Some((
            pending.owner,
            put(pending.owner, CF_DIB.0, &pending.dib),
            pending.since.elapsed(),
        ))
    }) else {
        return;
    };
    let done = match outcome {
        Ok(()) => {
            crate::log::info(&format!("clipboard set after {} ms", waited.as_millis()));
            true
        }
        Err(PutError::Failed(e)) => {
            crate::agent::notify("No se pudo copiar la captura", &e, true);
            true
        }
        Err(PutError::Busy) if waited >= RETRY_FOR => {
            let who = holder()
                .map(|h| format!("{h} tiene el portapapeles ocupado."))
                .unwrap_or_else(|| "Otro programa tiene el portapapeles ocupado.".into());
            crate::log::error(&format!(
                "clipboard: gave up after {} ms: {who}",
                waited.as_millis()
            ));
            crate::agent::notify(
                "No se pudo copiar la captura",
                &format!("{who} La captura está en el historial."),
                true,
            );
            true
        }
        Err(PutError::Busy) => false,
    };
    if done {
        PENDING.with(|p| p.borrow_mut().take());
        unsafe {
            let _ = KillTimer(Some(owner), RETRY_TIMER);
        }
    }
}

/// A bottom-up 32-bit DIB: the most widely understood form.
pub fn dib_bytes(image: &Image) -> Vec<u8> {
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: image.width as i32,
        biHeight: image.height as i32,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: (image.width * image.height * 4),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(size_of::<BITMAPINFOHEADER>() + image.pixels.len());
    out.extend_from_slice(unsafe {
        std::slice::from_raw_parts(
            &header as *const _ as *const u8,
            size_of::<BITMAPINFOHEADER>(),
        )
    });
    for y in (0..image.height as usize).rev() {
        out.extend_from_slice(&image.pixels[y * image.stride()..(y + 1) * image.stride()]);
    }
    out
}

/// Text, for copying out of a text annotation being edited.
pub fn set_text(owner: HWND, text: &str) -> Result<(), String> {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(units.as_ptr() as *const u8, units.len() * 2) };
    put(owner, CF_UNICODETEXT.0, bytes).map_err(|e| match e {
        PutError::Busy => "otro programa tiene el portapapeles abierto".to_string(),
        PutError::Failed(e) => e,
    })
}

/// The clipboard's text, if it holds any.
pub fn get_text(owner: HWND) -> Option<String> {
    unsafe {
        IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).ok()?;
        if !open(owner) {
            return None;
        }
        let text = GetClipboardData(CF_UNICODETEXT.0 as u32)
            .ok()
            .and_then(|h| {
                let mem = HGLOBAL(h.0);
                let size = GlobalSize(mem) / 2;
                let ptr = GlobalLock(mem) as *const u16;
                if ptr.is_null() {
                    return None;
                }
                let units = std::slice::from_raw_parts(ptr, size);
                let len = units.iter().position(|u| *u == 0).unwrap_or(units.len());
                let text = String::from_utf16_lossy(&units[..len]);
                let _ = GlobalUnlock(mem);
                Some(text)
            });
        let _ = CloseClipboard();
        text
    }
}

/// The clipboard's image, if it holds one as a DIB of 24 or 32 bits.
pub fn get_image(owner: HWND) -> Option<Image> {
    unsafe {
        IsClipboardFormatAvailable(CF_DIB.0 as u32).ok()?;
        if !open(owner) {
            return None;
        }
        let image = GetClipboardData(CF_DIB.0 as u32).ok().and_then(|h| {
            let mem = HGLOBAL(h.0);
            let size = GlobalSize(mem);
            let ptr = GlobalLock(mem) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let decoded = decode_dib(std::slice::from_raw_parts(ptr, size));
            let _ = GlobalUnlock(mem);
            decoded
        });
        let _ = CloseClipboard();
        image
    }
}

/// Reads an uncompressed 24- or 32-bit DIB (with or without the three
/// BI_BITFIELDS masks that some programs add).
pub fn decode_dib(bytes: &[u8]) -> Option<Image> {
    if bytes.len() < 40 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let header_size = u32_at(0) as usize;
    let width = u32_at(4) as i32;
    let height = u32_at(8) as i32;
    let bits = u16::from_le_bytes([bytes[14], bytes[15]]);
    let compression = u32_at(16);
    if width <= 0
        || height == 0
        || width > 32768
        || height.abs() > 32768
        || !(bits == 24 || bits == 32)
    {
        return None;
    }
    let mut offset = header_size;
    if compression == 3 && header_size == 40 {
        offset += 12;
    } else if compression != 0 && compression != 3 {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let src_stride = (w * bits as usize / 8 + 3) & !3;
    if bytes.len() < offset + src_stride * h {
        return None;
    }
    let mut pixels = vec![0u8; w * h * 4];
    for row in 0..h {
        let src_row = if height > 0 { h - 1 - row } else { row };
        let src = &bytes[offset + src_row * src_stride..];
        for x in 0..w {
            let d = (row * w + x) * 4;
            let s = x * bits as usize / 8;
            pixels[d..d + 3].copy_from_slice(&src[s..s + 3]);
            pixels[d + 3] = 255;
        }
    }
    Some(Image {
        width: w as u32,
        height: h as u32,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_round_trip_keeps_rows_in_order() {
        let img = Image {
            width: 2,
            height: 2,
            pixels: vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255],
        };
        let back = decode_dib(&dib_bytes(&img)).unwrap();
        assert_eq!(back.pixels, img.pixels);
    }

    #[test]
    fn truncated_or_odd_dibs_are_refused() {
        let img = Image {
            width: 4,
            height: 4,
            pixels: vec![0; 64],
        };
        let bytes = dib_bytes(&img);
        assert!(decode_dib(&bytes[..bytes.len() - 1]).is_none());
        let mut odd = bytes.clone();
        odd[14] = 8; // 8-bit palette
        assert!(decode_dib(&odd).is_none());
    }
}
