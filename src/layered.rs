//! Windows with per-pixel alpha (`UpdateLayeredWindow`): real rounded corners
//! and soft shadows over whatever is behind, for the quick-access cards and
//! pinned captures. Direct2D draws into a DIB the window then shows.

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, ID2D1DCRenderTarget,
    ID2D1Factory, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC,
    HGDIOBJ, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::Result;

use crate::gfx::rgba;

pub struct Surface {
    pub factory: ID2D1Factory,
    pub width: i32,
    pub height: i32,
    dc: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
    target: ID2D1DCRenderTarget,
    pub brush: ID2D1SolidColorBrush,
}

impl Surface {
    pub fn new(factory: ID2D1Factory, width: i32, height: i32) -> Result<Self> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let dib = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(d) => d,
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(e);
                }
            };
            let old = SelectObject(dc, dib.into());
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
            let target = factory.CreateDCRenderTarget(&props)?;
            target.BindDC(
                dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                },
            )?;
            let brush = target.CreateSolidColorBrush(&rgba(0, 0, 0, 1.0), None)?;
            Ok(Self {
                factory,
                width,
                height,
                dc,
                dib,
                old,
                target,
                brush,
            })
        }
    }

    /// The tallest or widest bitmap this software target can hold — the same
    /// rasteriser the editor and the pins draw with.
    pub fn max_bitmap_size(&self) -> u32 {
        unsafe { self.target.GetMaximumBitmapSize() }
    }

    /// Clears to transparent and runs `paint`.
    pub fn draw(
        &self,
        paint: impl FnOnce(&ID2D1RenderTarget, &ID2D1SolidColorBrush),
    ) -> Result<()> {
        unsafe {
            self.target.BeginDraw();
            self.target.Clear(Some(&rgba(0, 0, 0, 0.0)));
            paint(&self.target, &self.brush);
            self.target.EndDraw(None, None)
        }
    }

    /// Shows the surface as `hwnd`'s whole content, its top-left at `at`.
    pub fn present(&self, hwnd: HWND, at: (i32, i32), opacity: f32) -> Result<()> {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&POINT { x: at.0, y: at.1 }),
                Some(&SIZE {
                    cx: self.width,
                    cy: self.height,
                }),
                Some(self.dc),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.dib.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// A box-filtered copy of `image` at most `max_w` × `max_h`, for thumbnails:
/// cheaper to draw every frame than the full capture scaled by Direct2D.
pub fn thumbnail(image: &crate::image::Image, max_w: u32, max_h: u32) -> crate::image::Image {
    let k = (max_w as f32 / image.width as f32)
        .min(max_h as f32 / image.height as f32)
        .min(1.0);
    let w = ((image.width as f32 * k).round() as u32).max(1);
    let h = ((image.height as f32 * k).round() as u32).max(1);
    if w == image.width && h == image.height {
        return image.clone();
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    let sx = image.width as f32 / w as f32;
    let sy = image.height as f32 / h as f32;
    for y in 0..h {
        let y0 = (y as f32 * sy) as u32;
        let y1 = (((y + 1) as f32 * sy) as u32).clamp(y0 + 1, image.height);
        for x in 0..w {
            let x0 = (x as f32 * sx) as u32;
            let x1 = (((x + 1) as f32 * sx) as u32).clamp(x0 + 1, image.width);
            let mut sum = [0u32; 4];
            for yy in y0..y1 {
                let row = yy as usize * image.stride();
                for xx in x0..x1 {
                    let i = row + xx as usize * 4;
                    for c in 0..4 {
                        sum[c] += image.pixels[i + c] as u32;
                    }
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            let o = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (sum[c] / n) as u8;
            }
        }
    }
    crate::image::Image {
        width: w,
        height: h,
        pixels: out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Image;

    #[test]
    fn thumbnail_keeps_the_aspect_and_averages() {
        let img = Image {
            width: 4,
            height: 2,
            pixels: [[0u8, 0, 0, 255], [200, 200, 200, 255]].concat().repeat(4),
        };
        let t = thumbnail(&img, 2, 2);
        assert_eq!((t.width, t.height), (2, 1));
        assert_eq!(t.pixels[0], 100);
    }

    #[test]
    fn a_small_image_is_not_enlarged() {
        let img = Image {
            width: 3,
            height: 3,
            pixels: vec![7; 36],
        };
        let t = thumbnail(&img, 100, 100);
        assert_eq!((t.width, t.height), (3, 3));
    }
}
