//! Pixels and files. An `Image` is straight BGRA, top-down — the layout GDI,
//! Direct2D and WIC all speak, so nothing is converted on the way through.
//! Encoding is Windows' own WIC: no codec ships with us.

use std::path::Path;

use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_ContainerFormatPng,
    GUID_WICPixelFormat24bppBGR, GUID_WICPixelFormat32bppBGRA, IWICBitmapFrameEncode,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeFant, WICBitmapInterpolationModeHighQualityCubic,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
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

/// The brightness of a BGR(A) pixel, 0 to 255, weighted as the eye does.
pub fn luma(px: &[u8]) -> u32 {
    (px[2] as u32 * 77 + px[1] as u32 * 150 + px[0] as u32 * 29) >> 8
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

    /// A copy turned a quarter: clockwise, or counter-clockwise. The pixel
    /// at (x, y) goes to (h-1-y, x) clockwise and to (y, w-1-x) the other
    /// way.
    pub fn rotated(&self, clockwise: bool) -> Image {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut pixels = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (nx, ny) = if clockwise {
                    (h - 1 - y, x)
                } else {
                    (y, w - 1 - x)
                };
                let src = (y * w + x) * 4;
                let dst = (ny * h + nx) * 4;
                pixels[dst..dst + 4].copy_from_slice(&self.pixels[src..src + 4]);
            }
        }
        Image {
            width: self.height,
            height: self.width,
            pixels,
        }
    }

    /// The sharpest edge near `at`, for a crop's side to snap to: within
    /// `reach` pixels, the column (`vertical`) or row where the picture
    /// changes most between neighbours over `span` — the stretch of the
    /// other axis the side covers. `None` where nothing there is a clear
    /// edge (a mean step under 24 of 255), so a side over a flat area or a
    /// photo moves freely. The picture's own border always counts.
    pub fn edge_near(&self, vertical: bool, at: f32, span: (f32, f32), reach: f32) -> Option<f32> {
        let (len, across) = if vertical {
            (self.width, self.height)
        } else {
            (self.height, self.width)
        };
        if at.abs() <= reach {
            return Some(0.0);
        }
        if (len as f32 - at).abs() <= reach {
            return Some(len as f32);
        }
        let lo = (at - reach).floor().max(1.0) as u32;
        let hi = (at + reach).ceil().min(len as f32 - 1.0) as u32;
        let s0 = span.0.min(span.1).max(0.0) as u32;
        let s1 = (span.0.max(span.1) as u32).min(across);
        if hi < lo || s1 <= s0 {
            return None;
        }
        // Up to 400 samples along the side: a tall capture costs no more.
        let step = ((s1 - s0) / 400).max(1);
        let px = |i: u32, j: u32| {
            let (x, y) = if vertical { (i, j) } else { (j, i) };
            let o = (y as usize * self.width as usize + x as usize) * 4;
            luma(&self.pixels[o..o + 3]) as i32
        };
        let (mut best, mut best_at) = (0u64, 0u32);
        for i in lo..=hi {
            let (mut sum, mut n) = (0u64, 0u64);
            let mut j = s0;
            while j < s1 {
                sum += (px(i, j) - px(i - 1, j)).unsigned_abs() as u64;
                n += 1;
                j += step;
            }
            let strength = sum / n.max(1);
            if strength > best {
                (best, best_at) = (strength, i);
            }
        }
        (best >= 24).then_some(best_at as f32)
    }

    /// A mirror copy: left for right, or top for bottom.
    pub fn flipped(&self, horizontal: bool) -> Image {
        let (w, h) = (self.width as usize, self.height as usize);
        let stride = w * 4;
        let mut pixels = vec![0u8; w * h * 4];
        for y in 0..h {
            let src = &self.pixels[y * stride..(y + 1) * stride];
            if horizontal {
                let row = &mut pixels[y * stride..(y + 1) * stride];
                for x in 0..w {
                    row[x * 4..x * 4 + 4].copy_from_slice(&src[(w - 1 - x) * 4..(w - x) * 4]);
                }
            } else {
                let ny = h - 1 - y;
                pixels[ny * stride..(ny + 1) * stride].copy_from_slice(src);
            }
        }
        Image {
            width: self.width,
            height: self.height,
            pixels,
        }
    }

    /// A copy scaled to `w`×`h` by WIC's high-quality cubic, both ways: it
    /// takes every pixel into account when shrinking, as Fant did, and keeps
    /// text edges crisper than Fant's averaging.
    pub fn resized(&self, w: u32, h: u32) -> windows::core::Result<Image> {
        if !within_bounds(w, h) {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80070057u32 as i32),
                crate::i18n::trs(
                    format!("imagen de {w}×{h}: demasiado grande o vacía"),
                    format!("{w}×{h} image: too large or empty"),
                ),
            ));
        }
        unsafe {
            let factory = wic()?;
            let source = factory.CreateBitmapFromMemory(
                self.width,
                self.height,
                &GUID_WICPixelFormat32bppBGRA,
                self.stride() as u32,
                &self.pixels,
            )?;
            let scaler = factory.CreateBitmapScaler()?;
            scaler.Initialize(&source, w, h, WICBitmapInterpolationModeHighQualityCubic)?;
            let mut pixels = vec![0u8; w as usize * h as usize * 4];
            scaler.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
            Ok(Image {
                width: w,
                height: h,
                pixels,
            })
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
    fn a_crop_side_finds_a_sharp_edge_and_ignores_a_flat_area() {
        // A dark panel from column 50 and row 40 on a light page.
        let (w, h) = (120u32, 90u32);
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = if x >= 50 && y >= 40 { 30 } else { 230 };
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let img = Image {
            width: w,
            height: h,
            pixels,
        };
        assert_eq!(img.edge_near(true, 46.0, (40.0, 90.0), 6.0), Some(50.0));
        assert_eq!(img.edge_near(false, 43.5, (50.0, 120.0), 6.0), Some(40.0));
        // Above the panel the page is flat: nothing to hold the side.
        assert_eq!(img.edge_near(true, 46.0, (0.0, 30.0), 6.0), None);
        // The border itself always counts.
        assert_eq!(img.edge_near(true, 3.0, (0.0, 90.0), 6.0), Some(0.0));
        assert_eq!(img.edge_near(false, 87.0, (0.0, 120.0), 6.0), Some(90.0));
    }

    #[test]
    fn turning_and_mirroring_move_each_pixel_where_it_belongs() {
        // gradient(w, h) puts (x, y) in a pixel's first two bytes.
        let img = gradient(5, 3);
        let at = |i: &Image, x: u32, y: u32| {
            let o = ((y * i.width + x) * 4) as usize;
            (i.pixels[o], i.pixels[o + 1])
        };
        let cw = img.rotated(true);
        assert_eq!((cw.width, cw.height), (3, 5));
        // (x, y) → (h-1-y, x): the top-left corner goes top-right.
        assert_eq!(at(&cw, 2, 0), (0, 0));
        assert_eq!(at(&cw, 0, 4), (4, 2));
        let ccw = img.rotated(false);
        assert_eq!(at(&ccw, 0, 4), (0, 0));
        assert_eq!(ccw.rotated(true).pixels, img.pixels, "a turn and back");
        let h = img.flipped(true);
        assert_eq!(at(&h, 4, 1), (0, 1));
        let v = img.flipped(false);
        assert_eq!(at(&v, 3, 2), (3, 0));
        assert_eq!(v.flipped(false).pixels, img.pixels, "twice is nothing");
    }

    #[test]
    fn resized_keeps_a_flat_colour_and_takes_the_new_size() {
        com_for_test();
        let flat = Image {
            width: 40,
            height: 30,
            pixels: [10u8, 120, 200, 255].repeat(40 * 30),
        };
        for (w, h) in [(20, 15), (100, 75)] {
            let r = flat.resized(w, h).unwrap();
            assert_eq!(
                (r.width, r.height, r.pixels.len()),
                (w, h, (w * h * 4) as usize)
            );
            assert!(r.pixels.chunks(4).all(|p| p == [10, 120, 200, 255]));
        }
        assert!(flat.resized(0, 10).is_err());
        assert!(flat.resized(MAX_SIDE + 1, 10).is_err());
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
