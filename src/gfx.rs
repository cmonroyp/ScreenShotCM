//! Direct2D and DirectWrite helpers, shared by every window that draws.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
    D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_PROPERTIES, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_DASH, D2D1_DASH_STYLE_SOLID,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_OPTIONS, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_LINE_JOIN_ROUND, D2D1_PRESENT_OPTIONS_NONE,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_STROKE_STYLE_PROPERTIES,
    D2D1CreateFactory, ID2D1Bitmap, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1RenderTarget,
    ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_MEASURING_MODE_NATURAL, DWriteCreateFactory, IDWriteFactory,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{Result, w};
use windows_numerics::Vector2;

use crate::image::Image;
use crate::util::Rect;

// The factories are made per window and dropped with it, never cached: a
// Direct2D factory keeps its device and the device keeps screen-sized
// surfaces, so a cached one held 90 MB in the idle agent after one capture.
pub fn d2d() -> Result<ID2D1Factory> {
    unsafe {
        D2D1CreateFactory(
            D2D1_FACTORY_TYPE_SINGLE_THREADED,
            Some(&D2D1_FACTORY_OPTIONS::default()),
        )
    }
}

pub fn dwrite() -> Result<IDWriteFactory> {
    unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
}

/// A render target over a window's client area, in physical pixels: DPI is
/// fixed at 96 so one Direct2D unit is one screen pixel.
pub fn window_target(
    factory: &ID2D1Factory,
    hwnd: HWND,
    width: u32,
    height: u32,
) -> Result<ID2D1HwndRenderTarget> {
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        // Software (WARP): a hardware target loads every GPU vendor's
        // driver into the process — 130 MB on a laptop with two GPUs — and
        // they never unload, so a resident agent would keep them for good.
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        ..Default::default()
    };
    let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
        hwnd,
        pixelSize: D2D_SIZE_U { width, height },
        presentOptions: D2D1_PRESENT_OPTIONS_NONE,
    };
    unsafe { factory.CreateHwndRenderTarget(&props, &hwnd_props) }
}

/// Uploads an opaque image as a bitmap of `target`.
pub fn bitmap(target: &ID2D1RenderTarget, image: &Image) -> Result<ID2D1Bitmap> {
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    unsafe {
        target.CreateBitmap(
            D2D_SIZE_U {
                width: image.width,
                height: image.height,
            },
            Some(image.pixels.as_ptr() as *const _),
            image.stride() as u32,
            &props,
        )
    }
}

pub fn text_format(size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
    text_format_in(&dwrite()?, size, weight)
}

/// `text_format` from a factory the caller already holds.
pub fn text_format_in(
    factory: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat> {
    unsafe {
        factory.CreateTextFormat(
            w!("Segoe UI"),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!(""),
        )
    }
}

pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

pub fn rect(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x as f32,
        top: r.y as f32,
        right: r.right() as f32,
        bottom: r.bottom() as f32,
    }
}

pub fn rectf(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

pub fn point(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// The brand blue, the mark's own (`icon.rs` draws it): what frames a
/// selection on the screen. Buttons take `theme::ACCENT`, the system-blue
/// accent the editor's «Listo» and every other button share.
pub const BRAND: (u8, u8, u8) = (37, 99, 235);

/// Round caps and joins, the stroke every line, outline and icon is drawn
/// with; dashed for the crop and selection frames.
pub fn round_stroke(factory: &ID2D1Factory, dashed: bool) -> Result<ID2D1StrokeStyle> {
    unsafe {
        factory.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: if dashed {
                    D2D1_DASH_STYLE_DASH
                } else {
                    D2D1_DASH_STYLE_SOLID
                },
                dashOffset: 0.0,
            },
            None,
        )
    }
}

/// Draws `text` (UTF-16) in `r` with `format`, in the brush's colour.
pub fn draw_text(
    t: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    format: &IDWriteTextFormat,
    text: &[u16],
    r: &D2D_RECT_F,
) {
    unsafe {
        t.DrawText(
            text,
            format,
            r,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
}

/// `draw_text` for a `&str`.
pub fn draw_str(
    t: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    format: &IDWriteTextFormat,
    text: &str,
    r: &D2D_RECT_F,
) {
    let units: Vec<u16> = text.encode_utf16().collect();
    draw_text(t, brush, format, &units, r);
}
