//! Toolbar icons, drawn as vectors on a 20×20 grid: crisp at every scale and
//! independent of which icon font a Windows version happens to ship.

use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED,
    D2D1_FIGURE_END_OPEN,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_ROUNDED_RECT, ID2D1Factory, ID2D1RenderTarget,
    ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, IDWriteFactory,
};
use windows::core::w;

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
                Icon::Background => {
                    rrect(2.5, 4.0, 15.0, 12.0, 2.5, false);
                    rrect(6.0, 7.0, 8.0, 6.0, 1.2, true);
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
                Icon::Spotlight => {
                    rrect(2.5, 4.0, 15.0, 12.0, 2.0, false);
                    rrect(6.0, 7.5, 8.0, 5.0, 1.0, false);
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
                Icon::Pin => {
                    poly(
                        &[
                            (7.0, 3.5),
                            (13.0, 3.5),
                            (12.0, 8.5),
                            (14.5, 11.5),
                            (5.5, 11.5),
                            (8.0, 8.5),
                        ],
                        false,
                        true,
                    );
                    line((10.0, 11.5), (10.0, 17.0));
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
                Icon::Undo => {
                    poly(&[(7.5, 4.5), (4.0, 8.0), (7.5, 11.5)], false, false);
                    poly(
                        &[
                            (4.0, 8.0),
                            (12.0, 8.0),
                            (15.5, 11.0),
                            (15.5, 13.0),
                            (12.0, 16.0),
                            (8.0, 16.0),
                        ],
                        false,
                        false,
                    );
                }
                Icon::Redo => {
                    poly(&[(12.5, 4.5), (16.0, 8.0), (12.5, 11.5)], false, false);
                    poly(
                        &[
                            (16.0, 8.0),
                            (8.0, 8.0),
                            (4.5, 11.0),
                            (4.5, 13.0),
                            (8.0, 16.0),
                            (12.0, 16.0),
                        ],
                        false,
                        false,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
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
            let Ok(format) = self.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size * 0.85,
                w!(""),
            ) else {
                return;
            };
            let _ = format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let units: Vec<u16> = text.encode_utf16().collect();
            brush.SetColor(&color);
            t.DrawText(
                &units,
                &format,
                &rectf(cx - size, cy - size * 1.06, size * 2.0, size * 2.0),
                brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                Default::default(),
            );
        }
    }
}
