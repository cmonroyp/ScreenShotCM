//! Pixels and files. An `Image` is straight BGRA, top-down — the layout GDI,
//! Direct2D and WIC all speak, so nothing is converted on the way through.
//! Encoding is Windows' own WIC: no codec ships with us.

use std::path::Path;

use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_ContainerFormatPng,
    GUID_WICPixelFormat24bppBGR, GUID_WICPixelFormat32bppBGRA, IWICBitmapFrameEncode,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::{IPropertyBag2, PROPBAG2};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{PCWSTR, PWSTR};

use crate::settings::ImageFormat;
use crate::util::{Rect, wide};

/// The largest image the app opens, makes or takes from the clipboard: a
/// file, a scrolling capture and a pasted bitmap share one bound, so what
/// one part makes another can open again. A few hundred bytes of PNG can
/// declare 20000×20000 — 1.6 GB to allocate; 100 megapixels (400 MB) still
/// holds three 8K screens side by side.
pub const MAX_SIDE: u32 = 20_000;
pub const MAX_PIXELS: u64 = 100_000_000;

/// Whether an image of `w`×`h` is within `MAX_SIDE` and `MAX_PIXELS`.
pub fn within_bounds(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE && w as u64 * h as u64 <= MAX_PIXELS
}

#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }

    /// A copy of `r`, which must lie inside the image.
    pub fn crop(&self, r: Rect) -> Image {
        let r = r.intersect(&Rect::new(0, 0, self.width as i32, self.height as i32));
        let mut pixels = Vec::with_capacity(r.w as usize * r.h as usize * 4);
        for y in r.y..r.bottom() {
            let start = y as usize * self.stride() + r.x as usize * 4;
            pixels.extend_from_slice(&self.pixels[start..start + r.w as usize * 4]);
        }
        Image {
            width: r.w as u32,
            height: r.h as u32,
            pixels,
        }
    }

    /// BGR without alpha, which both PNG and JPEG take as is: a screenshot
    /// is opaque, and three bytes a pixel is a smaller file.
    fn bgr(&self) -> Vec<u8> {
        let stride = (self.width as usize * 3 + 3) & !3;
        let mut out = vec![0u8; stride * self.height as usize];
        for y in 0..self.height as usize {
            let src = &self.pixels[y * self.stride()..(y + 1) * self.stride()];
            let dst = &mut out[y * stride..y * stride + self.width as usize * 3];
            for (d, s) in dst
                .as_chunks_mut::<3>()
                .0
                .iter_mut()
                .zip(src.as_chunks::<4>().0)
            {
                d.copy_from_slice(&s[..3]);
            }
        }
        out
    }
}

pub fn wic() -> windows::core::Result<IWICImagingFactory> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
}

/// Writes `image` to `path` through a temporary file, so a reader never sees
/// half a file.
pub fn save(
    image: &Image,
    path: &Path,
    format: ImageFormat,
    jpeg_quality: u8,
) -> windows::core::Result<()> {
    // Unique per write: two writers of one path (the history worker and
    // a card that needs the file now) never share a temporary file.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}-{n}.partial", std::process::id()));
    let result = encode(image, &tmp, format, jpeg_quality, false);
    match result {
        Ok(()) => std::fs::rename(&tmp, path).map_err(|e| {
            windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), e.to_string())
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Like `save`, but only if nothing is at `path` when the file is ready:
/// the history worker encodes for seconds, and in that time the editor's
/// «Listo» may have written the annotated version there. Returns whether it
/// wrote. A hard link publishes without replacing; where links are not
/// supported (FAT), the check and the rename are as close as they can be.
pub fn save_new(image: &Image, path: &Path) -> windows::core::Result<bool> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}-new{n}.partial", std::process::id()));
    if let Err(e) = encode(image, &tmp, ImageFormat::Png, 100, false) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let wrote = match std::fs::hard_link(&tmp, path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(_) if path.exists() => false,
        Err(_) => std::fs::rename(&tmp, path).is_ok(),
    };
    let _ = std::fs::remove_file(&tmp);
    Ok(wrote)
}

/// Reads any image Windows can decode (PNG, JPEG, BMP, GIF, TIFF, …) as BGRA.
/// Images larger than the shared page bound are refused before a pixel is
/// allocated.
pub fn load(path: &Path) -> windows::core::Result<Image> {
    unsafe {
        let factory = wic()?;
        let p = wide(path);
        let decoder = factory.CreateDecoderFromFilename(
            PCWSTR(p.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let (mut w, mut h) = (0u32, 0u32);
        converter.GetSize(&mut w, &mut h)?;
        if !within_bounds(w, h) {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80070057u32 as i32),
                crate::i18n::trs(
                    format!("imagen de {w}×{h}: demasiado grande o vacía"),
                    format!("{w}×{h} image: too large or empty"),
                ),
            ));
        }
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        converter.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
        Ok(Image {
            width: w,
            height: h,
            pixels,
        })
    }
}

