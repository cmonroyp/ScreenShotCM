//! The mark: a viewfinder — four white corner brackets around a dot — on a
//! rounded blue square. Drawn here rather than shipped as a resource, so the
//! tray gets a crisp icon at whatever size the monitor's DPI asks for, and
//! the installer's `.ico` is written from the same drawing.

use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

/// `gfx::BRAND`, as floats for the shading.
const BLUE: [f32; 3] = [
    crate::gfx::BRAND.0 as f32,
    crate::gfx::BRAND.1 as f32,
    crate::gfx::BRAND.2 as f32,
];
const BLUE_DARK: [f32; 3] = [29.0, 78.0, 216.0];

/// Signed distance from `p` to a rounded box centred at `c`.
fn rounded_box(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hw - r);
    let qy = (py - cy).abs() - (hh - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

fn coverage(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// Straight-alpha BGRA pixels, top-down, `size` × `size`.
pub fn render(size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let c = s / 2.0;
    let stroke = (s * 0.085).max(1.0);
    let arm = s * 0.2;
    let inner = s * 0.27; // half-size of the viewfinder
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let bg = coverage(rounded_box(px, py, c, c, c, c, s * 0.22));
            if bg <= 0.0 {
                continue;
            }
            // A slight vertical gradient keeps the square from looking flat.
            let t = py / s;
            let mut rgb = [0f32; 3];
            for i in 0..3 {
                rgb[i] = BLUE[i] * (1.0 - t) + BLUE_DARK[i] * t;
            }
            // Corner brackets: an L at each corner of the viewfinder.
            let mut white: f32 = 0.0;
            for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let ox = c + sx * inner;
                let oy = c + sy * inner;
                // Horizontal arm, pointing inwards.
                let hx = ox - sx * arm / 2.0;
                let h = rounded_box(px, py, hx, oy, arm / 2.0, stroke / 2.0, stroke / 2.0);
                let vy = oy - sy * arm / 2.0;
                let v = rounded_box(px, py, ox, vy, stroke / 2.0, arm / 2.0, stroke / 2.0);
                white = white.max(coverage(h.min(v)));
            }
            let dot = ((px - c).powi(2) + (py - c).powi(2)).sqrt() - s * 0.085;
            white = white.max(coverage(dot));
            for ch in &mut rgb {
                *ch = *ch * (1.0 - white) + 255.0 * white;
            }
            let i = ((y * size + x) * 4) as usize;
            out[i] = rgb[2].round() as u8;
            out[i + 1] = rgb[1].round() as u8;
            out[i + 2] = rgb[0].round() as u8;
            out[i + 3] = (bg * 255.0).round() as u8;
        }
    }
    out
}

/// The 32-pixel icon window classes register with, made once and kept for
/// the process: a class is registered again on every open of its window,
/// only the first registration keeps its icon, and a fresh HICON each time
/// was a GDI handle lost per opening.
pub fn class_icon() -> HICON {
    thread_local! {
        static ICON: std::cell::OnceCell<HICON> = const { std::cell::OnceCell::new() };
    }
    ICON.with(|i| *i.get_or_init(|| create(32).unwrap_or_default()))
}

/// An HICON of `size` pixels; the caller destroys it.
pub fn create(size: u32) -> windows::core::Result<HICON> {
    let pixels = render(size);
    unsafe {
        let color = CreateBitmap(
            size as i32,
            size as i32,
            1,
            32,
            Some(pixels.as_ptr() as *const _),
        );
        // The mask is ignored for a 32-bit icon with alpha, but must exist.
        let mask_bits = vec![0u8; (size.div_ceil(16) * 2 * size) as usize];
        let mask = CreateBitmap(
            size as i32,
            size as i32,
            1,
            1,
            Some(mask_bits.as_ptr() as *const _),
        );
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

/// A Windows `.ico` holding the mark at the sizes Explorer and the installer
/// ask for: 32-bit DIBs, and the 256-pixel one as PNG, as Windows' own icons
/// store it (a DIB that size is 260 KB of the executable).
pub fn ico_file() -> Vec<u8> {
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 256];
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            if s >= 256 {
                png_entry(s).unwrap_or_else(|| dib_entry(s))
            } else {
                dib_entry(s)
            }
        })
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

