//! Annotations: what the editor draws over a capture. Shapes live in image
//! pixels, so the same list paints the editor's zoomed view and the exported
//! file. Pixelate and blur change pixels and are applied to a working copy
//! of the image; everything else is vector, drawn over it.

use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED,
    D2D1_FIGURE_END_OPEN, D2D1_FILL_MODE_ALTERNATE,
};
use windows::Win32::Graphics::Direct2D::D2D1_QUADRATIC_BEZIER_SEGMENT;
use windows::Win32::Graphics::Direct2D::{
    D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_ELLIPSE, D2D1_LINE_JOIN_ROUND, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
    ID2D1Factory, ID2D1PathGeometry, ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS, IDWriteFactory, IDWriteTextLayout,
};
use windows::core::Result;

use crate::gfx::{point, rectf, rgba};
use crate::image::Image;

pub type Pt = (f32, f32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    Plain,
    /// White text on a rounded label of the colour.
    Label,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Obscure {
    /// Blocks of one colour: unrecoverable, the "secure" kind.
    Pixelate,
    /// A soft blur.
    Blur,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub kind: Kind,
    /// Rectangle corners, line ends; the anchor of a text or a counter.
    pub a: Pt,
    pub b: Pt,
    /// Freehand points (pen, highlighter).
    pub points: Vec<Pt>,
    pub color: (u8, u8, u8),
    /// Stroke width; a text's font size; a counter's size.
    pub size: f32,
    pub text: String,
    pub number: u32,
    pub text_style: TextStyle,
    pub obscure: Obscure,
    /// A curved arrow's control point (a quadratic Bézier from `a` to `b`);
    /// `None` is a straight arrow.
    pub ctrl: Option<Pt>,
}

/// The point at `t` of the quadratic Bézier `a` → `b` pulled towards `c`.
pub fn quad(a: Pt, c: Pt, b: Pt, t: f32) -> Pt {
    let u = 1.0 - t;
    (
        u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
        u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
    )
}

/// The control point of a fresh curved arrow: bent to one side by a
/// quarter of its length, upwards for an arrow drawn left to right.
pub fn default_bend(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let (nx, ny) = (dy / len, -dx / len);
    let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    (mid.0 + nx * len * 0.3, mid.1 + ny * len * 0.3)
}

impl Shape {
    /// The arrow's path as points, for hit-testing.
    fn path_points(&self) -> Vec<Pt> {
        match self.ctrl {
            Some(c) => (0..=24)
                .map(|k| quad(self.a, c, self.b, k as f32 / 24.0))
                .collect(),
            None => vec![self.a, self.b],
        }
    }

    /// The middle of the arrow, where its bending handle sits.
    pub fn midpoint(&self) -> Pt {
        match self.ctrl {
            Some(c) => quad(self.a, c, self.b, 0.5),
            None => ((self.a.0 + self.b.0) / 2.0, (self.a.1 + self.b.1) / 2.0),
        }
    }
    pub fn new(kind: Kind, at: Pt, color: (u8, u8, u8), size: f32) -> Self {
        Self {
            kind,
            a: at,
            b: at,
            points: if matches!(kind, Kind::Pen | Kind::Highlighter) {
                vec![at]
            } else {
                Vec::new()
            },
            color,
            size,
            text: String::new(),
            number: 0,
            text_style: TextStyle::Plain,
            obscure: Obscure::Pixelate,
            ctrl: None,
        }
    }

    /// Rectangle-like shapes are defined by two corners.
    pub fn is_boxy(&self) -> bool {
        matches!(
            self.kind,
            Kind::Rect | Kind::FilledRect | Kind::Ellipse | Kind::Pixelate | Kind::Spotlight
        )
    }

    pub fn is_linear(&self) -> bool {
        matches!(self.kind, Kind::Line | Kind::Arrow)
    }

    pub fn rect(&self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.a.0.min(self.b.0),
            top: self.a.1.min(self.b.1),
            right: self.a.0.max(self.b.0),
            bottom: self.a.1.max(self.b.1),
        }
    }

    pub fn counter_radius(&self) -> f32 {
        self.size * 2.2 + 8.0
    }

    /// The area the shape covers, for hit-testing and the selection frame.
    /// Text needs its measured size, which the editor passes in.
    pub fn bounds(&self, text_size: Option<(f32, f32)>) -> D2D_RECT_F {
        match self.kind {
            Kind::Pen | Kind::Highlighter => {
                let mut r = D2D_RECT_F {
                    left: f32::MAX,
                    top: f32::MAX,
                    right: f32::MIN,
                    bottom: f32::MIN,
                };
                for p in &self.points {
                    r.left = r.left.min(p.0);
                    r.top = r.top.min(p.1);
                    r.right = r.right.max(p.0);
                    r.bottom = r.bottom.max(p.1);
                }
                let pad = self.stroke_width() / 2.0;
                D2D_RECT_F {
                    left: r.left - pad,
                    top: r.top - pad,
                    right: r.right + pad,
                    bottom: r.bottom + pad,
                }
            }
            Kind::Counter => {
                let r = self.counter_radius();
                D2D_RECT_F {
                    left: self.a.0 - r,
                    top: self.a.1 - r,
                    right: self.a.0 + r,
                    bottom: self.a.1 + r,
                }
            }
            Kind::Text => {
                let (w, h) = text_size.unwrap_or((self.size, self.size * 1.3));
                let pad = if self.text_style == TextStyle::Label {
                    self.size * 0.35
                } else {
                    0.0
                };
                D2D_RECT_F {
                    left: self.a.0 - pad,
                    top: self.a.1 - pad,
                    right: self.a.0 + w + pad,
                    bottom: self.a.1 + h + pad,
                }
            }
            Kind::Arrow if self.ctrl.is_some() => {
                let pts = self.path_points();
                let pad = self.size * 2.0;
                D2D_RECT_F {
                    left: pts.iter().map(|p| p.0).fold(f32::MAX, f32::min) - pad,
                    top: pts.iter().map(|p| p.1).fold(f32::MAX, f32::min) - pad,
                    right: pts.iter().map(|p| p.0).fold(f32::MIN, f32::max) + pad,
                    bottom: pts.iter().map(|p| p.1).fold(f32::MIN, f32::max) + pad,
                }
            }
            _ => {
                let r = self.rect();
                let pad = if self.is_linear() {
                    self.size * 2.0
                } else {
                    0.0
                };
                D2D_RECT_F {
                    left: r.left - pad,
                    top: r.top - pad,
                    right: r.right + pad,
                    bottom: r.bottom + pad,
                }
            }
        }
    }

    pub fn stroke_width(&self) -> f32 {
        match self.kind {
            Kind::Highlighter => self.size * 4.0 + 6.0,
            _ => self.size,
        }
    }

    /// Whether `p` (image pixels) is on the shape, with `tol` of slack.
    pub fn hit(&self, p: Pt, tol: f32, text_size: Option<(f32, f32)>) -> bool {
        match self.kind {
            Kind::Line | Kind::Arrow => self
                .path_points()
                .windows(2)
                .any(|s| segment_distance(p, s[0], s[1]) <= self.size / 2.0 + tol),
            Kind::Pen | Kind::Highlighter => {
                let reach = self.stroke_width() / 2.0 + tol;
                if self.points.len() == 1 {
                    return dist(p, self.points[0]) <= reach;
                }
                self.points
                    .windows(2)
                    .any(|s| segment_distance(p, s[0], s[1]) <= reach)
            }
            Kind::Rect | Kind::Ellipse => {
                // An outline is hit on its stroke, so what is inside stays
                // reachable.
                let r = self.rect();
                let reach = self.size / 2.0 + tol;
                let inside_outer = p.0 >= r.left - reach
                    && p.0 <= r.right + reach
                    && p.1 >= r.top - reach
                    && p.1 <= r.bottom + reach;
                let inside_inner = p.0 > r.left + reach
                    && p.0 < r.right - reach
                    && p.1 > r.top + reach
                    && p.1 < r.bottom - reach;
                if self.kind == Kind::Ellipse {
                    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                    let (rx, ry) = (
                        ((r.right - r.left) / 2.0).max(1.0),
                        ((r.bottom - r.top) / 2.0).max(1.0),
                    );
                    let d = (((p.0 - cx) / rx).powi(2) + ((p.1 - cy) / ry).powi(2)).sqrt();
                    let band = reach / rx.min(ry);
                    return (d - 1.0).abs() <= band;
                }
                inside_outer && !inside_inner
            }
            _ => {
                let b = self.bounds(text_size);
                p.0 >= b.left - tol
                    && p.0 <= b.right + tol
                    && p.1 >= b.top - tol
                    && p.1 <= b.bottom + tol
            }
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.a = (self.a.0 + dx, self.a.1 + dy);
        self.b = (self.b.0 + dx, self.b.1 + dy);
        if let Some(c) = &mut self.ctrl {
            *c = (c.0 + dx, c.1 + dy);
        }
        for p in &mut self.points {
            *p = (p.0 + dx, p.1 + dy);
        }
    }

    /// Follows the image it is drawn on when that is resized: every point
    /// by `kx`, `ky`, and the stroke, font or counter size by `kx` (the two
    /// differ only by rounding — a resize keeps the proportion).
    pub fn scale(&mut self, kx: f32, ky: f32) {
        let at = |p: Pt| (p.0 * kx, p.1 * ky);
        self.a = at(self.a);
        self.b = at(self.b);
        self.ctrl = self.ctrl.map(at);
        for p in &mut self.points {
            *p = at(*p);
        }
        self.size *= kx;
    }

    /// Handles the user can drag: a box's corners or a line's ends.
    pub fn handles(&self) -> Vec<Pt> {
        if self.is_boxy() {
            let r = self.rect();
            vec![
                (r.left, r.top),
                (r.right, r.top),
                (r.left, r.bottom),
                (r.right, r.bottom),
            ]
        } else if self.kind == Kind::Arrow {
            // The third handle, in the middle, bends the arrow.
            vec![self.a, self.b, self.midpoint()]
        } else if self.is_linear() {
            vec![self.a, self.b]
        } else {
            Vec::new()
        }
    }

    /// Moves handle `i` (as numbered by `handles`) to `p`.
    pub fn drag_handle(&mut self, i: usize, p: Pt) {
        if self.is_linear() {
            match i {
                0 => self.a = p,
                1 => self.b = p,
                _ => self.bend_through(p),
            }
            return;
        }
        let r = self.rect();
        let (mut l, mut t, mut rr, mut b) = (r.left, r.top, r.right, r.bottom);
        match i {
            0 => (l, t) = p,
            1 => (rr, t) = p,
            2 => (l, b) = p,
            _ => (rr, b) = p,
        }
        self.a = (l, t);
        self.b = (rr, b);
    }

    /// Bends the arrow so its middle passes through `p`. Brought back within
    /// a couple of pixels of the straight line, it is straight again.
    pub fn bend_through(&mut self, p: Pt) {
        // B(½) = ¼a + ½c + ¼b, so the control point that puts the middle
        // at p is 2p − (a + b)/2.
        let c = (
            2.0 * p.0 - (self.a.0 + self.b.0) / 2.0,
            2.0 * p.1 - (self.a.1 + self.b.1) / 2.0,
        );
        let straight = segment_distance(p, self.a, self.b) < 3.0;
        self.ctrl = if straight { None } else { Some(c) };
    }

    /// A shape too small to mean anything is dropped when drawing ends.
    pub fn is_degenerate(&self) -> bool {
        match self.kind {
            Kind::Text => self.text.trim().is_empty(),
            Kind::Counter => false,
            Kind::Pen | Kind::Highlighter => self.points.is_empty(),
            _ => dist(self.a, self.b) < 2.0,
        }
    }
}

fn dist(a: Pt, b: Pt) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn segment_distance(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return dist(p, a);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// Applies every pixelate and blur in `shapes`, in order, to a copy of `base`.
pub fn obscured(base: &Image, shapes: &[Shape]) -> Image {
    let mut img = base.clone();
    for s in shapes.iter().filter(|s| s.kind == Kind::Pixelate) {
        let r = s.rect();
        let (x0, y0) = (r.left.max(0.0) as u32, r.top.max(0.0) as u32);
        let (x1, y1) = (
            (r.right.max(0.0) as u32).min(img.width),
            (r.bottom.max(0.0) as u32).min(img.height),
        );
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        match s.obscure {
            Obscure::Pixelate => pixelate(
                &mut img,
                x0,
                y0,
                x1,
                y1,
                (s.size * 2.0 + 6.0).max(4.0) as u32,
            ),
            Obscure::Blur => blur(
                &mut img,
                x0,
                y0,
                x1,
                y1,
                (s.size * 1.5 + 4.0).max(3.0) as u32,
            ),
        }
    }
    img
}

/// Blocks of `block` pixels, each the mean of what it covers.
pub fn pixelate(img: &mut Image, x0: u32, y0: u32, x1: u32, y1: u32, block: u32) {
    let stride = img.stride();
    let mut by = y0;
    while by < y1 {
        let bh = block.min(y1 - by);
        let mut bx = x0;
        while bx < x1 {
            let bw = block.min(x1 - bx);
            let mut sum = [0u64; 3];
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let i = y as usize * stride + x as usize * 4;
                    for c in 0..3 {
                        sum[c] += img.pixels[i + c] as u64;
                    }
                }
            }
            let n = (bw * bh) as u64;
            let mean = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8];
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let i = y as usize * stride + x as usize * 4;
                    img.pixels[i..i + 3].copy_from_slice(&mean);
                }
            }
            bx += block;
        }
        by += block;
    }
}

