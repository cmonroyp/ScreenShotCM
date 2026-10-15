//! Toolbar icons, drawn as vectors on a 20×20 grid: crisp at every scale and
//! independent of which icon font a Windows version happens to ship.

use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW,
    D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ELLIPSE, D2D1_ROUNDED_RECT, ID2D1Factory, ID2D1RenderTarget, ID2D1SolidColorBrush,
    ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    IDWriteFactory,
};

use crate::gfx::{point, rectf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Crop,
    Background,
    Select,
    Rect,
    FilledRect,
    Ellipse,
    Line,
    Arrow,
    Text,
    Pixelate,
    Spotlight,
    Counter,
    Pen,
    Highlighter,
    Pin,
    Copy,
    Save,
    Close,
    Minimize,
    Maximize,
    Trash,
    Chevron,
    Grip,
    Undo,
    Redo,
    RotateLeft,
    RotateRight,
    FlipH,
    FlipV,
    Magnet,
    Drop,
    DropLock,
    Check,
}

/// A step of an outline on the 20×20 grid: a straight line, or a cubic
/// curve through two control points.
#[derive(Clone, Copy)]
enum Seg {
    L(f32, f32),
    C(f32, f32, f32, f32, f32, f32),
}

pub struct Icons<'a> {
    pub factory: &'a ID2D1Factory,
    pub dwrite: &'a IDWriteFactory,
    pub round: &'a ID2D1StrokeStyle,
}

