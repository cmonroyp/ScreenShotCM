//! Drawing into memory instead of a window: the export of an annotated
//! capture, and the surfaces of the layered (per-pixel alpha) windows.

use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, ID2D1Factory,
    ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad, WICBitmapLockRead, WICRect,
};
use windows::core::Result;

use crate::gfx::rgba;
use crate::image::{self, Image};

/// Renders `draw` into a `w` × `h` image. Pixels are returned straight
/// (not premultiplied) BGRA.
pub fn offscreen(
    factory: &ID2D1Factory,
    w: u32,
    h: u32,
    draw: impl FnOnce(&ID2D1RenderTarget, &ID2D1SolidColorBrush),
) -> Result<Image> {
    unsafe {
        let wic = image::wic()?;
        let bitmap =
            wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let target = factory.CreateWicBitmapRenderTarget(&bitmap, &props)?;
        let brush = target.CreateSolidColorBrush(&rgba(0, 0, 0, 1.0), None)?;
        target.BeginDraw();
        draw(&target, &brush);
        target.EndDraw(None, None)?;
        drop(brush);
        drop(target);

        let rect = WICRect {
            X: 0,
            Y: 0,
            Width: w as i32,
            Height: h as i32,
        };
        let lock = bitmap.Lock(&rect, WICBitmapLockRead.0 as u32)?;
        let mut size = 0u32;
        let mut ptr = std::ptr::null_mut();
        lock.GetDataPointer(&mut size, &mut ptr)?;
        let stride = lock.GetStride()? as usize;
        let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h as usize {
            let row = std::slice::from_raw_parts(ptr.add(y * stride), w as usize * 4);
            pixels.extend_from_slice(row);
        }
        unpremultiply(&mut pixels);
        Ok(Image {
            width: w,
            height: h,
            pixels,
        })
    }
}

pub fn unpremultiply(pixels: &mut [u8]) {
    for px in pixels.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpremultiply_restores_colour_and_leaves_extremes() {
        let mut px = vec![50, 100, 0, 128, 9, 9, 9, 0, 1, 2, 3, 255];
        unpremultiply(&mut px);
        assert_eq!(&px[..4], &[100, 199, 0, 128]);
        assert_eq!(&px[4..], &[9, 9, 9, 0, 1, 2, 3, 255]);
    }
}