/// The tiles and logos an MSIX package declares, written as PNGs into `dir`:
/// the mark on transparency, the wide tile with the mark centred.
pub fn export_store_assets(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let square = |name: &str, size: u32| -> Result<(), String> {
        let image = crate::image::Image {
            width: size,
            height: size,
            pixels: render(size),
        };
        crate::image::save_png_with_alpha(&image, &dir.join(name)).map_err(|e| e.to_string())
    };
    square("Square44x44Logo.png", 44)?;
    square("Square150x150Logo.png", 150)?;
    square("StoreLogo.png", 50)?;
    // The wide tile: the mark, two thirds of the height, on transparency.
    let (w, h, mark) = (310u32, 150u32, 100u32);
    let icon = render(mark);
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let (ox, oy) = ((w - mark) / 2, (h - mark) / 2);
    for y in 0..mark {
        let src = (y * mark * 4) as usize;
        let dst = (((y + oy) * w + ox) * 4) as usize;
        pixels[dst..dst + (mark * 4) as usize]
            .copy_from_slice(&icon[src..src + (mark * 4) as usize]);
    }
    let wide = crate::image::Image {
        width: w,
        height: h,
        pixels,
    };
    crate::image::save_png_with_alpha(&wide, &dir.join("Wide310x150Logo.png"))
        .map_err(|e| e.to_string())
}

fn png_entry(size: u32) -> Option<Vec<u8>> {
    let image = crate::image::Image {
        width: size,
        height: size,
        pixels: render(size),
    };
    let tmp = std::env::temp_dir().join(format!("screenshotcm-icon-{}.png", std::process::id()));
    crate::image::save_png_with_alpha(&image, &tmp).ok()?;
    let bytes = std::fs::read(&tmp).ok();
    let _ = std::fs::remove_file(&tmp);
    bytes
}

fn dib_entry(size: u32) -> Vec<u8> {
    let pixels = render(size);
    let mask_stride = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    // BITMAPINFOHEADER; the height counts the colour and the mask rows.
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    // Bottom-up rows.
    for y in (0..size).rev() {
        let row = (y * size * 4) as usize;
        out.extend_from_slice(&pixels[row..row + (size * 4) as usize]);
    }
    out.extend(std::iter::repeat_n(0u8, (mask_stride * size) as usize));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_are_transparent_and_centre_is_white() {
        let s = 32;
        let px = render(s);
        assert_eq!(px[3], 0, "top-left corner is outside the rounded square");
        let centre = (((s / 2) * s + s / 2) * 4) as usize;
        assert!(
            px[centre] > 240 && px[centre + 3] == 255,
            "the dot is white and opaque"
        );
    }

    #[test]
    fn ico_directory_points_at_each_image() {
        // The 256-pixel entry goes through WIC, as it does for the installer:
        // without COM on this thread it fell back to a DIB, or crashed when
        // it borrowed another test's apartment.
        crate::image::tests::com_for_test();
        let ico = ico_file();
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        let mut end = 0;
        for i in 0..count {
            let e = 6 + 16 * i;
            let size = ico[e];
            let len = u32::from_le_bytes(ico[e + 8..e + 12].try_into().unwrap()) as usize;
            let off = u32::from_le_bytes(ico[e + 12..e + 16].try_into().unwrap()) as usize;
            if size == 0 {
                assert_eq!(&ico[off..off + 4], b"\x89PNG", "256 pixels: PNG");
            } else {
                let header = u32::from_le_bytes(ico[off..off + 4].try_into().unwrap());
                assert_eq!(header, 40, "{size} pixels: DIB");
            }
            end = end.max(off + len);
        }
        assert_eq!(end, ico.len());
    }
}