impl Icons<'_> {
    /// Draws `icon` centred at (`cx`, `cy`), `size` pixels across.
    pub fn draw(
        &self,
        t: &ID2D1RenderTarget,
        brush: &ID2D1SolidColorBrush,
        icon: Icon,
        cx: f32,
        cy: f32,
        size: f32,
        color: D2D1_COLOR_F,
    ) {
        let k = size / 20.0;
        let p = |x: f32, y: f32| point(cx + (x - 10.0) * k, cy + (y - 10.0) * k);
        let sw = 1.6 * k;
        unsafe {
            brush.SetColor(&color);
            let line = |a: (f32, f32), b: (f32, f32)| {
                t.DrawLine(p(a.0, a.1), p(b.0, b.1), brush, sw, self.round)
            };
            let poly = |pts: &[(f32, f32)], fill: bool, close: bool| {
                let Ok(geo) = self.factory.CreatePathGeometry() else {
                    return;
                };
                let Ok(sink) = geo.Open() else { return };
                sink.BeginFigure(
                    p(pts[0].0, pts[0].1),
                    if fill {
                        D2D1_FIGURE_BEGIN_FILLED
                    } else {
                        D2D1_FIGURE_BEGIN_HOLLOW
                    },
                );
                let rest: Vec<_> = pts[1..].iter().map(|q| p(q.0, q.1)).collect();
                sink.AddLines(&rest);
                sink.EndFigure(if close {
                    D2D1_FIGURE_END_CLOSED
                } else {
                    D2D1_FIGURE_END_OPEN
                });
                let _ = sink.Close();
                if fill {
                    t.FillGeometry(&geo, brush, None);
                } else {
                    t.DrawGeometry(&geo, brush, sw, self.round);
                }
            };
            // An open stroked outline of lines and curves.
            let path = |start: (f32, f32), segs: &[Seg]| {
                let Ok(geo) = self.factory.CreatePathGeometry() else {
                    return;
                };
                let Ok(sink) = geo.Open() else { return };
                sink.BeginFigure(p(start.0, start.1), D2D1_FIGURE_BEGIN_HOLLOW);
                for s in segs {
                    match *s {
                        Seg::L(x, y) => sink.AddLine(p(x, y)),
                        Seg::C(x1, y1, x2, y2, x, y) => sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                            point1: p(x1, y1),
                            point2: p(x2, y2),
                            point3: p(x, y),
                        }),
                    }
                }
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                let _ = sink.Close();
                t.DrawGeometry(&geo, brush, sw, self.round);
            };
            let rrect = |x: f32, y: f32, w: f32, h: f32, r: f32, fill: bool| {
                let rr = D2D1_ROUNDED_RECT {
                    rect: rectf(cx + (x - 10.0) * k, cy + (y - 10.0) * k, w * k, h * k),
                    radiusX: r * k,
                    radiusY: r * k,
                };
                if fill {
                    t.FillRoundedRectangle(&rr, brush)
                } else {
                    t.DrawRoundedRectangle(&rr, brush, sw, self.round)
                }
            };
            let circle = |x: f32, y: f32, r: f32, fill: bool| {
                let e = D2D1_ELLIPSE {
                    point: p(x, y),
                    radiusX: r * k,
                    radiusY: r * k,
                };
                if fill {
                    t.FillEllipse(&e, brush)
                } else {
                    t.DrawEllipse(&e, brush, sw, self.round)
                }
            };
            match icon {
                Icon::Crop => {
                    poly(&[(6.0, 2.5), (6.0, 14.0), (17.5, 14.0)], false, false);
                    poly(&[(2.5, 6.0), (14.0, 6.0), (14.0, 17.5)], false, false);
                }
                // A picture: a frame, the sun and a hill.
                Icon::Background => {
                    rrect(2.5, 4.0, 15.0, 12.0, 2.5, false);
                    circle(7.0, 8.0, 1.4, true);
                    path(
                        (3.5, 14.5),
                        &[
                            Seg::L(8.0, 10.5),
                            Seg::L(11.0, 13.0),
                            Seg::L(13.5, 11.0),
                            Seg::L(16.5, 13.5),
                        ],
                    );
                }
                Icon::Select => poly(
                    &[
                        (5.0, 3.0),
                        (5.0, 16.0),
                        (8.5, 12.8),
                        (11.0, 17.5),
                        (13.0, 16.5),
                        (10.6, 11.8),
                        (15.0, 11.5),
                    ],
                    true,
                    true,
                ),
                Icon::Rect => rrect(4.0, 4.0, 12.0, 12.0, 1.5, false),
                Icon::FilledRect => rrect(4.0, 4.0, 12.0, 12.0, 1.5, true),
                Icon::Ellipse => circle(10.0, 10.0, 6.0, false),
                Icon::Line => line((4.5, 15.5), (15.5, 4.5)),
                Icon::Arrow => {
                    line((15.0, 5.0), (5.0, 15.0));
                    poly(&[(5.0, 8.5), (5.0, 15.0), (11.5, 15.0)], false, false);
                }
                Icon::Text => self.glyph(t, brush, "A", cx, cy, size, color),
                Icon::Pixelate => {
                    for (i, j) in [(0, 0), (2, 0), (1, 1), (0, 2), (2, 2)] {
                        rrect(
                            4.0 + i as f32 * 4.0,
                            4.0 + j as f32 * 4.0,
                            4.0,
                            4.0,
                            0.3,
                            true,
                        );
                    }
                    rrect(4.0, 4.0, 12.0, 12.0, 1.0, false);
                }
                // A viewfinder: four corners round a lit centre.
                Icon::Spotlight => {
                    for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                        let (x, y) = (10.0 - 6.5 * sx, 10.0 - 6.5 * sy);
                        path(
                            (x, y + 3.5 * sy),
                            &[
                                Seg::L(x, y + 1.5 * sy),
                                Seg::C(x, y + 0.6 * sy, x + 0.6 * sx, y, x + 1.5 * sx, y),
                                Seg::L(x + 3.5 * sx, y),
                            ],
                        );
                    }
                    circle(10.0, 10.0, 2.6, true);
                }
                Icon::Counter => {
                    circle(10.0, 10.0, 7.0, true);
                    // The digit is cut out of the circle: white on a dark icon,
                    // the accent on a white one (the tool when it is active).
                    let digit = if color.r > 0.8 && color.g > 0.8 && color.b > 0.8 {
                        crate::gfx::rgba(10, 122, 255, 1.0)
                    } else {
                        crate::gfx::rgba(255, 255, 255, 1.0)
                    };
                    self.glyph(t, brush, "1", cx, cy, size * 0.75, digit);
                }
                Icon::Pen => {
                    poly(
                        &[
                            (4.0, 16.0),
                            (5.0, 12.5),
                            (13.5, 4.0),
                            (16.0, 6.5),
                            (7.5, 15.0),
                        ],
                        false,
                        true,
                    );
                    line((11.8, 5.8), (14.2, 8.2));
                }
                Icon::Highlighter => {
                    poly(
                        &[(6.0, 13.0), (12.5, 3.5), (16.5, 6.5), (10.0, 16.0)],
                        false,
                        true,
                    );
                    poly(
                        &[(6.0, 13.0), (4.0, 17.0), (8.5, 17.0), (10.0, 16.0)],
                        false,
                        false,
                    );
                }
                // A push pin: a rounded head on a body that flares, the
                // point below.
                Icon::Pin => {
                    path(
                        (7.0, 3.5),
                        &[
                            Seg::L(13.0, 3.5),
                            Seg::L(12.2, 8.3),
                            Seg::C(13.8, 9.2, 14.8, 10.4, 14.8, 11.8),
                            Seg::L(5.2, 11.8),
                            Seg::C(5.2, 10.4, 6.2, 9.2, 7.8, 8.3),
                            Seg::L(7.0, 3.5),
                        ],
                    );
                    line((10.0, 11.8), (10.0, 17.0));
                }
                Icon::Copy => {
                    rrect(6.5, 6.5, 10.0, 10.0, 2.0, false);
                    poly(
                        &[
                            (13.5, 4.0),
                            (13.5, 3.5),
                            (4.5, 3.5),
                            (3.5, 4.5),
                            (3.5, 13.5),
                            (4.0, 13.5),
                        ],
                        false,
                        false,
                    );
                }
                Icon::Save => {
                    line((10.0, 3.0), (10.0, 12.0));
                    poly(&[(6.5, 8.5), (10.0, 12.0), (13.5, 8.5)], false, false);
                    poly(
                        &[(3.5, 12.5), (3.5, 16.5), (16.5, 16.5), (16.5, 12.5)],
                        false,
                        false,
                    );
                }
                Icon::Close => {
                    line((5.5, 5.5), (14.5, 14.5));
                    line((14.5, 5.5), (5.5, 14.5));
                }
                Icon::Minimize => line((5.0, 10.0), (15.0, 10.0)),
                Icon::Maximize => rrect(5.0, 5.0, 10.0, 10.0, 1.0, false),
                Icon::Trash => {
                    line((4.0, 5.5), (16.0, 5.5));
                    poly(
                        &[(8.0, 5.5), (8.0, 3.5), (12.0, 3.5), (12.0, 5.5)],
                        false,
                        false,
                    );
                    poly(
                        &[(5.5, 5.5), (6.5, 16.5), (13.5, 16.5), (14.5, 5.5)],
                        false,
                        false,
                    );
                }
                Icon::Chevron => poly(&[(6.0, 8.0), (10.0, 12.0), (14.0, 8.0)], false, false),
                Icon::Grip => {
                    for y in [7.0, 10.0, 13.0] {
                        line((7.0, y), (13.0, y));
                    }
                }
                // A quarter of a circle with its arrowhead: the way it turns.
                // A page with the arc of its turn above, the head showing
                // which way.
                Icon::RotateLeft => {
                    rrect(3.5, 9.5, 8.0, 8.0, 1.5, false);
                    path((16.0, 13.5), &[Seg::C(16.0, 8.0, 12.5, 5.0, 7.5, 5.0)]);
                    poly(&[(9.8, 2.6), (7.5, 5.0), (9.8, 7.4)], false, false);
                }
                Icon::RotateRight => {
                    rrect(8.5, 9.5, 8.0, 8.0, 1.5, false);
                    path((4.0, 13.5), &[Seg::C(4.0, 8.0, 7.5, 5.0, 12.5, 5.0)]);
                    poly(&[(10.2, 2.6), (12.5, 5.0), (10.2, 7.4)], false, false);
                }
                // Two halves facing across a dashed axis.
                Icon::FlipH => {
                    poly(&[(8.0, 4.5), (8.0, 15.5), (3.0, 15.5)], true, true);
                    poly(&[(12.0, 4.5), (12.0, 15.5), (17.0, 15.5)], false, true);
                    for y in [3.0, 7.0, 11.0, 15.0] {
                        line((10.0, y), (10.0, y + 1.5));
                    }
                }
                Icon::FlipV => {
                    poly(&[(4.5, 8.0), (15.5, 8.0), (15.5, 3.0)], true, true);
                    poly(&[(4.5, 12.0), (15.5, 12.0), (15.5, 17.0)], false, true);
                    for x in [3.0, 7.0, 11.0, 15.0] {
                        line((x, 10.0), (x + 1.5, 10.0));
                    }
                }
                // A horseshoe magnet: the crop holds to edges.
                Icon::Magnet => {
                    path(
                        (5.0, 4.0),
                        &[
                            Seg::L(5.0, 10.5),
                            Seg::C(5.0, 13.8, 7.2, 16.0, 10.0, 16.0),
                            Seg::C(12.8, 16.0, 15.0, 13.8, 15.0, 10.5),
                            Seg::L(15.0, 4.0),
                        ],
                    );
                    line((3.5, 7.0), (6.5, 7.0));
                    line((13.5, 7.0), (16.5, 7.0));
                }
                // A drop: blurred, softly.
                Icon::Drop => path(
                    (10.0, 3.0),
                    &[
                        Seg::C(10.0, 3.0, 4.5, 9.0, 4.5, 12.0),
                        Seg::C(4.5, 15.2, 7.0, 17.0, 10.0, 17.0),
                        Seg::C(13.0, 17.0, 15.5, 15.2, 15.5, 12.0),
                        Seg::C(15.5, 9.0, 10.0, 3.0, 10.0, 3.0),
                    ],
                ),
                // The drop with a padlock: blurred past recovery.
                Icon::DropLock => {
                    path(
                        (8.5, 3.0),
                        &[
                            Seg::C(8.5, 3.0, 3.5, 8.5, 3.5, 11.5),
                            Seg::C(3.5, 14.6, 5.6, 16.5, 8.5, 16.5),
                        ],
                    );
                    path((8.5, 3.0), &[Seg::C(8.5, 3.0, 11.0, 5.8, 12.4, 8.0)]);
                    rrect(11.0, 11.5, 6.5, 5.5, 1.0, true);
                    path(
                        (12.4, 11.5),
                        &[
                            Seg::L(12.4, 10.2),
                            Seg::C(12.4, 8.2, 16.1, 8.2, 16.1, 10.2),
                            Seg::L(16.1, 11.5),
                        ],
                    );
                }
                Icon::Check => poly(&[(4.5, 10.5), (8.5, 14.5), (15.5, 5.5)], false, false),
                // A U-turn arrow: a smooth bend, as the system's own.
                Icon::Undo => {
                    poly(&[(7.5, 4.5), (4.0, 8.0), (7.5, 11.5)], false, false);
                    path(
                        (4.0, 8.0),
                        &[
                            Seg::L(11.5, 8.0),
                            Seg::C(14.0, 8.0, 16.0, 10.0, 16.0, 12.0),
                            Seg::C(16.0, 14.0, 14.0, 16.0, 11.5, 16.0),
                            Seg::L(8.5, 16.0),
                        ],
                    );
                }
                Icon::Redo => {
                    poly(&[(12.5, 4.5), (16.0, 8.0), (12.5, 11.5)], false, false);
                    path(
                        (16.0, 8.0),
                        &[
                            Seg::L(8.5, 8.0),
                            Seg::C(6.0, 8.0, 4.0, 10.0, 4.0, 12.0),
                            Seg::C(4.0, 14.0, 6.0, 16.0, 8.5, 16.0),
                            Seg::L(11.5, 16.0),
                        ],
                    );
                }
            }
        }
    }

    fn glyph(
        &self,
        t: &ID2D1RenderTarget,
        brush: &ID2D1SolidColorBrush,
        text: &str,
        cx: f32,
        cy: f32,
        size: f32,
        color: D2D1_COLOR_F,
    ) {
        unsafe {
            let Ok(format) =
                crate::gfx::text_format_in(self.dwrite, size * 0.85, DWRITE_FONT_WEIGHT_BOLD)
            else {
                return;
            };
            let _ = format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let units: Vec<u16> = text.encode_utf16().collect();
            brush.SetColor(&color);
            crate::gfx::draw_text(
                t,
                brush,
                &format,
                &units,
                &rectf(cx - size, cy - size * 1.06, size * 2.0, size * 2.0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Direct2D::Common::{
        D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
    };
    use windows::Win32::Graphics::Direct2D::{
        D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Imaging::{GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad};

    const ALL: [Icon; 33] = [
        Icon::Crop,
        Icon::Background,
        Icon::Select,
        Icon::Rect,
        Icon::FilledRect,
        Icon::Ellipse,
        Icon::Line,
        Icon::Arrow,
        Icon::Text,
        Icon::Pixelate,
        Icon::Spotlight,
        Icon::Counter,
        Icon::Pen,
        Icon::Highlighter,
        Icon::Pin,
        Icon::Copy,
        Icon::Save,
        Icon::Close,
        Icon::Minimize,
        Icon::Maximize,
        Icon::Trash,
        Icon::Chevron,
        Icon::Grip,
        Icon::Undo,
        Icon::Redo,
        Icon::RotateLeft,
        Icon::RotateRight,
        Icon::FlipH,
        Icon::FlipV,
        Icon::Magnet,
        Icon::Drop,
        Icon::DropLock,
        Icon::Check,
    ];

    /// Every icon at toolbar size and four times over, on the light and the
    /// dark chrome, written to `SCM_ICON_SHEET` (`--ignored`): the sheet
    /// the set is drawn and reviewed against.
    #[test]
    #[ignore]
    fn icon_sheet() {
        crate::image::tests::com_for_test();
        let out = std::env::var("SCM_ICON_SHEET").unwrap_or_else(|_| {
            std::env::temp_dir()
                .join("scm-icons.png")
                .display()
                .to_string()
        });
        let (cols, big, small) = (13u32, 88f32, 18f32);
        let rows = ALL.len().div_ceil(cols as usize) as u32;
        let band = rows as f32 * big + 40.0;
        let (w, h) = (cols * big as u32, (band * 2.0) as u32);
        unsafe {
            let wic = crate::image::wic().unwrap();
            let bitmap = wic
                .CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)
                .unwrap();
            let factory = crate::gfx::d2d().unwrap();
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
            let t = factory
                .CreateWicBitmapRenderTarget(&bitmap, &props)
                .unwrap();
            let rt = t;
            let brush = rt
                .CreateSolidColorBrush(&crate::gfx::rgba(0, 0, 0, 1.0), None)
                .unwrap();
            let dwrite = crate::gfx::dwrite().unwrap();
            let round = crate::gfx::round_stroke(&factory, false).unwrap();
            let icons = Icons {
                factory: &factory,
                dwrite: &dwrite,
                round: &round,
            };
            rt.BeginDraw();
            for (b, pal) in [crate::theme::light(), crate::theme::dark()]
                .iter()
                .enumerate()
            {
                let top = b as f32 * band;
                brush.SetColor(&pal.chrome);
                rt.FillRectangle(&rectf(0.0, top, w as f32, band), &brush);
                for (i, icon) in ALL.iter().enumerate() {
                    let (c, r) = (i as u32 % cols, i as u32 / cols);
                    let (x, y) = (c as f32 * big + big / 2.0, top + r as f32 * big + big / 2.0);
                    icons.draw(&rt, &brush, *icon, x, y, big * 0.62, pal.icon);
                    let sx = 12.0 + i as f32 * (small + 14.0);
                    icons.draw(
                        &rt,
                        &brush,
                        *icon,
                        sx + small / 2.0,
                        top + band - 20.0,
                        small,
                        pal.icon,
                    );
                }
            }
            rt.EndDraw(None, None).unwrap();
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            bitmap
                .CopyPixels(std::ptr::null(), w * 4, &mut pixels)
                .unwrap();
            let img = crate::image::Image {
                width: w,
                height: h,
                pixels,
            };
            crate::image::save(
                &img,
                std::path::Path::new(&out),
                crate::settings::ImageFormat::Png,
                100,
            )
            .unwrap();
        }
    }
}