/// Decodes `path` already scaled down to fit `max_w` × `max_h` (WIC's Fant
/// scaler reads only what it needs), for history thumbnails.
pub fn load_thumbnail(path: &Path, max_w: u32, max_h: u32) -> windows::core::Result<Image> {
    unsafe {
        let factory = wic()?;
        let p = wide(path);
        let decoder = factory.CreateDecoderFromFilename(
            PCWSTR(p.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h)?;
        let k = (max_w as f32 / w.max(1) as f32)
            .min(max_h as f32 / h.max(1) as f32)
            .min(1.0);
        let (tw, th) = (
            ((w as f32 * k).round() as u32).max(1),
            ((h as f32 * k).round() as u32).max(1),
        );
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&frame, tw, th, WICBitmapInterpolationModeFant)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let mut pixels = vec![0u8; tw as usize * th as usize * 4];
        converter.CopyPixels(std::ptr::null(), tw * 4, &mut pixels)?;
        Ok(Image {
            width: tw,
            height: th,
            pixels,
        })
    }
}

/// A PNG that keeps the alpha channel, for icons.
pub fn save_png_with_alpha(image: &Image, path: &Path) -> windows::core::Result<()> {
    encode(image, path, ImageFormat::Png, 100, true)
}

/// Writes `image`; without `alpha` the pixels go as 24bpp, which every
/// Windows PNG and JPEG encoder takes and which drops the unused channel.
fn encode(
    image: &Image,
    path: &Path,
    format: ImageFormat,
    jpeg_quality: u8,
    alpha: bool,
) -> windows::core::Result<()> {
    unsafe {
        let factory = wic()?;
        let container = match format {
            ImageFormat::Png => GUID_ContainerFormatPng,
            ImageFormat::Jpeg => GUID_ContainerFormatJpeg,
        };
        let stream = factory.CreateStream()?;
        let p = wide(path);
        stream.InitializeFromFilename(PCWSTR(p.as_ptr()), GENERIC_WRITE.0)?;
        let encoder = factory.CreateEncoder(&container, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;

        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut props: Option<IPropertyBag2> = None;
        encoder.CreateNewFrame(&mut frame, &mut props)?;
        let frame = frame.ok_or_else(windows::core::Error::empty)?;
        if let (ImageFormat::Jpeg, Some(bag)) = (format, &props) {
            let mut name = wide("ImageQuality");
            let option = PROPBAG2 {
                pstrName: PWSTR(name.as_mut_ptr()),
                ..Default::default()
            };
            let value = VARIANT::from(jpeg_quality as f32 / 100.0);
            bag.Write(1, &option, &value)?;
        }
        frame.Initialize(props.as_ref())?;
        frame.SetSize(image.width, image.height)?;
        let mut pixel_format = if alpha {
            GUID_WICPixelFormat32bppBGRA
        } else {
            GUID_WICPixelFormat24bppBGR
        };
        frame.SetPixelFormat(&mut pixel_format)?;
        if pixel_format == GUID_WICPixelFormat24bppBGR {
            let bgr = image.bgr();
            let stride = bgr.len() as u32 / image.height.max(1);
            frame.WritePixels(image.height, stride, &bgr)?;
        } else {
            frame.WritePixels(image.height, image.stride() as u32, &image.pixels)?;
        }
        frame.Commit()?;
        encoder.Commit()?;
        stream.Commit(windows::Win32::System::Com::STGC_DEFAULT)?;
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Image {
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                pixels.extend_from_slice(&[x as u8, y as u8, (x ^ y) as u8, 255]);
            }
        }
        Image {
            width: w,
            height: h,
            pixels,
        }
    }

    /// Joins this test's thread to COM's multithreaded apartment. Every test
    /// that reaches WIC calls it on its own thread: one that leaned on another
    /// test's apartment crashed (0xC0000005) when that test finished first.
    pub fn com_for_test() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
    }

    #[test]
    fn crop_takes_exactly_the_rectangle() {
        let img = gradient(10, 8);
        let c = img.crop(Rect::new(3, 2, 4, 5));
        assert_eq!((c.width, c.height), (4, 5));
        assert_eq!(&c.pixels[..4], &[3, 2, 3 ^ 2, 255]);
        let last = c.pixels.len() - 4;
        assert_eq!(&c.pixels[last..], &[6, 6, 0, 255]);
    }

    #[test]
    fn crop_is_clamped_to_the_image() {
        let c = gradient(10, 8).crop(Rect::new(8, 6, 10, 10));
        assert_eq!((c.width, c.height), (2, 2));
    }

    #[test]
    fn bgr_rows_are_padded_to_four_bytes() {
        let img = gradient(3, 2); // 9 bytes a row, padded to 12
        let bgr = img.bgr();
        assert_eq!(bgr.len(), 24);
        assert_eq!(&bgr[12..15], &[0, 1, 1]);
    }

    #[test]
    fn png_and_jpeg_are_written_whole() {
        com_for_test();
        let dir = std::env::temp_dir().join(format!("scm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = gradient(37, 21);
        for (fmt, magic) in [
            (ImageFormat::Png, &b"\x89PNG"[..]),
            (ImageFormat::Jpeg, &b"\xFF\xD8\xFF"[..]),
        ] {
            let path = dir.join(format!("t.{}", fmt.extension()));
            save(&img, &path, fmt, 80).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert!(bytes.starts_with(magic), "{fmt:?}");
            let leftovers = std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "partial"))
                .count();
            assert_eq!(leftovers, 0, "no temporary file is left behind");
            // And what was written reads back, pixel for pixel for PNG.
            let back = load(&path).unwrap();
            assert_eq!((back.width, back.height), (37, 21));
            if fmt == ImageFormat::Png {
                assert_eq!(back.pixels, img.pixels);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