/// Three box blurs of `radius`: close to a Gaussian, linear in the area.
/// Only pixels inside the rectangle change; the edges sample themselves.
/// Each pass works one row or one column at a time, in place, through a
/// buffer of one line: a whole copy of the area in floats (24 bytes a pixel)
/// was 1.8 GB for a blur over a long scrolling capture.
pub fn blur(img: &mut Image, x0: u32, y0: u32, x1: u32, y1: u32, radius: u32) {
    let (x0, y0) = (x0 as usize, y0 as usize);
    let (w, h) = (x1 as usize - x0, y1 as usize - y0);
    if w == 0 || h == 0 {
        return;
    }
    let stride = img.stride();
    let r = radius as usize;
    let mut line: Vec<[u8; 3]> = Vec::with_capacity(w.max(h));
    for _ in 0..3 {
        for y in 0..h {
            let at = |x: usize| (y0 + y) * stride + (x0 + x) * 4;
            box_line(img, &mut line, (0..w).map(at), r);
        }
        for x in 0..w {
            let at = |y: usize| (y0 + y) * stride + (x0 + x) * 4;
            box_line(img, &mut line, (0..h).map(at), r);
        }
    }
}

/// One box blur along the pixels at `offsets` (a row or a column).
fn box_line(
    img: &mut Image,
    line: &mut Vec<[u8; 3]>,
    offsets: impl Iterator<Item = usize> + Clone,
    r: usize,
) {
    line.clear();
    line.extend(
        offsets
            .clone()
            .map(|i| [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2]]),
    );
    let n = line.len() as isize;
    let at = |i: isize| line[i.clamp(0, n - 1) as usize];
    let (ri, div) = (r as isize, (2 * r + 1) as u32);
    let mut sum = [0u32; 3];
    for i in -ri..=ri {
        let p = at(i);
        for c in 0..3 {
            sum[c] += p[c] as u32;
        }
    }
    for (i, out) in offsets.enumerate() {
        for c in 0..3 {
            img.pixels[out + c] = ((sum[c] + div / 2) / div) as u8;
        }
        let (add, sub) = (at(i as isize + ri + 1), at(i as isize - ri));
        for c in 0..3 {
            sum[c] = sum[c] + add[c] as u32 - sub[c] as u32;
        }
    }
}

