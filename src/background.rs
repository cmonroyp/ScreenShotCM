//! The background tool: the capture set on a gradient or a colour, with
//! padding, rounded corners and a soft shadow.

use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F, D2D1_GRADIENT_STOP};
use windows::Win32::Graphics::Direct2D::{
    D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_ROUNDED_RECT, ID2D1Brush, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::core::Interface;

use crate::gfx::{point, rgba};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Gradient(usize),
    Color(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Background {
    pub fill: Fill,
    /// Space around the capture, in image pixels.
    pub padding: f32,
    /// Corner radius of the capture, in image pixels.
    pub radius: f32,
    /// 0 to 1.
    pub shadow: f32,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            fill: Fill::Gradient(3),
            padding: 64.0,
            radius: 12.0,
            shadow: 0.6,
        }
    }
}

/// Two-colour diagonal gradients, light to deep.
pub const GRADIENTS: [[(u8, u8, u8); 2]; 15] = [
    [(255, 94, 98), (255, 153, 102)],
    [(106, 17, 203), (37, 117, 252)],
    [(19, 31, 76), (52, 94, 186)],
    [(56, 189, 248), (99, 102, 241)],
    [(255, 140, 120), (255, 75, 140)],
    [(168, 85, 247), (236, 72, 153)],
    [(226, 232, 240), (203, 213, 225)],
    [(148, 163, 184), (100, 116, 139)],
    [(52, 211, 153), (14, 165, 233)],
    [(17, 24, 39), (75, 85, 99)],
    [(251, 191, 36), (249, 115, 22)],
    [(244, 114, 182), (139, 92, 246)],
    [(45, 212, 191), (34, 197, 94)],
    [(250, 204, 21), (132, 204, 22)],
    [(15, 23, 42), (88, 28, 135)],
];

pub const COLORS: [(u8, u8, u8); 10] = [
    (255, 255, 255),
    (245, 245, 247),
    (209, 213, 219),
    (31, 41, 55),
    (0, 0, 0),
    (254, 226, 226),
    (254, 243, 199),
    (220, 252, 231),
    (219, 234, 254),
    (237, 233, 254),
];

fn color((r, g, b): (u8, u8, u8)) -> D2D1_COLOR_F {
    rgba(r, g, b, 1.0)
}

/// A brush for `fill` spanning `area`.
pub fn brush(t: &ID2D1RenderTarget, fill: Fill, area: D2D_RECT_F) -> Option<ID2D1Brush> {
    unsafe {
        match fill {
            Fill::Color(i) => t
                .CreateSolidColorBrush(&color(COLORS[i % COLORS.len()]), None)
                .ok()
                .and_then(|b: ID2D1SolidColorBrush| b.cast().ok()),
            Fill::Gradient(i) => {
                let [a, b] = GRADIENTS[i % GRADIENTS.len()];
                let stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: color(a),
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: color(b),
                    },
                ];
                let collection = t
                    .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
                    .ok()?;
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: point(area.left, area.top),
                    endPoint: point(area.right, area.bottom),
                };
                t.CreateLinearGradientBrush(&props, None, &collection)
                    .ok()
                    .and_then(|b| b.cast().ok())
            }
        }
    }
}

/// A soft shadow under `r`: stacked translucent rounded rectangles, each a
/// little larger and fainter, offset downwards.
pub fn shadow(
    t: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    r: D2D_RECT_F,
    radius: f32,
    strength: f32,
    reach: f32,
) {
    if strength <= 0.0 {
        return;
    }
    const STEPS: usize = 12;
    unsafe {
        for i in 0..STEPS {
            let f = (i + 1) as f32 / STEPS as f32;
            let grow = reach * f;
            let alpha = strength * 0.10 * (1.0 - f).powf(1.6);
            brush.SetColor(&rgba(0, 0, 0, alpha));
            let rr = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: r.left - grow,
                    top: r.top - grow + reach * 0.35,
                    right: r.right + grow,
                    bottom: r.bottom + grow + reach * 0.35,
                },
                radiusX: radius + grow,
                radiusY: radius + grow,
            };
            t.FillRoundedRectangle(&rr, brush);
        }
    }
}