/// Draws shapes onto a render target whose transform maps image pixels.
pub struct Painter {
    pub factory: ID2D1Factory,
    pub dwrite: IDWriteFactory,
    round: ID2D1StrokeStyle,
    flat: ID2D1StrokeStyle,
}

impl Painter {
    pub fn new(factory: ID2D1Factory) -> Result<Self> {
        let props = |cap| D2D1_STROKE_STYLE_PROPERTIES {
            startCap: cap,
            endCap: cap,
            dashCap: cap,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        unsafe {
            let round = factory.CreateStrokeStyle(&props(D2D1_CAP_STYLE_ROUND), None)?;
            let flat = factory.CreateStrokeStyle(&props(D2D1_CAP_STYLE_FLAT), None)?;
            Ok(Self {
                dwrite: crate::gfx::dwrite()?,
                factory,
                round,
                flat,
            })
        }
    }

    /// Spotlights first (they dim what is outside them), then every other
    /// shape in the order it was drawn.
    pub fn draw_all(
        &self,
        t: &ID2D1RenderTarget,
        brush: &ID2D1SolidColorBrush,
        shapes: &[Shape],
        image_size: (f32, f32),
    ) {
        self.draw_spotlights(t, brush, shapes, image_size);
        for s in shapes {
            self.draw(t, brush, s);
        }
    }

    fn draw_spotlights(
        &self,
        t: &ID2D1RenderTarget,
        brush: &ID2D1SolidColorBrush,
        shapes: &[Shape],
        (w, h): (f32, f32),
    ) {
        let spots: Vec<D2D_RECT_F> = shapes
            .iter()
            .filter(|s| s.kind == Kind::Spotlight)
            .map(Shape::rect)
            .collect();
        if spots.is_empty() {
            return;
        }
        unsafe {
            let Ok(geo) = self.factory.CreatePathGeometry() else {
                return;
            };
            let Ok(sink) = geo.Open() else { return };
            sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE);
            for r in std::iter::once(rectf(0.0, 0.0, w, h)).chain(spots.iter().copied()) {
                sink.BeginFigure(point(r.left, r.top), D2D1_FIGURE_BEGIN_FILLED);
                sink.AddLines(&[
                    point(r.right, r.top),
                    point(r.right, r.bottom),
                    point(r.left, r.bottom),
                ]);
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
            let _ = sink.Close();
            brush.SetColor(&rgba(0, 0, 0, 0.55));
            t.FillGeometry(&geo, brush, None);
        }
    }

    pub fn draw(&self, t: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush, s: &Shape) {
        let (r, g, b) = s.color;
        let color = rgba(r, g, b, 1.0);
        unsafe {
            brush.SetColor(&color);
            match s.kind {
                Kind::Rect => {
                    let rc = s.rect();
                    let rounded = D2D1_ROUNDED_RECT {
                        rect: rc,
                        radiusX: s.size * 0.6,
                        radiusY: s.size * 0.6,
                    };
                    t.DrawRoundedRectangle(&rounded, brush, s.size, &self.round);
                }
                Kind::FilledRect => {
                    let rounded = D2D1_ROUNDED_RECT {
                        rect: s.rect(),
                        radiusX: s.size * 0.6,
                        radiusY: s.size * 0.6,
                    };
                    t.FillRoundedRectangle(&rounded, brush);
                }
                Kind::Ellipse => {
                    let rc = s.rect();
                    let e = D2D1_ELLIPSE {
                        point: point((rc.left + rc.right) / 2.0, (rc.top + rc.bottom) / 2.0),
                        radiusX: (rc.right - rc.left) / 2.0,
                        radiusY: (rc.bottom - rc.top) / 2.0,
                    };
                    t.DrawEllipse(&e, brush, s.size, &self.round);
                }
                Kind::Line => t.DrawLine(
                    point(s.a.0, s.a.1),
                    point(s.b.0, s.b.1),
                    brush,
                    s.size,
                    &self.round,
                ),
                Kind::Arrow => {
                    let geo = match s.ctrl {
                        Some(c) => self.curved_arrow(s.a, c, s.b, s.size),
                        None => self.arrow(s.a, s.b, s.size),
                    };
                    if let Ok(geo) = geo {
                        t.FillGeometry(&geo, brush, None);
                        // A hairline in the same colour rounds the joins.
                        t.DrawGeometry(&geo, brush, (s.size * 0.25).max(1.0), &self.round);
                    }
                }
                Kind::Counter => self.counter(t, brush, s),
                Kind::Text => self.text(t, brush, s),
                Kind::Pen | Kind::Highlighter => {
                    if s.kind == Kind::Highlighter {
                        brush.SetColor(&rgba(r, g, b, 0.38));
                    }
                    if let Ok(geo) = self.freehand(&s.points) {
                        let style = if s.kind == Kind::Highlighter {
                            &self.flat
                        } else {
                            &self.round
                        };
                        t.DrawGeometry(&geo, brush, s.stroke_width(), style);
                    }
                }
                // Drawn by `obscured` and `draw_spotlights`.
                Kind::Pixelate | Kind::Spotlight => {}
            }
        }
    }

    /// A curved arrow in the same style as the straight one: the shaft
    /// follows the curve and widens towards the head, and the head points
    /// along the curve's last stretch.
    pub fn curved_arrow(&self, a: Pt, c: Pt, b: Pt, size: f32) -> Result<ID2D1PathGeometry> {
        let geo = unsafe { self.factory.CreatePathGeometry()? };
        let chord = dist(a, b).max(0.001);
        let head = (size * 4.5 + 8.0).min(chord * 0.6);
        let half_head = head * 0.55;
        // Where the head starts: the last point of the curve a head's
        // length away from the tip.
        let mut t_end = 1.0f32;
        while t_end > 0.0 && dist(quad(a, c, b, t_end), b) < head {
            t_end -= 0.005;
        }
        let t_end = t_end.max(0.0);
        let base = quad(a, c, b, t_end);
        let (hx, hy) = (b.0 - base.0, b.1 - base.1);
        let hl = (hx * hx + hy * hy).sqrt().max(0.001);
        let (hnx, hny) = (-hy / hl, hx / hl);
        let (tail, neck) = (size * 0.15, size * 0.55);

        const STEPS: usize = 32;
        let mut left = Vec::with_capacity(STEPS + 1);
        let mut right = Vec::with_capacity(STEPS + 1);
        for k in 0..=STEPS {
            let t = t_end * k as f32 / STEPS as f32;
            let p = quad(a, c, b, t);
            // The tangent: B'(t) = 2(1−t)(c−a) + 2t(b−c).
            let (dx, dy) = (
                2.0 * (1.0 - t) * (c.0 - a.0) + 2.0 * t * (b.0 - c.0),
                2.0 * (1.0 - t) * (c.1 - a.1) + 2.0 * t * (b.1 - c.1),
            );
            let l = (dx * dx + dy * dy).sqrt().max(0.001);
            let (nx, ny) = (-dy / l, dx / l);
            let w = tail + (neck - tail) * (k as f32 / STEPS as f32);
            left.push((p.0 + nx * w, p.1 + ny * w));
            right.push((p.0 - nx * w, p.1 - ny * w));
        }
        let mut pts = left;
        pts.push((base.0 + hnx * half_head, base.1 + hny * half_head));
        pts.push(b);
        pts.push((base.0 - hnx * half_head, base.1 - hny * half_head));
        pts.extend(right.into_iter().rev());
        unsafe {
            let sink = geo.Open()?;
            sink.BeginFigure(point(pts[0].0, pts[0].1), D2D1_FIGURE_BEGIN_FILLED);
            let rest: Vec<_> = pts[1..].iter().map(|p| point(p.0, p.1)).collect();
            sink.AddLines(&rest);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
        }
        Ok(geo)
    }

    /// A tapered shaft and a sharp head.
    pub fn arrow(&self, a: Pt, b: Pt, size: f32) -> Result<ID2D1PathGeometry> {
        let geo = unsafe { self.factory.CreatePathGeometry()? };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        let (ux, uy) = (dx / len, dy / len);
        let (nx, ny) = (-uy, ux);
        let head = (size * 4.5 + 8.0).min(len * 0.6);
        let half_head = head * 0.55;
        let base = (b.0 - ux * head, b.1 - uy * head);
        let tail = size * 0.15;
        let neck = size * 0.55;
        let pts = [
            (a.0 + nx * tail, a.1 + ny * tail),
            (base.0 + nx * neck, base.1 + ny * neck),
            (base.0 + nx * half_head, base.1 + ny * half_head),
            b,
            (base.0 - nx * half_head, base.1 - ny * half_head),
            (base.0 - nx * neck, base.1 - ny * neck),
            (a.0 - nx * tail, a.1 - ny * tail),
        ];
        unsafe {
            let sink = geo.Open()?;
            sink.BeginFigure(point(pts[0].0, pts[0].1), D2D1_FIGURE_BEGIN_FILLED);
            let rest: Vec<_> = pts[1..].iter().map(|p| point(p.0, p.1)).collect();
            sink.AddLines(&rest);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
        }
        Ok(geo)
    }

    /// A smooth path through the points: quadratic curves between midpoints.
    fn freehand(&self, pts: &[Pt]) -> Result<ID2D1PathGeometry> {
        let geo = unsafe { self.factory.CreatePathGeometry()? };
        unsafe {
            let sink = geo.Open()?;
            sink.BeginFigure(point(pts[0].0, pts[0].1), D2D1_FIGURE_BEGIN_HOLLOW);
            if pts.len() == 1 {
                sink.AddLine(point(pts[0].0 + 0.01, pts[0].1));
            } else {
                for i in 1..pts.len() - 1 {
                    let mid = (
                        (pts[i].0 + pts[i + 1].0) / 2.0,
                        (pts[i].1 + pts[i + 1].1) / 2.0,
                    );
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
                        point1: point(pts[i].0, pts[i].1),
                        point2: point(mid.0, mid.1),
                    });
                }
                let last = pts[pts.len() - 1];
                sink.AddLine(point(last.0, last.1));
            }
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
        }
        Ok(geo)
    }

    fn counter(&self, t: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush, s: &Shape) {
        let r = s.counter_radius();
        let e = D2D1_ELLIPSE {
            point: point(s.a.0, s.a.1),
            radiusX: r,
            radiusY: r,
        };
        unsafe {
            t.FillEllipse(&e, brush);
            brush.SetColor(&rgba(255, 255, 255, 1.0));
            t.DrawEllipse(&e, brush, (r * 0.12).max(1.5), None);
            let Ok(format) =
                crate::gfx::text_format_in(&self.dwrite, r * 1.15, DWRITE_FONT_WEIGHT_BOLD)
            else {
                return;
            };
            let _ = format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let label: Vec<u16> = s.number.to_string().encode_utf16().collect();
            crate::gfx::draw_text(
                t,
                brush,
                &format,
                &label,
                &rectf(s.a.0 - r, s.a.1 - r * 1.08, 2.0 * r, 2.0 * r),
            );
        }
    }

    /// The text's layout, unwrapped, as it is drawn: the editor hit-tests it
    /// to place the caret and paint the selection on the very glyphs drawn.
    pub fn text_layout(&self, s: &Shape) -> Option<IDWriteTextLayout> {
        let units: Vec<u16> = s.text.encode_utf16().collect();
        unsafe {
            let format = self.text_format(s.size).ok()?;
            self.dwrite
                .CreateTextLayout(&units, &format, 100_000.0, 100_000.0)
                .ok()
        }
    }

    /// The measured size of a text shape, in image pixels.
    pub fn text_size(&self, s: &Shape) -> (f32, f32) {
        let shown = if s.text.is_empty() {
            " "
        } else {
            s.text.as_str()
        };
        let units: Vec<u16> = shown.encode_utf16().collect();
        unsafe {
            let Ok(format) = self.text_format(s.size) else {
                return (s.size, s.size * 1.3);
            };
            let Ok(layout) = self
                .dwrite
                .CreateTextLayout(&units, &format, 100_000.0, 100_000.0)
            else {
                return (s.size, s.size * 1.3);
            };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            (
                m.widthIncludingTrailingWhitespace.max(s.size * 0.3),
                m.height,
            )
        }
    }

    fn text_format(
        &self,
        size: f32,
    ) -> Result<windows::Win32::Graphics::DirectWrite::IDWriteTextFormat> {
        crate::gfx::text_format_in(&self.dwrite, size, DWRITE_FONT_WEIGHT_SEMI_BOLD)
    }

    fn text(&self, t: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush, s: &Shape) {
        let (w, h) = self.text_size(s);
        let Some(layout) = self.text_layout(s) else {
            return;
        };
        unsafe {
            if s.text_style == TextStyle::Label {
                let pad = s.size * 0.35;
                let label = D2D1_ROUNDED_RECT {
                    rect: rectf(s.a.0 - pad, s.a.1 - pad, w + 2.0 * pad, h + 2.0 * pad),
                    radiusX: pad * 1.2,
                    radiusY: pad * 1.2,
                };
                t.FillRoundedRectangle(&label, brush);
                let light = s.color.0 as u32 + s.color.1 as u32 + s.color.2 as u32 > 600;
                brush.SetColor(&if light {
                    rgba(20, 20, 20, 1.0)
                } else {
                    rgba(255, 255, 255, 1.0)
                });
            }
            // The same layout the editor hit-tests, so the caret sits on
            // the glyphs.
            t.DrawTextLayout(
                point(s.a.0, s.a.1),
                &layout,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }
}

/// Editing a text annotation. Positions are UTF-16 offsets, as DirectWrite
/// counts them; these helpers keep the `String` and those offsets in step.
pub mod textedit {
    pub fn len(s: &str) -> usize {
        s.encode_utf16().count()
    }

    /// The byte index of UTF-16 offset `pos` (clamped to the end).
    pub fn byte(s: &str, pos: usize) -> usize {
        let mut units = 0;
        for (i, c) in s.char_indices() {
            if units >= pos {
                return i;
            }
            units += c.len_utf16();
        }
        s.len()
    }

    fn chars(s: &str) -> Vec<(usize, char)> {
        let mut out = Vec::new();
        let mut units = 0;
        for c in s.chars() {
            out.push((units, c));
            units += c.len_utf16();
        }
        out
    }

    /// Replaces the selection between `a` and `b` with `text`; returns the
    /// caret after the insertion.
    pub fn replace(s: &mut String, a: usize, b: usize, text: &str) -> usize {
        let (lo, hi) = (a.min(b), a.max(b));
        let (bl, bh) = (byte(s, lo), byte(s, hi));
        s.replace_range(bl..bh, text);
        lo + len(text)
    }

    /// The character before `pos`, or the word before it with `word`.
    pub fn prev(s: &str, pos: usize, word: bool) -> usize {
        let cs: Vec<_> = chars(s).into_iter().filter(|(u, _)| *u < pos).collect();
        let Some(&(last, _)) = cs.last() else {
            return 0;
        };
        if !word {
            return last;
        }
        let mut i = cs.len();
        while i > 0 && cs[i - 1].1.is_whitespace() {
            i -= 1;
        }
        while i > 0 && !cs[i - 1].1.is_whitespace() {
            i -= 1;
        }
        cs.get(i).map(|c| c.0).unwrap_or(0)
    }

    /// The character after `pos`, or the end of the next word with `word`.
    pub fn next(s: &str, pos: usize, word: bool) -> usize {
        let end = len(s);
        let cs: Vec<_> = chars(s).into_iter().filter(|(u, _)| *u >= pos).collect();
        if cs.is_empty() {
            return end;
        }
        if !word {
            return cs.get(1).map(|c| c.0).unwrap_or(end);
        }
        let mut i = 0;
        while i < cs.len() && cs[i].1.is_whitespace() {
            i += 1;
        }
        while i < cs.len() && !cs[i].1.is_whitespace() {
            i += 1;
        }
        cs.get(i).map(|c| c.0).unwrap_or(end)
    }

    /// The word around `pos`, as (start, end).
    pub fn word_at(s: &str, pos: usize) -> (usize, usize) {
        let cs = chars(s);
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let i = cs.iter().position(|(u, _)| *u >= pos).unwrap_or(cs.len());
        let mut lo = i;
        while lo > 0 && is_word(cs[lo - 1].1) {
            lo -= 1;
        }
        let mut hi = i;
        while hi < cs.len() && is_word(cs[hi].1) {
            hi += 1;
        }
        let at = |k: usize| cs.get(k).map(|c| c.0).unwrap_or(len(s));
        (at(lo), at(hi))
    }

    pub fn line_start(s: &str, pos: usize) -> usize {
        chars(s)
            .into_iter()
            .filter(|(u, c)| *u < pos && *c == '\n')
            .map(|(u, _)| u + 1)
            .next_back()
            .unwrap_or(0)
    }

    pub fn line_end(s: &str, pos: usize) -> usize {
        chars(s)
            .into_iter()
            .find(|(u, c)| *u >= pos && *c == '\n')
            .map(|(u, _)| u)
            .unwrap_or(len(s))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn replace_inserts_at_the_caret_and_over_a_selection() {
            let mut s = "Hola mundo".to_string();
            assert_eq!(replace(&mut s, 4, 4, ","), 5);
            assert_eq!(s, "Hola, mundo");
            assert_eq!(replace(&mut s, 11, 6, "Ana"), 9);
            assert_eq!(s, "Hola, Ana");
        }

        #[test]
        fn offsets_count_utf16_units_not_bytes() {
            let mut s = "año 😀 fin".to_string();
            assert_eq!(len(&s), 10);
            assert_eq!(byte(&s, 1), 1);
            assert_eq!(byte(&s, 2), 3, "ñ is two bytes, one unit");
            // The emoji is two units: stepping over it moves two.
            assert_eq!(next(&s, 4, false), 6);
            assert_eq!(prev(&s, 6, false), 4);
            replace(&mut s, 4, 6, "x");
            assert_eq!(s, "año x fin");
        }

        #[test]
        fn word_steps_skip_spaces_then_the_word() {
            let s = "uno  dos tres";
            assert_eq!(next(s, 0, true), 3);
            assert_eq!(next(s, 3, true), 8);
            assert_eq!(prev(s, 13, true), 9);
            assert_eq!(prev(s, 9, true), 5);
            assert_eq!(word_at(s, 6), (5, 8));
        }

        #[test]
        fn lines_are_bounded_by_newlines() {
            let s = "uno\ndos\ntres";
            assert_eq!(line_start(s, 6), 4);
            assert_eq!(line_end(s, 6), 7);
            assert_eq!(line_start(s, 2), 0);
            assert_eq!(line_end(s, 10), 12);
        }

        #[test]
        fn steps_stop_at_the_ends() {
            assert_eq!(prev("ab", 0, false), 0);
            assert_eq!(next("ab", 2, false), 2);
            assert_eq!(next("", 0, true), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Image {
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 0 } else { 200 };
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        Image {
            width: w,
            height: h,
            pixels,
        }
    }

    #[test]
    fn pixelate_makes_each_block_one_colour_and_leaves_the_rest() {
        let mut img = checker(8, 8);
        pixelate(&mut img, 0, 0, 4, 4, 4);
        let first = &img.pixels[0..3].to_vec();
        for y in 0..4 {
            for x in 0..4 {
                let i = (y * 8 + x) * 4;
                assert_eq!(&img.pixels[i..i + 3], first.as_slice());
            }
        }
        assert_eq!(first[0], 100);
        let outside = (5 * 8 + 5) * 4;
        assert_eq!(img.pixels[outside], checker(8, 8).pixels[outside]);
    }

    #[test]
    fn blur_keeps_a_plain_colour_and_touches_nothing_outside() {
        let mut img = checker(20, 20);
        let before = img.pixels.clone();
        blur(&mut img, 5, 5, 12, 9, 4);
        for y in 0..20 {
            for x in 0..20 {
                let i = (y * 20 + x) * 4;
                if !(5..12).contains(&x) || !(5..9).contains(&y) {
                    assert_eq!(img.pixels[i..i + 4], before[i..i + 4], "({x},{y})");
                }
            }
        }
        let mut plain = Image {
            width: 8,
            height: 8,
            pixels: [37, 120, 201, 255].repeat(64),
        };
        blur(&mut plain, 0, 0, 8, 8, 3);
        assert!(plain.pixels.chunks(4).all(|p| p == [37, 120, 201, 255]));
    }

    #[test]
    fn blur_smooths_a_checkerboard_towards_its_mean() {
        let mut img = checker(20, 20);
        blur(&mut img, 0, 0, 20, 20, 3);
        let centre = (10 * 20 + 10) * 4;
        assert!(
            (img.pixels[centre] as i32 - 100).abs() < 15,
            "{}",
            img.pixels[centre]
        );
    }

    #[test]
    fn obscured_ignores_rectangles_outside_the_image() {
        let base = checker(10, 10);
        let mut s = Shape::new(Kind::Pixelate, (20.0, 20.0), (0, 0, 0), 4.0);
        s.b = (30.0, 30.0);
        assert_eq!(obscured(&base, &[s]).pixels, base.pixels);
    }

    #[test]
    fn an_outline_is_hit_on_its_stroke_not_inside() {
        let mut s = Shape::new(Kind::Rect, (0.0, 0.0), (0, 0, 0), 4.0);
        s.b = (100.0, 100.0);
        assert!(s.hit((0.0, 50.0), 2.0, None));
        assert!(!s.hit((50.0, 50.0), 2.0, None));
    }

    #[test]
    fn a_line_is_hit_near_the_segment_only() {
        let mut s = Shape::new(Kind::Arrow, (0.0, 0.0), (0, 0, 0), 4.0);
        s.b = (100.0, 0.0);
        assert!(s.hit((50.0, 3.0), 2.0, None));
        assert!(!s.hit((50.0, 20.0), 2.0, None));
        assert!(!s.hit((130.0, 0.0), 2.0, None));
    }

    #[test]
    fn dragging_the_middle_handle_bends_the_arrow_through_the_pointer() {
        let mut s = Shape::new(Kind::Arrow, (0.0, 0.0), (0, 0, 0), 4.0);
        s.b = (100.0, 0.0);
        assert_eq!(s.handles().len(), 3, "ends and a bending handle");
        s.drag_handle(2, (50.0, -40.0));
        let mid = s.midpoint();
        assert!(
            (mid.0 - 50.0).abs() < 0.01 && (mid.1 + 40.0).abs() < 0.01,
            "{mid:?}"
        );
        // The curve is hit where it runs, not on the straight chord.
        assert!(s.hit((50.0, -40.0), 2.0, None));
        assert!(!s.hit((50.0, 0.0), 2.0, None));
        // Moving it carries the bend along.
        s.translate(10.0, 5.0);
        let moved = s.midpoint();
        assert!((moved.0 - 60.0).abs() < 0.01 && (moved.1 + 35.0).abs() < 0.01);
        // Brought back onto the line, it is straight again.
        s.drag_handle(2, (61.0, 6.0));
        assert_eq!(s.ctrl, None);
    }

    #[test]
    fn a_fresh_curve_bends_upwards_for_a_left_to_right_arrow() {
        let c = default_bend((0.0, 100.0), (100.0, 100.0));
        assert!(c.1 < 100.0, "{c:?}");
    }

    #[test]
    fn dragging_a_corner_keeps_the_opposite_one() {
        let mut s = Shape::new(Kind::Rect, (10.0, 10.0), (0, 0, 0), 4.0);
        s.b = (50.0, 50.0);
        s.drag_handle(3, (80.0, 90.0));
        assert_eq!(
            s.rect(),
            D2D_RECT_F {
                left: 10.0,
                top: 10.0,
                right: 80.0,
                bottom: 90.0
            }
        );
    }
}
