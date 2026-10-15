//! The annotation editor: a toolbar along the top
//! (crop and background, the tools in one pill, the active tool's options,
//! «Guardar como…» and «Listo»), the capture in the middle, and a bottom bar
//! with the zoom, undo, the drag handle and pin / copy / save.
//!
//! The window draws its own title bar; Windows still provides the frame, the
//! shadow, snapping and resizing.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use windows::Win32::Foundation::{
    D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_BRUSH_PROPERTIES, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_ROUNDED_RECT, ID2D1Bitmap, ID2D1HwndRenderTarget, ID2D1RenderTarget, ID2D1SolidColorBrush,
    ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN,
    VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetClientRect, GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, HTBOTTOM, HTBOTTOMLEFT,
    HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IDC_ARROW,
    IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENWSE, IDNO, IDYES, IsZoomed,
    KillTimer, LoadCursorW, MB_ICONQUESTION, MB_YESNOCANCEL, MINMAXINFO, MessageBoxW,
    NCCALCSIZE_PARAMS, PostMessageW, RegisterClassExW, SM_CXDRAG, SM_CYDRAG, SW_MAXIMIZE,
    SW_MINIMIZE, SW_RESTORE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SetCursor,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_APP, WM_CHAR,
    WM_CLOSE, WM_CREATE, WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WM_SIZE,
    WM_TIMER, WNDCLASSEXW, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCWSTR, PWSTR, w};
use windows_numerics::Matrix3x2;

use crate::annot::{Kind, Obscure, Painter, Pt, Shape, TextStyle};
use crate::background::{self, Background, Fill};
use crate::gfx::{self, point, rectf, rgba};
use crate::icons::{Icon, Icons};
use crate::image::Image;
use crate::settings::ImageFormat;
use crate::theme::{self, MARK_COLORS, Palette};
use crate::util::{lparam_point, wide};
use crate::{agent, clipboard, dragdrop, flow, log};

const CLASS: PCWSTR = w!("ScreenShotCM.Editor");
const WM_EDITOR_CLOSE: u32 = WM_APP + 20;
const FLASH_TIMER: usize = 7;
/// The narrowest editor that still shows every toolbar control: crop and
/// background, twelve tools, the active tool's colour, size and style,
/// "Guardar como…", "Listo" and the window buttons. At 881 px the colour and
/// size had no room and were dropped.
const MIN_W: f32 = 1080.0;
/// The toolbar's and the bottom bar's heights, at 96 DPI.
const TOP_H: f32 = 52.0;
const BOTTOM_H: f32 = 48.0;

/// The editor's outer size for an image: 100 % when that fits in 85 % of the
/// work area, at least the toolbar's width and 480 px tall — but never more
/// than the work area itself. A portrait 1080-px monitor at 125 % has less
/// room than the minimum; a `clamp` there panicked and took the agent down.
fn frame_size(image: (f32, f32), work: (f32, f32), s: f32) -> (f32, f32) {
    let w = (image.0 + 80.0 * s)
        .min(work.0 * 0.85)
        .max((MIN_W * s).min(work.0));
    let h = (image.1 + (TOP_H + BOTTOM_H + 80.0) * s)
        .min(work.1 * 0.85)
        .max((480.0 * s).min(work.1));
    (w, h)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Draw(Kind),
    Crop,
}

/// The tools, with their names in Spanish and English: shown in a label
/// under the button the pointer rests on.
const TOOLS: [(Tool, Icon, &str, &str); 12] = [
    (Tool::Select, Icon::Select, "Seleccionar (V)", "Select (V)"),
    (
        Tool::Draw(Kind::Rect),
        Icon::Rect,
        "Rectángulo (R)",
        "Rectangle (R)",
    ),
    (
        Tool::Draw(Kind::FilledRect),
        Icon::FilledRect,
        "Rectángulo relleno (F)",
        "Filled rectangle (F)",
    ),
    (
        Tool::Draw(Kind::Ellipse),
        Icon::Ellipse,
        "Elipse (O)",
        "Ellipse (O)",
    ),
    (Tool::Draw(Kind::Line), Icon::Line, "Línea (L)", "Line (L)"),
    (
        Tool::Draw(Kind::Arrow),
        Icon::Arrow,
        "Flecha (A)",
        "Arrow (A)",
    ),
    (Tool::Draw(Kind::Text), Icon::Text, "Texto (T)", "Text (T)"),
    (
        Tool::Draw(Kind::Pixelate),
        Icon::Pixelate,
        "Ocultar datos: pixelar, desenfocar o tapar (B)",
        "Hide data: pixelate, blur or black out (B)",
    ),
    (
        Tool::Draw(Kind::Spotlight),
        Icon::Spotlight,
        "Foco (S)",
        "Spotlight (S)",
    ),
    (
        Tool::Draw(Kind::Counter),
        Icon::Counter,
        "Contador (N)",
        "Counter (N)",
    ),
    (Tool::Draw(Kind::Pen), Icon::Pen, "Lápiz (P)", "Pen (P)"),
    (
        Tool::Draw(Kind::Highlighter),
        Icon::Highlighter,
        "Resaltador (H)",
        "Highlighter (H)",
    ),
];

/// The other toolbar buttons that get a label on hover.
fn button_name(act: Act) -> Option<&'static str> {
    use crate::i18n::tr;
    Some(match act {
        Act::Tool(Tool::Crop) => tr("Recortar (C)", "Crop (C)"),
        Act::Background => tr("Fondo", "Background"),
        Act::Undo => tr("Deshacer (Ctrl+Z)", "Undo (Ctrl+Z)"),
        Act::Redo => tr("Rehacer (Ctrl+Y)", "Redo (Ctrl+Y)"),
        Act::Pin => tr("Fijar en pantalla", "Pin to screen"),
        Act::Copy => tr("Copiar (Ctrl+C)", "Copy (Ctrl+C)"),
        Act::Save => tr("Guardar (Ctrl+S)", "Save (Ctrl+S)"),
        Act::Resize => tr("Tamaño de la imagen", "Image size"),
        Act::SaveAs => tr("Guardar como…", "Save as…"),
        Act::CropApply => tr("Recortar (Intro)", "Crop (Enter)"),
        Act::Turn(Turn::CounterClockwise) => tr("Girar a la izquierda", "Rotate left"),
        Act::Turn(Turn::Clockwise) => tr("Girar a la derecha", "Rotate right"),
        Act::Turn(Turn::MirrorH) => tr("Voltear horizontal", "Flip horizontal"),
        Act::Turn(Turn::MirrorV) => tr("Voltear vertical", "Flip vertical"),
        Act::CropSnap => tr("Ajustar a los bordes", "Snap to edges"),
        Act::Drag => tr(
            "Arrastra la imagen a otra aplicación",
            "Drag the picture into another app",
        ),
        Act::Tool(t) => {
            let &(_, _, es, en) = TOOLS.iter().find(|x| x.0 == t)?;
            tr(es, en)
        }
        _ => return None,
    })
}

const STROKE_SIZES: [f32; 6] = [2.0, 4.0, 6.0, 8.0, 12.0, 16.0];
const TEXT_SIZES: [f32; 9] = [14.0, 18.0, 24.0, 30.0, 36.0, 48.0, 64.0, 72.0, 96.0];
const ZOOMS: [Option<f32>; 6] = [None, Some(0.25), Some(0.5), Some(1.0), Some(2.0), Some(4.0)];
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 8.0;
/// Ctrl + / Ctrl − and the menu's steps: a quarter, so four steps double.
const ZOOM_STEP: f32 = 1.25;
/// A crop's proportion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Aspect {
    Free,
    /// The picture's own.
    Original,
    Ratio(u16, u16),
}

const ASPECTS: [Aspect; 7] = [
    Aspect::Free,
    Aspect::Original,
    Aspect::Ratio(1, 1),
    Aspect::Ratio(4, 3),
    Aspect::Ratio(3, 2),
    Aspect::Ratio(16, 9),
    Aspect::Ratio(9, 16),
];

impl Aspect {
    /// Width over height, for a `w`×`h` picture; `None` when free.
    fn ratio(self, w: f32, h: f32) -> Option<f32> {
        match self {
            Aspect::Free => None,
            Aspect::Original => Some(w / h.max(1.0)),
            Aspect::Ratio(a, b) => Some(a as f32 / b as f32),
        }
    }

    fn label(self) -> String {
        match self {
            Aspect::Free => crate::i18n::tr("Libre", "Freeform").into(),
            Aspect::Original => crate::i18n::tr("Original", "Original").into(),
            Aspect::Ratio(1, 1) => crate::i18n::tr("Cuadrado", "Square").into(),
            Aspect::Ratio(a, b) => format!("{a}:{b}"),
        }
    }
}

/// The largest rectangle of `ratio` inside `c` and the `w`×`h` picture,
/// round `c`'s centre: what a crop becomes when a proportion is picked.
fn aspect_fit(c: D2D_RECT_F, ratio: f32, w: f32, h: f32) -> D2D_RECT_F {
    let (cw, ch) = ((c.right - c.left).max(1.0), (c.bottom - c.top).max(1.0));
    let mut nw = cw.min(ch * ratio);
    let mut nh = nw / ratio;
    if nw > w {
        (nw, nh) = (w, w / ratio);
    }
    if nh > h {
        (nw, nh) = (h * ratio, h);
    }
    let cx = ((c.left + c.right) / 2.0).clamp(nw / 2.0, w - nw / 2.0);
    let cy = ((c.top + c.bottom) / 2.0).clamp(nh / 2.0, h - nh / 2.0);
    D2D_RECT_F {
        left: cx - nw / 2.0,
        top: cy - nh / 2.0,
        right: cx + nw / 2.0,
        bottom: cy + nh / 2.0,
    }
}

/// A crop dragged from `fixed` to `to` holding `ratio`: the side that
/// moved most leads, and the whole stays inside the `w`×`h` picture.
fn aspect_corner(fixed: Pt, to: Pt, ratio: f32, w: f32, h: f32) -> D2D_RECT_F {
    let (dx, dy) = (to.0 - fixed.0, to.1 - fixed.1);
    let (mut aw, mut ah) = (dx.abs().max(1.0), dy.abs().max(1.0));
    if aw >= ah * ratio {
        ah = aw / ratio;
    } else {
        aw = ah * ratio;
    }
    let room_w = if dx >= 0.0 { w - fixed.0 } else { fixed.0 };
    let room_h = if dy >= 0.0 { h - fixed.1 } else { fixed.1 };
    let k = (room_w / aw).min(room_h / ah).clamp(0.0, 1.0);
    let (aw, ah) = (aw * k, ah * k);
    let x = fixed.0 + aw * dx.signum();
    let y = fixed.1 + ah * dy.signum();
    D2D_RECT_F {
        left: fixed.0.min(x),
        top: fixed.1.min(y),
        right: fixed.0.max(x),
        bottom: fixed.1.max(y),
    }
}

/// The sign of a way of hiding, in its menu and on its dropdown.
fn obscure_icon(o: Obscure) -> Icon {
    match o {
        Obscure::Pixelate => Icon::Pixelate,
        Obscure::SecureBlur => Icon::DropLock,
        Obscure::Blur => Icon::Drop,
        Obscure::BlackOut => Icon::FilledRect,
    }
}

/// The resize menu's ready-made sizes, as fractions of the current width.
const RESIZES: [f32; 5] = [0.25, 0.5, 0.75, 1.5, 2.0];

/// How far a picture `content` long may move off centre on a canvas
/// `canvas` long keeping `margin` at each end: nothing when it fits.
fn pan_limit(canvas: f32, content: f32, margin: f32) -> f32 {
    ((content - (canvas - 2.0 * margin)) / 2.0).max(0.0)
}

/// The size an image of `w0`×`h0` takes at width `w`, keeping its
/// proportion; `None` when that is outside what the app can hold.
fn resized_size(w0: u32, h0: u32, w: u32) -> Option<(u32, u32)> {
    let h = (h0 as f64 * w as f64 / w0.max(1) as f64).round().max(1.0) as u32;
    crate::image::within_bounds(w, h).then_some((w, h))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    Tool(Tool),
    Background,
    Color,
    Size,
    Style,
    SaveAs,
    Done,
    Min,
    Max,
    Close,
    Zoom,
    Undo,
    Redo,
    Drag,
    Pin,
    Copy,
    Save,
    BgNone,
    BgFill(Fill),
    Slider(Slider),
    // Inside a popover.
    PickColor(usize),
    PickSize(f32),
    PickTextStyle(TextStyle),
    PickObscure(Obscure),
    PickArrowCurved(bool),
    PickZoom(Option<f32>),
    /// One step in or out, about the middle of the canvas.
    ZoomStep(bool),
    /// The image's size, in the bottom bar: opens the resize menu.
    Resize,
    /// Resizes the image to this width, keeping its proportion.
    ResizeTo(u32),
    /// Resizes to the width typed in the menu.
    ResizeTyped,
    /// Every crop and resize undone at once: the picture as it opened.
    RevertOriginal,
    /// Turns or mirrors the whole picture.
    Turn(Turn),
    /// The crop's proportion: opens its menu.
    CropAspect,
    PickAspect(Aspect),
    /// Whether the crop's sides hold to the picture's edges.
    CropSnap,
    /// Crops to what is set out (as Enter does).
    CropApply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slider {
    Padding,
    Radius,
    Shadow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct R {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl R {
    fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }
    fn d2d(&self) -> D2D_RECT_F {
        rectf(self.x, self.y, self.w, self.h)
    }
    fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

#[derive(Clone, Debug)]
enum Face {
    Icon(Icon),
    Label(String),
    /// A colour swatch with a chevron.
    Swatch,
    /// A label with a chevron.
    Dropdown(String),
    /// An icon with a chevron: a choice shown by its sign, where a word
    /// would not fit.
    IconDropdown(Icon),
    Primary(String),
    Secondary(String),
    Grip(String),
    Hidden,
}

#[derive(Clone, Debug)]
struct Item {
    r: R,
    act: Act,
    face: Face,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Popover {
    Color,
    Size,
    Style,
    Zoom,
    Resize,
    Aspect,
}

#[derive(Clone)]
struct Snapshot {
    base: Rc<Image>,
    shapes: Vec<Shape>,
    background: Option<Background>,
    to_original: ToOriginal,
}

/// How a point of the picture as it now is maps to the picture it opened
/// as — an affine map, `original = (a·x + b·y + tx, c·x + d·y + ty)`. Each
/// crop, resize, turn and mirror composes the way back onto it, so
/// «Volver al original» can put the marks back where they were drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ToOriginal {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
}

impl ToOriginal {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    fn apply(self, p: Pt) -> Pt {
        (
            self.a * p.0 + self.b * p.1 + self.tx,
            self.c * p.0 + self.d * p.1 + self.ty,
        )
    }

    /// `self` after an edit whose way back is `back` (new point → the
    /// point it was before the edit).
    fn then(self, back: ToOriginal) -> Self {
        let t = self.apply((back.tx, back.ty));
        Self {
            a: self.a * back.a + self.b * back.c,
            b: self.a * back.b + self.b * back.d,
            c: self.c * back.a + self.d * back.c,
            d: self.c * back.b + self.d * back.d,
            tx: t.0,
            ty: t.1,
        }
    }

    /// After a crop whose top-left corner was at `(x, y)`.
    fn cropped(self, x: f32, y: f32) -> Self {
        self.then(Self {
            tx: x,
            ty: y,
            ..Self::IDENTITY
        })
    }

    /// After a resize by `kx`, `ky`.
    fn resized(self, kx: f32, ky: f32) -> Self {
        self.then(Self {
            a: 1.0 / kx,
            d: 1.0 / ky,
            ..Self::IDENTITY
        })
    }

    /// After `turn` was applied to a picture `w`×`h` (its size before).
    fn turned(self, turn: Turn, w: f32, h: f32) -> Self {
        self.then(turn.back(w, h))
    }

    /// How much longer a length is on the original: what a stroke's width
    /// is multiplied by on the way back.
    fn scale(self) -> f32 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }
}

/// A quarter turn or a mirror of the whole picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn {
    Clockwise,
    CounterClockwise,
    MirrorH,
    MirrorV,
}

impl Turn {
    /// Where a point of a `w`×`h` picture lands after the turn.
    fn forward(self, w: f32, h: f32) -> impl Fn(Pt) -> Pt {
        move |(x, y)| match self {
            Turn::Clockwise => (h - y, x),
            Turn::CounterClockwise => (y, w - x),
            Turn::MirrorH => (w - x, y),
            Turn::MirrorV => (x, h - y),
        }
    }

    /// The way back from the turned picture to the `w`×`h` one.
    fn back(self, w: f32, h: f32) -> ToOriginal {
        let m = |a, b, c, d, tx, ty| ToOriginal { a, b, c, d, tx, ty };
        match self {
            // (x', y') = (h - y, x)  ⇒  (x, y) = (y', h - x')
            Turn::Clockwise => m(0.0, 1.0, -1.0, 0.0, 0.0, h),
            // (x', y') = (y, w - x)  ⇒  (x, y) = (w - y', x')
            Turn::CounterClockwise => m(0.0, -1.0, 1.0, 0.0, w, 0.0),
            Turn::MirrorH => m(-1.0, 0.0, 0.0, 1.0, w, 0.0),
            Turn::MirrorV => m(1.0, 0.0, 0.0, -1.0, 0.0, h),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    /// Drawing the last shape.
    Create,
    Move {
        index: usize,
        last: Pt,
    },
    Handle {
        index: usize,
        handle: usize,
    },
    CropNew {
        start: Pt,
    },
    CropMove {
        last: Pt,
    },
    CropHandle {
        handle: usize,
    },
    Slider(Slider),
    Pan {
        last: (f32, f32),
    },
    /// Selecting inside the text being edited.
    TextSelect {
        index: usize,
    },
}

struct Gpu {
    target: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    bitmap: Option<ID2D1Bitmap>,
    /// The picture reduced to the size it is shown at (`shown_size`), by
    /// WIC, and drawn one pixel for one.
    shown: Option<((u32, u32), ID2D1Bitmap)>,
    dashed: ID2D1StrokeStyle,
    round: ID2D1StrokeStyle,
    ui: IDWriteTextFormat,
    ui_bold: IDWriteTextFormat,
}

struct Editor {
    hwnd: HWND,
    pal: Palette,
    s: f32,
    gpu: Option<Gpu>,
    painter: Painter,
    base: Rc<Image>,
    /// The crop's proportion, and whether its sides hold to edges.
    crop_aspect: Aspect,
    crop_snap: bool,
    /// The picture as the editor opened it, for «Volver al original».
    original: Rc<Image>,
    to_original: ToOriginal,
    /// `base` with every pixelate applied.
    effective: Rc<Image>,
    shapes: Vec<Shape>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The state before the change under way (`checkpoint`).
    pending: Option<Snapshot>,
    tool: Tool,
    color: usize,
    stroke: f32,
    text_size: f32,
    text_style: TextStyle,
    obscure: Obscure,
    /// New arrows are drawn curved.
    arrow_curved: bool,
    selected: Option<usize>,
    editing: Option<usize>,
    drag: Drag,
    crop: Option<D2D_RECT_F>,
    background: Option<Background>,
    /// The background the editor opened with: closing with another one
    /// makes that the next capture's.
    opened_with: Option<Background>,
    sidebar: bool,
    zoom: Option<f32>,
    pan: (f32, f32),
    mouse: (f32, f32),
    popover: Option<Popover>,
    /// The width typed in the resize menu, and whether the next digit
    /// replaces it (it starts as the current width, selected).
    resize_text: String,
    resize_fresh: bool,
    items: Vec<Item>,
    pop_items: Vec<Item>,
    /// The history file this capture came from; «Listo» writes it back.
    source: Option<PathBuf>,
    dirty: bool,
    flash: Option<String>,
    high_surrogate: Option<u16>,
    /// The text being edited: the caret and the other end of the selection,
    /// as UTF-16 offsets (equal when nothing is selected).
    caret: usize,
    anchor: usize,
    /// The button the mouse went down on: a button acts only when the
    /// release is over the same button.
    pressed: Option<Act>,
    /// Where that button was pressed: «Arrastrar» starts once the pointer
    /// leaves it by the system's drag distance.
    press_at: (f32, f32),
    /// The file «Arrastrar» hands over, written on the press: written on
    /// the move instead, a large picture took long enough that the button
    /// was up before the drag began, and Windows ended it at once.
    drag_ready: Option<PathBuf>,
    /// A text pressed with the text tool: a drag moves it, and a release
    /// where it was pressed opens it for editing there.
    edit_on_release: Option<(usize, (f32, f32))>,
    /// The highlighter stroke as the hand drew it; the shape shows it
    /// straightened while it stays nearly straight (`straight_stroke`).
    raw_stroke: Vec<Pt>,
    /// False until the window is placed and shown: a DPI change before
    /// that is Windows rescaling a window that is about to be placed.
    shown: bool,
}

/// Opens an editor on `image`. `source` is the capture's file in the history,
/// rewritten with the annotations when the user presses «Listo».
pub fn open(image: Image, source: Option<PathBuf>) {
    open_with(image, source, None);
}

/// A capture just taken: it opens on the background the editor last used.
/// A file reopened from the history or a pin may already carry one, so it
/// opens as it is (`open`).
pub fn open_capture(image: Image, source: Option<PathBuf>) {
    open_with(image, source, agent::settings().background);
}

fn open_with(image: Image, source: Option<PathBuf>, background: Option<Background>) {
    if let Err(e) = create(image, source, background) {
        log::error(&format!("editor: {e}"));
        agent::notify(
            crate::i18n::tr("No se pudo abrir el editor", "Could not open the editor"),
            &e,
            true,
        );
    }
}

fn create(
    image: Image,
    source: Option<PathBuf>,
    background: Option<Background>,
) -> Result<(), String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hIcon: crate::icon::class_icon(),
            ..Default::default()
        };
        RegisterClassExW(&class);

        // Sized to show the capture at 100 % when it fits in 85 % of the
        // monitor under the cursor, centred there.
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let (x, y, w, h) = frame(cursor, (image.width as f32, image.height as f32));

        let state = Box::new(RefCell::new(None::<Editor>));
        let raw = Box::into_raw(state);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            CLASS,
            w!("ScreenShotCM — Editor"),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            Some(raw as *const _),
        )
        .map_err(|e| {
            drop(Box::from_raw(raw));
            e.to_string()
        })?;

        let s = GetDpiForWindow(hwnd) as f32 / 96.0;
        let image_size = (image.width as f32, image.height as f32);
        // The window exists from here on: a failure destroys it (and with
        // it the state boxed into it) rather than leaving it hidden.
        let painter = match gfx::d2d().and_then(Painter::new) {
            Ok(p) => p,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(e.to_string());
            }
        };
        let base = Rc::new(image);
        let mut ed = Editor {
            hwnd,
            pal: theme::current(),
            s,
            gpu: None,
            painter,
            effective: base.clone(),
            crop_aspect: Aspect::Free,
            crop_snap: true,
            original: base.clone(),
            to_original: ToOriginal::IDENTITY,
            base,
            shapes: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            tool: Tool::Draw(Kind::Arrow),
            color: 0,
            stroke: 4.0,
            text_size: 30.0,
            text_style: TextStyle::Plain,
            obscure: Obscure::Pixelate,
            arrow_curved: false,
            selected: None,
            editing: None,
            drag: Drag::None,
            crop: None,
            background,
            opened_with: background,
            sidebar: false,
            zoom: None,
            pan: (0.0, 0.0),
            mouse: (0.0, 0.0),
            popover: None,
            resize_text: String::new(),
            resize_fresh: true,
            items: Vec::new(),
            pop_items: Vec::new(),
            source,
            dirty: false,
            flash: None,
            high_surrogate: None,
            caret: 0,
            anchor: 0,
            pressed: None,
            press_at: (0.0, 0.0),
            drag_ready: None,
            edit_on_release: None,
            raw_stroke: Vec::new(),
            shown: false,
        };
        ed.layout();
        *(*raw).borrow_mut() = Some(ed);

        // Keep Windows' shadow around a window without a caption.
        let margins = MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 1,
            cyBottomHeight: 0,
        };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED
                | SWP_NOZORDER
                | SWP_NOACTIVATE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE,
        );
        // Placed again now that the window has a DPI of its own: created on
        // a monitor of another scale, Windows rescaled it, and it opened
        // half off screen (x 3049 of 3840, 1101 wide for 881).
        place(hwnd, cursor, image_size);
        with(hwnd, |ed| ed.shown = true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
    Ok(())
}

/// Sizes the editor to show `image` at 100 % when that fits in 85 % of the
/// work area of the monitor under `cursor`, centred there, and never past
/// its edges. Measured with the window's own DPI.
fn place(hwnd: HWND, cursor: POINT, image: (f32, f32)) {
    let (x, y, w, h) = frame(cursor, image);
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// The editor's frame on the monitor under `cursor` (`frame_size`, centred
/// in the work area), in screen pixels.
fn frame(cursor: POINT, image: (f32, f32)) -> (i32, i32, i32, i32) {
    let m = crate::capture::monitor_under(cursor.x, cursor.y);
    let work = m.work;
    let (ww, wh) = (
        (work.right - work.left) as f32,
        (work.bottom - work.top) as f32,
    );
    let (w, h) = frame_size(image, (ww, wh), m.scale);
    (
        work.left + ((ww - w) / 2.0).max(0.0) as i32,
        work.top + ((wh - h) / 2.0).max(0.0) as i32,
        w as i32,
        h as i32,
    )
}

fn state(hwnd: HWND) -> Option<&'static RefCell<Option<Editor>>> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const RefCell<Option<Editor>>;
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &*ptr })
    }
}

fn with<R>(hwnd: HWND, f: impl FnOnce(&mut Editor) -> R) -> Option<R> {
    let cell = state(hwnd)?;
    let mut guard = cell.try_borrow_mut().ok()?;
    let ed = guard.as_mut()?;
    let r = f(ed);
    // Every event ends here: whatever it changed becomes one undo step.
    ed.settle();
    Some(r)
}

fn mark(c: usize) -> (u8, u8, u8) {
    MARK_COLORS[c % MARK_COLORS.len()]
}

impl Editor {
    fn client(&self) -> (f32, f32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        ((rc.right - rc.left) as f32, (rc.bottom - rc.top) as f32)
    }

    fn top_h(&self) -> f32 {
        TOP_H * self.s
    }

    fn bottom_h(&self) -> f32 {
        BOTTOM_H * self.s
    }

    fn sidebar_w(&self) -> f32 {
        if self.sidebar { 264.0 * self.s } else { 0.0 }
    }

    fn canvas(&self) -> R {
        let (w, h) = self.client();
        R {
            x: self.sidebar_w(),
            y: self.top_h(),
            w: (w - self.sidebar_w()).max(1.0),
            h: (h - self.top_h() - self.bottom_h()).max(1.0),
        }
    }

    fn padding(&self) -> f32 {
        self.background.map(|b| b.padding).unwrap_or(0.0)
    }

    /// The whole picture: the capture plus its background, in image pixels.
    fn content_size(&self) -> (f32, f32) {
        let p = self.padding();
        (
            self.base.width as f32 + 2.0 * p,
            self.base.height as f32 + 2.0 * p,
        )
    }

    fn fit_zoom(&self) -> f32 {
        let c = self.canvas();
        let (w, h) = self.content_size();
        let margin = 48.0 * self.s;
        ((c.w - margin) / w)
            .min((c.h - margin) / h)
            .clamp(0.02, 1.0)
    }

    fn z(&self) -> f32 {
        self.zoom.unwrap_or_else(|| self.fit_zoom())
    }

    /// Keeps the picture on the canvas: centred along a side where it fits,
    /// and elsewhere movable only until its edge meets the canvas's, with
    /// the margin `fit_zoom` leaves. The wheel once scrolled it out of sight.
    fn clamp_pan(&mut self) {
        let c = self.canvas();
        let (w, h) = self.content_size();
        let z = self.z();
        let margin = 24.0 * self.s;
        let lx = pan_limit(c.w, w * z, margin);
        let ly = pan_limit(c.h, h * z, margin);
        self.pan = (self.pan.0.clamp(-lx, lx), self.pan.1.clamp(-ly, ly));
    }

    /// Whether the picture is larger than the canvas along either side, so
    /// there is somewhere to move it.
    fn can_pan(&self) -> bool {
        let c = self.canvas();
        let (w, h) = self.content_size();
        let z = self.z();
        let margin = 24.0 * self.s;
        pan_limit(c.w, w * z, margin) > 0.0 || pan_limit(c.h, h * z, margin) > 0.0
    }

    /// The size, in screen pixels, a picture shown smaller than its own
    /// pixels is drawn at; `None` from 100 % up, where the bitmap itself is
    /// drawn. Direct2D's linear filter reads four pixels for each one it
    /// writes, so a reduction drops the rest: text came out uneven, some
    /// strokes thick and others faint, while the file itself was sharp.
    fn shown_size(&self) -> Option<(u32, u32)> {
        let z = self.z();
        (z < 1.0).then(|| {
            (
                ((self.effective.width as f32 * z).round() as u32).max(1),
                ((self.effective.height as f32 * z).round() as u32).max(1),
            )
        })
    }

    /// Where content (0, 0) lands in the window.
    fn origin(&self) -> (f32, f32) {
        let c = self.canvas();
        let (w, h) = self.content_size();
        let z = self.z();
        (
            (c.x + (c.w - w * z) / 2.0 + self.pan.0).round(),
            (c.y + (c.h - h * z) / 2.0 + self.pan.1).round(),
        )
    }

    /// Window → image pixels (the capture's own, excluding the background).
    fn to_image(&self, p: (f32, f32)) -> Pt {
        let (ox, oy) = self.origin();
        let z = self.z();
        let pad = self.padding();
        ((p.0 - ox) / z - pad, (p.1 - oy) / z - pad)
    }

    fn to_view(&self, p: Pt) -> (f32, f32) {
        let (ox, oy) = self.origin();
        let z = self.z();
        let pad = self.padding();
        (ox + (p.0 + pad) * z, oy + (p.1 + pad) * z)
    }

    fn current_color(&self) -> (u8, u8, u8) {
        mark(self.color)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            base: self.base.clone(),
            shapes: self.shapes.clone(),
            background: self.background,
            to_original: self.to_original,
        }
    }

    /// Remembers the state before something that may change it. It becomes
    /// an undo step only if it did change (`settle`): a click that merely
    /// selects, a slider pressed and let go, a text opened and left as it
    /// was must neither mark the capture as edited nor drop the redo list.
    fn checkpoint(&mut self) {
        if self.pending.is_none() {
            self.pending = Some(self.snapshot());
        }
    }

    /// Records what `checkpoint` remembered, once a drag or a text edit is
    /// over, if anything changed since.
    fn settle(&mut self) {
        if self.drag != Drag::None || self.editing.is_some() {
            return;
        }
        let Some(before) = self.pending.take() else {
            return;
        };
        if Rc::ptr_eq(&before.base, &self.base)
            && before.shapes == self.shapes
            && before.background == self.background
        {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn restore(&mut self, snap: Snapshot) {
        self.base = snap.base;
        self.shapes = snap.shapes;
        self.background = snap.background;
        self.to_original = snap.to_original;
        self.selected = None;
        self.editing = None;
        self.refresh_pixels();
    }

    fn undo(&mut self) {
        // A text being typed is a step of its own: close it first.
        self.commit_text();
        self.settle();
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(s);
            self.dirty = true;
        }
    }

    fn redo(&mut self) {
        self.commit_text();
        self.settle();
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(s);
            self.dirty = true;
        }
    }

    /// Re-applies pixelates; uploads the result on the next paint. Without
    /// any, the capture itself is shown — shared, not copied: a copy per undo
    /// was 400 MB each for a tall scrolling capture.
    fn refresh_pixels(&mut self) {
        let pixelated = self.shapes.iter().any(|s| s.kind == Kind::Pixelate);
        self.effective = if pixelated {
            Rc::new(crate::annot::obscured(&self.base, &self.shapes))
        } else {
            self.base.clone()
        };
        if let Some(g) = &mut self.gpu {
            g.bitmap = None;
        }
    }

    fn next_counter(&self) -> u32 {
        self.shapes
            .iter()
            .filter(|s| s.kind == Kind::Counter)
            .map(|s| s.number)
            .max()
            .unwrap_or(0)
            + 1
    }

    fn text_measure(&self, s: &Shape) -> Option<(f32, f32)> {
        (s.kind == Kind::Text).then(|| self.painter.text_size(s))
    }

    /// The topmost shape under the window point `p`.
    fn shape_at(&self, p: (f32, f32)) -> Option<usize> {
        let ip = self.to_image(p);
        let tol = 4.0 * self.s / self.z();
        (0..self.shapes.len()).rev().find(|&i| {
            let s = &self.shapes[i];
            s.hit(ip, tol, self.text_measure(s))
        })
    }

    fn handle_at(&self, p: (f32, f32)) -> Option<usize> {
        let i = self.selected?;
        let reach = 7.0 * self.s;
        self.shapes[i].handles().iter().position(|h| {
            let v = self.to_view(*h);
            (v.0 - p.0).abs() <= reach && (v.1 - p.1).abs() <= reach
        })
    }

    fn crop_handle_at(&self, p: (f32, f32)) -> Option<usize> {
        let c = self.crop?;
        let reach = 9.0 * self.s;
        [
            (c.left, c.top),
            (c.right, c.top),
            (c.left, c.bottom),
            (c.right, c.bottom),
        ]
        .iter()
        .position(|h| {
            let v = self.to_view(*h);
            (v.0 - p.0).abs() <= reach && (v.1 - p.1).abs() <= reach
        })
    }

    /// Starts editing text `i`, with the caret where `at` (window point)
    /// falls, or at the end. One undo step covers the whole edit.
    fn begin_edit(&mut self, i: usize, at: Option<(f32, f32)>) {
        self.checkpoint();
        self.selected = None;
        self.editing = Some(i);
        let end = crate::annot::textedit::len(&self.shapes[i].text);
        self.caret = at.map(|p| self.text_pos_at(i, p)).unwrap_or(end);
        self.anchor = self.caret;
    }

    /// Whether the window point `p` is on text `i` (or close to it).
    fn text_hit(&self, i: usize, p: (f32, f32)) -> bool {
        let Some(s) = self.shapes.get(i) else {
            return false;
        };
        let tol = 6.0 * self.s / self.z();
        s.hit(self.to_image(p), tol, self.text_measure(s))
    }

    /// Whether `p` is on the band around text `i` that carries its dashed
    /// frame: there the text being typed is grabbed and moved, while a
    /// press inside it places the caret. Without it, a text could not be
    /// moved while it was being edited — and it is, right after typing.
    fn text_grip(&self, i: usize, p: (f32, f32)) -> bool {
        let Some(s) = self.shapes.get(i) else {
            return false;
        };
        let b = s.bounds(self.text_measure(s));
        let a = self.to_view((b.left, b.top));
        let c = self.to_view((b.right, b.bottom));
        let within = |m: f32| p.0 >= a.0 - m && p.0 <= c.0 + m && p.1 >= a.1 - m && p.1 <= c.1 + m;
        within(10.0 * self.s) && !within(1.0 * self.s)
    }

    /// The text offset nearest the window point `p`.
    fn text_pos_at(&self, i: usize, p: (f32, f32)) -> usize {
        let Some(s) = self.shapes.get(i) else {
            return 0;
        };
        let Some(layout) = self.painter.text_layout(s) else {
            return 0;
        };
        let ip = self.to_image(p);
        let mut trailing = windows::core::BOOL(0);
        let mut inside = windows::core::BOOL(0);
        let mut m = windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            if layout
                .HitTestPoint(
                    ip.0 - s.a.0,
                    ip.1 - s.a.1,
                    &mut trailing,
                    &mut inside,
                    &mut m,
                )
                .is_err()
            {
                return self.caret;
            }
        }
        let pos = m.textPosition as usize
            + if trailing.as_bool() {
                m.length as usize
            } else {
                0
            };
        pos.min(crate::annot::textedit::len(&s.text))
    }

    /// The caret at offset `pos` of text `i`: top, x and height, in image
    /// pixels.
    fn caret_at(&self, i: usize, pos: usize) -> Option<(f32, f32, f32)> {
        let s = self.shapes.get(i)?;
        let layout = self.painter.text_layout(s)?;
        let (mut x, mut y) = (0f32, 0f32);
        let mut m = windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            layout
                .HitTestTextPosition(pos as u32, false, &mut x, &mut y, &mut m)
                .ok()?;
        }
        let h = if m.height > 0.0 {
            m.height
        } else {
            s.size * 1.33
        };
        Some((s.a.0 + x, s.a.1 + y, h))
    }

    /// The rectangles the selection covers in text `i`, in image pixels.
    fn selection_rects(&self, i: usize) -> Vec<D2D_RECT_F> {
        let (lo, hi) = (self.caret.min(self.anchor), self.caret.max(self.anchor));
        let Some(s) = self.shapes.get(i) else {
            return Vec::new();
        };
        if lo == hi {
            return Vec::new();
        }
        let Some(layout) = self.painter.text_layout(s) else {
            return Vec::new();
        };
        let mut buf =
            vec![windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS::default(); 32];
        let mut count = 0u32;
        unsafe {
            if layout
                .HitTestTextRange(
                    lo as u32,
                    (hi - lo) as u32,
                    s.a.0,
                    s.a.1,
                    Some(&mut buf),
                    &mut count,
                )
                .is_err()
            {
                return Vec::new();
            }
        }
        buf.iter()
            .take(count as usize)
            .map(|m| D2D_RECT_F {
                left: m.left,
                top: m.top,
                right: m.left + m.width,
                bottom: m.top + m.height,
            })
            .collect()
    }

    /// Replaces the selection (or inserts at the caret) with `text`.
    fn insert_text(&mut self, i: usize, text: &str) {
        let Some(s) = self.shapes.get_mut(i) else {
            return;
        };
        self.caret = crate::annot::textedit::replace(&mut s.text, self.anchor, self.caret, text);
        self.anchor = self.caret;
        self.dirty = true;
    }

    fn selected_text(&self, i: usize) -> String {
        let Some(s) = self.shapes.get(i) else {
            return String::new();
        };
        let (lo, hi) = (self.caret.min(self.anchor), self.caret.max(self.anchor));
        let (bl, bh) = (
            crate::annot::textedit::byte(&s.text, lo),
            crate::annot::textedit::byte(&s.text, hi),
        );
        s.text[bl..bh].to_string()
    }

    /// Keys while a text is being edited, as any text field takes them.
    fn edit_key(&mut self, i: usize, key: u16) {
        use crate::annot::textedit as te;
        let (c, sh) = (ctrl(), shift());
        let Some(text) = self.shapes.get(i).map(|s| s.text.clone()) else {
            return;
        };
        let has_selection = self.caret != self.anchor;
        let (lo, hi) = (self.caret.min(self.anchor), self.caret.max(self.anchor));
        // Moves the caret; Shift keeps the selection's other end.
        let go = |ed: &mut Editor, pos: usize| {
            ed.caret = pos;
            if !sh {
                ed.anchor = pos;
            }
        };
        match key {
            k if k == VK_ESCAPE.0 => self.finish_edit(),
            k if k == VK_RETURN.0 && c => self.finish_edit(),
            k if k == VK_LEFT.0 => {
                let pos = if has_selection && !sh {
                    lo
                } else {
                    te::prev(&text, self.caret, c)
                };
                go(self, pos);
            }
            k if k == VK_RIGHT.0 => {
                let pos = if has_selection && !sh {
                    hi
                } else {
                    te::next(&text, self.caret, c)
                };
                go(self, pos);
            }
            k if k == VK_HOME.0 => {
                let pos = if c {
                    0
                } else {
                    te::line_start(&text, self.caret)
                };
                go(self, pos);
            }
            k if k == VK_END.0 => {
                let pos = if c {
                    te::len(&text)
                } else {
                    te::line_end(&text, self.caret)
                };
                go(self, pos);
            }
            k if k == VK_UP.0 || k == VK_DOWN.0 => {
                // The same x on the line above or below.
                if let Some((x, y, h)) = self.caret_at(i, self.caret) {
                    let target_y = if k == VK_UP.0 {
                        y - h * 0.5
                    } else {
                        y + h * 1.5
                    };
                    let view = self.to_view((x, target_y));
                    let pos = if k == VK_UP.0 && te::line_start(&text, self.caret) == 0 {
                        0
                    } else if k == VK_DOWN.0 && te::line_end(&text, self.caret) == te::len(&text) {
                        te::len(&text)
                    } else {
                        self.text_pos_at(i, view)
                    };
                    go(self, pos);
                }
            }
            k if k == VK_BACK.0 => {
                if has_selection {
                    self.insert_text(i, "");
                } else if self.caret > 0 {
                    self.anchor = te::prev(&text, self.caret, c);
                    self.insert_text(i, "");
                }
            }
            k if k == VK_DELETE.0 => {
                if !has_selection {
                    self.anchor = te::next(&text, self.caret, c);
                }
                self.insert_text(i, "");
            }
            0x41 if c => {
                self.anchor = 0;
                self.caret = te::len(&text);
            }
            0x43 | 0x58 if c && has_selection => {
                let _ = clipboard::set_text(agent::hwnd(), &self.selected_text(i));
                if key == 0x58 {
                    self.insert_text(i, "");
                }
            }
            0x56 if c => {
                if let Some(pasted) = clipboard::get_text(agent::hwnd()) {
                    self.insert_text(i, &pasted.replace("\r\n", "\n").replace('\r', "\n"));
                }
            }
            _ => {}
        }
    }

    /// Esc, Ctrl+Enter or a press on the text's frame: typing ends and the
    /// text stays selected, so Delete removes it whole. It used to end
    /// unselected, and a text could only be emptied letter by letter.
    fn finish_edit(&mut self) {
        let Some(i) = self.editing else { return };
        let emptied = self.shapes.get(i).is_some_and(|s| s.is_degenerate());
        self.commit_text();
        if !emptied && i < self.shapes.len() {
            self.selected = Some(i);
        }
        self.layout();
    }

    fn commit_text(&mut self) {
        if let Some(i) = self.editing.take()
            && i < self.shapes.len()
            && self.shapes[i].is_degenerate()
        {
            self.shapes.remove(i);
            self.selected = None;
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        self.commit_text();
        if self.tool == Tool::Crop && tool != Tool::Crop {
            self.crop = None;
        }
        self.tool = tool;
        self.popover = None;
        if tool == Tool::Crop {
            self.selected = None;
            let (w, h) = (self.base.width as f32, self.base.height as f32);
            self.crop = Some(D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: w,
                bottom: h,
            });
            self.fit_crop_aspect();
        }
        self.layout();
    }

    /// Reshapes the crop being set out to the chosen proportion.
    fn fit_crop_aspect(&mut self) {
        let (w, h) = (self.base.width as f32, self.base.height as f32);
        if let (Some(c), Some(r)) = (self.crop, self.crop_aspect.ratio(w, h)) {
            self.crop = Some(aspect_fit(c, r, w, h));
        }
    }

    /// The edge a crop side being dragged to `at` holds to, if any — when
    /// snapping is on and the crop is free; `span` is the stretch the side
    /// covers. The reach is a few screen pixels, whatever the zoom.
    fn snap(&self, vertical: bool, at: f32, span: (f32, f32)) -> f32 {
        if !self.crop_snap || self.crop_aspect != Aspect::Free {
            return at;
        }
        let reach = 8.0 * self.s / self.z();
        self.base.edge_near(vertical, at, span, reach).unwrap_or(at)
    }

    fn apply_crop(&mut self) {
        let Some(c) = self.crop.take() else { return };
        let r = crate::util::Rect::new(
            c.left.round() as i32,
            c.top.round() as i32,
            (c.right - c.left).round() as i32,
            (c.bottom - c.top).round() as i32,
        );
        let full = crate::util::Rect::new(0, 0, self.base.width as i32, self.base.height as i32);
        let r = r.intersect(&full);
        if r.is_empty() || r == full {
            self.tool = Tool::Select;
            self.layout();
            return;
        }
        self.checkpoint();
        self.base = Rc::new(self.base.crop(r));
        self.to_original = self.to_original.cropped(r.x as f32, r.y as f32);
        for s in &mut self.shapes {
            s.translate(-r.x as f32, -r.y as f32);
        }
        self.refresh_pixels();
        self.tool = Tool::Select;
        self.zoom = None;
        self.pan = (0.0, 0.0);
        self.layout();
    }

    /// Applies a property change to the selected shape too.
    fn restyle_selected(&mut self, f: impl Fn(&mut Shape)) {
        if let Some(i) = self.selected.or(self.editing)
            && i < self.shapes.len()
        {
            self.checkpoint();
            f(&mut self.shapes[i]);
            if self.shapes[i].kind == Kind::Pixelate {
                self.refresh_pixels();
            }
        }
    }

    // --- Layout -----------------------------------------------------------

    fn active_kind(&self) -> Option<Kind> {
        match self.tool {
            Tool::Draw(k) => Some(k),
            _ => self
                .selected
                .and_then(|i| self.shapes.get(i))
                .map(|s| s.kind),
        }
    }

    fn size_label(&self) -> String {
        match self.active_kind() {
            Some(Kind::Text) => format!("{} pt", self.text_size as u32),
            _ => format!("{} px", self.stroke as u32),
        }
    }

    fn layout(&mut self) {
        self.clamp_pan();
        let s = self.s;
        let (cw, ch) = self.client();
        let mut items = Vec::new();
        let th = self.top_h();
        let btn = 32.0 * s;
        let y = (th - btn) / 2.0;
        let gap = 8.0 * s;

        // Window controls, Windows-style, at the far right.
        let ctl_w = 46.0 * s;
        let mut right = cw;
        for (act, icon) in [
            (Act::Close, Icon::Close),
            (Act::Max, Icon::Maximize),
            (Act::Min, Icon::Minimize),
        ] {
            right -= ctl_w;
            items.push(Item {
                r: R {
                    x: right,
                    y: 0.0,
                    w: ctl_w,
                    h: th * 0.62,
                },
                act,
                face: Face::Icon(icon),
            });
        }
        right -= gap;

        let done_w = 64.0 * s;
        right -= done_w;
        items.push(Item {
            r: R {
                x: right,
                y,
                w: done_w,
                h: btn,
            },
            act: Act::Done,
            face: Face::Primary(crate::i18n::tr("Listo", "Done").into()),
        });
        let save_w = 112.0 * s;
        right -= gap + save_w;
        items.push(Item {
            r: R {
                x: right,
                y,
                w: save_w,
                h: btn,
            },
            act: Act::SaveAs,
            face: Face::Secondary(crate::i18n::tr("Guardar como…", "Save as…").into()),
        });
        let right_limit = right - gap;

        let mut x = 14.0 * s;
        items.push(Item {
            r: R {
                x,
                y,
                w: btn,
                h: btn,
            },
            act: Act::Tool(Tool::Crop),
            face: Face::Icon(Icon::Crop),
        });
        x += btn + 4.0 * s;
        items.push(Item {
            r: R {
                x,
                y,
                w: btn,
                h: btn,
            },
            act: Act::Background,
            face: Face::Icon(Icon::Background),
        });
        x += btn + gap * 1.5;

        for (tool, icon, _, _) in TOOLS {
            items.push(Item {
                r: R {
                    x,
                    y,
                    w: btn,
                    h: btn,
                },
                act: Act::Tool(tool),
                face: Face::Icon(icon),
            });
            x += btn + 2.0 * s;
        }
        x += gap;

        // The active tool's options.
        let kind = self.active_kind();
        if kind.is_some() {
            let w = 48.0 * s;
            items.push(Item {
                r: R { x, y, w, h: btn },
                act: Act::Color,
                face: Face::Swatch,
            });
            x += w + 4.0 * s;
        }
        if matches!(kind, Some(k) if k != Kind::Spotlight) {
            let w = 64.0 * s;
            items.push(Item {
                r: R { x, y, w, h: btn },
                act: Act::Size,
                face: Face::Dropdown(self.size_label()),
            });
            x += w + 4.0 * s;
        }
        match kind {
            Some(Kind::Text) => {
                let label = if self.text_style == TextStyle::Plain {
                    "Aa"
                } else {
                    crate::i18n::tr("Etiqueta", "Label")
                };
                let w = 84.0 * s;
                items.push(Item {
                    r: R { x, y, w, h: btn },
                    act: Act::Style,
                    face: Face::Dropdown(label.into()),
                });
            }
            Some(Kind::Arrow) => {
                let label = if self.arrow_curved {
                    crate::i18n::tr("Curva", "Curved")
                } else {
                    crate::i18n::tr("Recta", "Straight")
                };
                let w = 84.0 * s;
                items.push(Item {
                    r: R { x, y, w, h: btn },
                    act: Act::Style,
                    face: Face::Dropdown(label.into()),
                });
            }
            // The way of hiding by its sign, as in its menu: a word here was
            // 132 px, and a narrow editor dropped the control altogether.
            Some(Kind::Pixelate) => {
                let w = 52.0 * s;
                items.push(Item {
                    r: R { x, y, w, h: btn },
                    act: Act::Style,
                    face: Face::IconDropdown(obscure_icon(self.obscure)),
                });
                x += w + 4.0 * s;
            }
            _ => {}
        }
        // The crop's options: proportion, turns, snapping, and the button
        // that crops.
        if self.tool == Tool::Crop {
            let w = 96.0 * s;
            items.push(Item {
                r: R { x, y, w, h: btn },
                act: Act::CropAspect,
                face: Face::Dropdown(self.crop_aspect.label()),
            });
            x += w + gap;
            for (act, icon) in [
                (Act::Turn(Turn::CounterClockwise), Icon::RotateLeft),
                (Act::Turn(Turn::Clockwise), Icon::RotateRight),
                (Act::Turn(Turn::MirrorH), Icon::FlipH),
                (Act::Turn(Turn::MirrorV), Icon::FlipV),
                (Act::CropSnap, Icon::Magnet),
            ] {
                items.push(Item {
                    r: R {
                        x,
                        y,
                        w: btn,
                        h: btn,
                    },
                    act,
                    face: Face::Icon(icon),
                });
                x += btn + 2.0 * s;
            }
            x += gap;
            let w = 86.0 * s;
            items.push(Item {
                r: R { x, y, w, h: btn },
                act: Act::CropApply,
                face: Face::Secondary(crate::i18n::tr("Recortar", "Crop").into()),
            });
        }
        // Short of room, «Guardar como…» gives up its words first: the
        // active tool's options matter more than a label.
        let mut right_limit = right_limit;
        // Where the left-hand controls end; the right-hand ones never count.
        // (Measured by what starts before the limit, «Recortar» — starting
        // just past it — was never counted, and was dropped.)
        let left_end = |items: &[Item]| {
            items
                .iter()
                .filter(|i| {
                    i.r.y == y
                        && !matches!(
                            i.act,
                            Act::Close | Act::Max | Act::Min | Act::Done | Act::SaveAs
                        )
                })
                .map(|i| i.r.x + i.r.w)
                .fold(0.0, f32::max)
        };
        if left_end(&items) > right_limit
            && let Some(save) = items.iter_mut().find(|i| i.act == Act::SaveAs)
        {
            let freed = save.r.w - btn;
            save.r.x += freed;
            save.r.w = btn;
            save.face = Face::Icon(Icon::Save);
            right_limit += freed;
        }
        // Then «Recortar» keeps its tick alone; Enter does the same.
        if left_end(&items) > right_limit
            && let Some(apply) = items.iter_mut().find(|i| i.act == Act::CropApply)
        {
            // Tucked against the turns: the gap before it was the last
            // few pixels it lacked.
            apply.r.x -= gap - 6.0 * s;
            apply.r.w = btn;
            apply.face = Face::Icon(Icon::Check);
        }
        // A window too narrow for every option drops the ones that collide.
        items.retain(|i| {
            i.r.x + i.r.w <= right_limit
                || matches!(
                    i.act,
                    Act::Close | Act::Max | Act::Min | Act::Done | Act::SaveAs
                )
        });

        // Bottom bar.
        let bh = self.bottom_h();
        let by = ch - bh + (bh - 30.0 * s) / 2.0;
        let small = 30.0 * s;
        let zoom_label = format!("{}%", (self.z() * 100.0).round() as u32);
        items.push(Item {
            r: R {
                x: 14.0 * s,
                y: by,
                w: 76.0 * s,
                h: small,
            },
            act: Act::Zoom,
            face: Face::Dropdown(zoom_label),
        });
        items.push(Item {
            r: R {
                x: 98.0 * s,
                y: by,
                w: small,
                h: small,
            },
            act: Act::Undo,
            face: Face::Icon(Icon::Undo),
        });
        items.push(Item {
            r: R {
                x: 98.0 * s + small + 2.0 * s,
                y: by,
                w: small,
                h: small,
            },
            act: Act::Redo,
            face: Face::Icon(Icon::Redo),
        });
        // Sized to what it says, so no gap opens before its chevron.
        let size_label = format!("{} × {}", self.base.width, self.base.height);
        items.push(Item {
            r: R {
                x: 98.0 * s + 2.0 * small + 12.0 * s,
                y: by,
                w: (size_label.chars().count() as f32 * 7.5 + 38.0).max(96.0) * s,
                h: small,
            },
            act: Act::Resize,
            face: Face::Dropdown(size_label),
        });
        let grip_w = 128.0 * s;
        items.push(Item {
            r: R {
                x: (cw - grip_w) / 2.0,
                y: by,
                w: grip_w,
                h: small,
            },
            act: Act::Drag,
            face: Face::Grip(crate::i18n::tr("Arrastrar", "Drag").into()),
        });
        let mut rx = cw - 14.0 * s;
        for (act, icon) in [
            (Act::Save, Icon::Save),
            (Act::Copy, Icon::Copy),
            (Act::Pin, Icon::Pin),
        ] {
            rx -= small;
            items.push(Item {
                r: R {
                    x: rx,
                    y: by,
                    w: small,
                    h: small,
                },
                act,
                face: Face::Icon(icon),
            });
            rx -= 6.0 * s;
        }

        // Background sidebar.
        if self.sidebar {
            let pad = 16.0 * s;
            let mut yy = th + pad;
            let full = 264.0 * s - 2.0 * pad;
            items.push(Item {
                r: R {
                    x: pad,
                    y: yy,
                    w: full,
                    h: 30.0 * s,
                },
                act: Act::BgNone,
                face: Face::Secondary(crate::i18n::tr("Sin fondo", "No background").into()),
            });
            yy += 30.0 * s + 22.0 * s + 8.0 * s;
            let cell = (full - 4.0 * 8.0 * s) / 5.0;
            for i in 0..background::GRADIENTS.len() {
                let (col, row) = ((i % 5) as f32, (i / 5) as f32);
                items.push(Item {
                    r: R {
                        x: pad + col * (cell + 8.0 * s),
                        y: yy + row * (cell + 8.0 * s),
                        w: cell,
                        h: cell,
                    },
                    act: Act::BgFill(Fill::Gradient(i)),
                    face: Face::Hidden,
                });
            }
            yy += 3.0 * (cell + 8.0 * s) + 22.0 * s + 8.0 * s;
            let dot = (full - 9.0 * 6.0 * s) / 10.0;
            for i in 0..background::COLORS.len() {
                items.push(Item {
                    r: R {
                        x: pad + i as f32 * (dot + 6.0 * s),
                        y: yy,
                        w: dot,
                        h: dot,
                    },
                    act: Act::BgFill(Fill::Color(i)),
                    face: Face::Hidden,
                });
            }
            yy += dot + 22.0 * s;
            for slider in [Slider::Padding, Slider::Radius, Slider::Shadow] {
                yy += 20.0 * s;
                items.push(Item {
                    r: R {
                        x: pad,
                        y: yy,
                        w: full,
                        h: 20.0 * s,
                    },
                    act: Act::Slider(slider),
                    face: Face::Hidden,
                });
                yy += 30.0 * s;
            }
        }
        self.items = items;
        self.layout_popover();
    }

    fn item(&self, act: Act) -> Option<&Item> {
        self.items.iter().find(|i| i.act == act)
    }

    fn layout_popover(&mut self) {
        self.pop_items.clear();
        let Some(pop) = self.popover else { return };
        let anchor = match pop {
            Popover::Color => self.item(Act::Color),
            Popover::Size => self.item(Act::Size),
            Popover::Style => self.item(Act::Style),
            Popover::Zoom => self.item(Act::Zoom),
            Popover::Resize => self.item(Act::Resize),
            Popover::Aspect => self.item(Act::CropAspect),
        };
        let Some(anchor) = anchor.map(|i| i.r) else {
            self.popover = None;
            return;
        };
        let s = self.s;
        let pad = 8.0 * s;
        let (cw, _) = self.client();
        // A popover opens under its anchor's left edge, but never past the
        // window's right one: an anchor near that edge shifts it left.
        let left = |width: f32| (anchor.x).min(cw - width - 4.0 * s).max(4.0 * s);
        let mut items = Vec::new();
        match pop {
            Popover::Color => {
                let d = 26.0 * s;
                let x0 = left(5.0 * (d + 6.0 * s) - 6.0 * s + 2.0 * pad);
                for i in 0..MARK_COLORS.len() {
                    let (c, r) = ((i % 5) as f32, (i / 5) as f32);
                    items.push(Item {
                        r: R {
                            x: x0 + pad + c * (d + 6.0 * s),
                            y: anchor.y + anchor.h + 6.0 * s + pad + r * (d + 6.0 * s),
                            w: d,
                            h: d,
                        },
                        act: Act::PickColor(i),
                        face: Face::Hidden,
                    });
                }
            }
            Popover::Size | Popover::Style | Popover::Zoom | Popover::Resize | Popover::Aspect => {
                let rows: Vec<(Act, String)> = match pop {
                    Popover::Size if self.active_kind() == Some(Kind::Text) => TEXT_SIZES
                        .iter()
                        .map(|v| (Act::PickSize(*v), format!("{} pt", *v as u32)))
                        .collect(),
                    Popover::Size => STROKE_SIZES
                        .iter()
                        .map(|v| (Act::PickSize(*v), format!("{} px", *v as u32)))
                        .collect(),
                    Popover::Style if self.active_kind() == Some(Kind::Text) => vec![
                        (
                            Act::PickTextStyle(TextStyle::Plain),
                            crate::i18n::tr("Texto (Aa)", "Text (Aa)").into(),
                        ),
                        (
                            Act::PickTextStyle(TextStyle::Label),
                            crate::i18n::tr("Etiqueta", "Label").into(),
                        ),
                    ],
                    Popover::Style if self.active_kind() == Some(Kind::Arrow) => vec![
                        (
                            Act::PickArrowCurved(false),
                            crate::i18n::tr("Flecha recta", "Straight arrow").into(),
                        ),
                        (
                            Act::PickArrowCurved(true),
                            crate::i18n::tr("Flecha curva", "Curved arrow").into(),
                        ),
                    ],
                    Popover::Style => vec![
                        (
                            Act::PickObscure(Obscure::Pixelate),
                            crate::i18n::tr("Pixelar (seguro)", "Pixelate (secure)").into(),
                        ),
                        (
                            Act::PickObscure(Obscure::SecureBlur),
                            crate::i18n::tr("Desenfocar (seguro)", "Blur (secure)").into(),
                        ),
                        (
                            Act::PickObscure(Obscure::Blur),
                            crate::i18n::tr("Desenfocar (suave)", "Blur (smooth)").into(),
                        ),
                        (
                            Act::PickObscure(Obscure::BlackOut),
                            crate::i18n::tr("Tapar en negro", "Black out").into(),
                        ),
                    ],
                    Popover::Aspect => ASPECTS
                        .iter()
                        .map(|a| (Act::PickAspect(*a), a.label()))
                        .collect(),
                    // The width is typed straight into the first row; each
                    // other row is a ready-made size, with what it makes.
                    Popover::Resize => {
                        let (w0, h0) = (self.base.width, self.base.height);
                        let typed = self.resize_text.parse::<u32>().ok();
                        let result = match typed.and_then(|w| resized_size(w0, h0, w)) {
                            Some((w, h)) => format!("{w} × {h}"),
                            None => crate::i18n::tr("fuera de límites", "out of range").into(),
                        };
                        let mut rows = vec![(
                            Act::ResizeTyped,
                            format!(
                                "{} {}\u{258F} px\t{result}",
                                crate::i18n::tr("Ancho:", "Width:"),
                                self.resize_text
                            ),
                        )];
                        if !Rc::ptr_eq(&self.base, &self.original) {
                            rows.push((
                                Act::RevertOriginal,
                                format!(
                                    "{}\t{} × {}",
                                    crate::i18n::tr("Volver al original", "Back to original"),
                                    self.original.width,
                                    self.original.height
                                ),
                            ));
                        }
                        for k in RESIZES {
                            let w = (w0 as f32 * k).round() as u32;
                            if let Some((w, h)) = resized_size(w0, h0, w) {
                                rows.push((
                                    Act::ResizeTo(w),
                                    format!("{}%\t{w} × {h}", (k * 100.0) as u32),
                                ));
                            }
                        }
                        rows
                    }
                    // A tab separates the name from its shortcut, drawn on
                    // the right: the menu is where the shortcuts are found.
                    _ => [
                        (
                            Act::ZoomStep(true),
                            format!("{}\tCtrl +", crate::i18n::tr("Acercar", "Zoom in")),
                        ),
                        (
                            Act::ZoomStep(false),
                            format!("{}\tCtrl −", crate::i18n::tr("Alejar", "Zoom out")),
                        ),
                    ]
                    .into_iter()
                    .chain(ZOOMS.iter().map(|z| {
                        let label = match z {
                            None => format!(
                                "{}\tCtrl+0",
                                crate::i18n::tr("Ajustar a la ventana", "Fit to window")
                            ),
                            Some(v) if *v == 1.0 => "100%\tCtrl+1".into(),
                            Some(v) => format!("{}%", (v * 100.0) as u32),
                        };
                        (Act::PickZoom(*z), label)
                    }))
                    .collect(),
                };
                let row_h = 28.0 * s;
                let w = match pop {
                    Popover::Zoom => 230.0,
                    Popover::Resize => 300.0,
                    Popover::Style => 200.0,
                    Popover::Aspect => 160.0,
                    _ => 170.0,
                } * s;
                let total = rows.len() as f32 * row_h + 2.0 * pad;
                // The zoom menu opens upwards from the bottom bar.
                let top = if matches!(pop, Popover::Zoom | Popover::Resize) {
                    anchor.y - 6.0 * s - total + pad
                } else {
                    anchor.y + anchor.h + 6.0 * s + pad
                };
                let x0 = left(w);
                for (i, (act, label)) in rows.into_iter().enumerate() {
                    items.push(Item {
                        r: R {
                            x: x0 + pad,
                            y: top + i as f32 * row_h,
                            w: w - 2.0 * pad,
                            h: row_h,
                        },
                        act,
                        face: Face::Label(label),
                    });
                }
            }
        }
        self.pop_items = items;
    }

    fn popover_frame(&self) -> Option<R> {
        let first = self.pop_items.first()?.r;
        let last = self.pop_items.last()?.r;
        let pad = 8.0 * self.s;
        let right = self
            .pop_items
            .iter()
            .map(|i| i.r.x + i.r.w)
            .fold(0.0, f32::max);
        Some(R {
            x: first.x - pad,
            y: first.y - pad,
            w: right - first.x + 2.0 * pad,
            h: last.y + last.h - first.y + 2.0 * pad,
        })
    }

    // --- Painting ---------------------------------------------------------

    fn ensure_gpu(&mut self) -> bool {
        if self.gpu.is_some() {
            return true;
        }
        let (w, h) = self.client();
        let make = || -> windows::core::Result<Gpu> {
            // The painter's factory: geometries and the target must share it.
            let target = gfx::window_target(
                &self.painter.factory,
                self.hwnd,
                w.max(1.0) as u32,
                h.max(1.0) as u32,
            )?;
            let brush = unsafe { target.CreateSolidColorBrush(&rgba(0, 0, 0, 1.0), None)? };
            let dashed = gfx::round_stroke(&self.painter.factory, true)?;
            // Made once here: the chrome is repainted on every mouse move.
            let round = gfx::round_stroke(&self.painter.factory, false)?;
            let ui = gfx::text_format(13.0 * self.s, DWRITE_FONT_WEIGHT_MEDIUM)?;
            let ui_bold = gfx::text_format(13.0 * self.s, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            for f in [&ui, &ui_bold] {
                unsafe {
                    let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                }
            }
            Ok(Gpu {
                target,
                brush,
                bitmap: None,
                shown: None,
                dashed,
                round,
                ui,
                ui_bold,
            })
        };
        match make() {
            Ok(g) => {
                self.gpu = Some(g);
                true
            }
            Err(e) => {
                log::error(&format!("editor surface: {e}"));
                false
            }
        }
    }

    fn paint(&mut self) {
        if !self.ensure_gpu() {
            return;
        }
        let want = self.shown_size();
        let g = self.gpu.as_mut().unwrap();
        if g.bitmap.is_none() {
            g.bitmap = gfx::bitmap(&g.target, &self.effective).ok();
            g.shown = None;
        }
        match want {
            Some(size) if g.shown.as_ref().is_none_or(|(s, _)| *s != size) => {
                g.shown = self
                    .effective
                    .resized(size.0, size.1)
                    .and_then(|small| gfx::bitmap(&g.target, &small))
                    .ok()
                    .map(|b| (size, b));
            }
            None => g.shown = None,
            _ => {}
        }
        let g = self.gpu.as_ref().unwrap();
        let t: &ID2D1RenderTarget = &g.target;
        unsafe {
            t.BeginDraw();
            t.SetTransform(&Matrix3x2::identity());
            t.Clear(Some(&self.pal.canvas));
            self.paint_canvas(t, g);
            self.paint_chrome(t, g);
            self.paint_popover(t, g);
            if let Err(e) = t.EndDraw(None, None)
                && e.code() == D2DERR_RECREATE_TARGET
            {
                self.gpu = None;
            }
        }
    }

    unsafe fn paint_canvas(&self, t: &ID2D1RenderTarget, g: &Gpu) {
        let Some(bitmap) = &g.bitmap else { return };
        let z = self.z();
        let (ox, oy) = self.origin();
        let (cw, chh) = self.content_size();
        let pad = self.padding();
        let (iw, ih) = (self.base.width as f32, self.base.height as f32);
        let canvas = self.canvas();
        unsafe {
            t.PushAxisAlignedClip(
                &canvas.d2d(),
                windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED,
            );
            let content = rectf(ox, oy, cw * z, chh * z);
            match self.background {
                Some(bg) => {
                    if let Some(brush) = background::brush(t, bg.fill, content) {
                        t.FillRectangle(&content, &brush);
                    }
                    let img = rectf(ox + pad * z, oy + pad * z, iw * z, ih * z);
                    background::shadow(
                        t,
                        &g.brush,
                        img,
                        bg.radius * z,
                        bg.shadow,
                        (pad * 0.35).max(6.0) * z,
                    );
                    self.paint_image(t, g, bitmap, img, bg.radius * z);
                }
                None => {
                    background::shadow(t, &g.brush, content, 0.0, 0.5, 14.0 * self.s);
                    self.paint_image(t, g, bitmap, content, 0.0);
                }
            }

            // Shapes, in image pixels.
            let m = Matrix3x2 {
                M11: z,
                M12: 0.0,
                M21: 0.0,
                M22: z,
                M31: ox + pad * z,
                M32: oy + pad * z,
            };
            t.SetTransform(&m);
            let clip = rectf(0.0, 0.0, iw, ih);
            t.PushAxisAlignedClip(
                &clip,
                windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
            self.painter.draw_all(t, &g.brush, &self.shapes, (iw, ih));
            // A pixelate being drawn shows its outline until it is let go.
            if self.drag == Drag::Create
                && let Some(s) = self.shapes.last().filter(|s| s.kind == Kind::Pixelate)
            {
                g.brush.SetColor(&rgba(255, 255, 255, 0.9));
                t.DrawRectangle(&s.rect(), &g.brush, 1.5 / z, &g.dashed);
            }
            t.PopAxisAlignedClip();
            t.SetTransform(&Matrix3x2::identity());

            self.paint_selection(t, g);
            self.paint_crop(t, g);
            t.PopAxisAlignedClip();
        }
    }

    unsafe fn paint_image(
        &self,
        t: &ID2D1RenderTarget,
        g: &Gpu,
        bitmap: &ID2D1Bitmap,
        r: D2D_RECT_F,
        radius: f32,
    ) {
        // Reduced: the WIC reduction, one pixel for one, on whole pixels.
        let (bitmap, r, z, interp) = match &g.shown {
            Some(((w, h), small)) => (
                small,
                rectf(r.left.round(), r.top.round(), *w as f32, *h as f32),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            ),
            None => {
                // Enlarged: whole pixels only where each becomes the same
                // number of screen pixels (200 %, 300 %…) or so large that
                // one more hardly shows (from 400 %). At 244 % nearest made
                // some columns two pixels wide and others three, and text
                // came out uneven; smoothed, as photo viewers do, it is even.
                let z = self.z();
                let whole = (z - z.round()).abs() < 0.001;
                let interp = if z >= 4.0 || (z >= 2.0 && whole) {
                    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR
                } else {
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR
                };
                (bitmap, r, z, interp)
            }
        };
        unsafe {
            if radius <= 0.5 {
                t.DrawBitmap(bitmap, Some(&r), 1.0, interp, None);
                return;
            }
            // Rounded corners: the image as a brush, filling a rounded rect.
            let Ok(brush) = t.CreateBitmapBrush(
                bitmap,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: interp,
                }),
                None,
            ) else {
                return;
            };
            brush.SetTransform(&Matrix3x2 {
                M11: z,
                M12: 0.0,
                M21: 0.0,
                M22: z,
                M31: r.left,
                M32: r.top,
            });
            t.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: r,
                    radiusX: radius,
                    radiusY: radius,
                },
                &brush,
            );
        }
    }

    unsafe fn paint_selection(&self, t: &ID2D1RenderTarget, g: &Gpu) {
        let Some(i) = self.selected.or(self.editing) else {
            return;
        };
        let Some(s) = self.shapes.get(i) else { return };
        let b = s.bounds(self.text_measure(s));
        let a = self.to_view((b.left, b.top));
        let c = self.to_view((b.right, b.bottom));
        let pad = 4.0 * self.s;
        unsafe {
            g.brush.SetColor(&self.pal.accent);
            t.DrawRectangle(
                &rectf(
                    a.0 - pad,
                    a.1 - pad,
                    c.0 - a.0 + 2.0 * pad,
                    c.1 - a.1 + 2.0 * pad,
                ),
                &g.brush,
                1.0,
                &g.dashed,
            );
            if self.editing == Some(i) {
                // The selection, painted over the glyphs it covers, then the
                // caret where the layout puts that offset.
                let mut tint = self.pal.accent;
                tint.a = 0.35;
                g.brush.SetColor(&tint);
                for r in self.selection_rects(i) {
                    let a = self.to_view((r.left, r.top));
                    let b = self.to_view((r.right, r.bottom));
                    t.FillRectangle(&rectf(a.0, a.1, b.0 - a.0, b.1 - a.1), &g.brush);
                }
                if let Some((x, y, h)) = self.caret_at(i, self.caret) {
                    let top = self.to_view((x, y));
                    let bottom = self.to_view((x, y + h));
                    g.brush.SetColor(&self.pal.accent);
                    t.DrawLine(
                        point(top.0, top.1),
                        point(bottom.0, bottom.1),
                        &g.brush,
                        2.0 * self.s,
                        None,
                    );
                }
            }
            for h in s.handles() {
                let v = self.to_view(h);
                let e = D2D1_ELLIPSE {
                    point: point(v.0, v.1),
                    radiusX: 5.0 * self.s,
                    radiusY: 5.0 * self.s,
                };
                g.brush.SetColor(&rgba(255, 255, 255, 1.0));
                t.FillEllipse(&e, &g.brush);
                g.brush.SetColor(&self.pal.accent);
                t.DrawEllipse(&e, &g.brush, 1.5 * self.s, None);
            }
        }
    }

    unsafe fn paint_crop(&self, t: &ID2D1RenderTarget, g: &Gpu) {
        let Some(c) = self.crop else { return };
        let a = self.to_view((c.left, c.top));
        let b = self.to_view((c.right, c.bottom));
        let img_a = self.to_view((0.0, 0.0));
        let img_b = self.to_view((self.base.width as f32, self.base.height as f32));
        unsafe {
            g.brush.SetColor(&rgba(0, 0, 0, 0.5));
            for r in [
                rectf(img_a.0, img_a.1, img_b.0 - img_a.0, a.1 - img_a.1),
                rectf(img_a.0, b.1, img_b.0 - img_a.0, img_b.1 - b.1),
                rectf(img_a.0, a.1, a.0 - img_a.0, b.1 - a.1),
                rectf(b.0, a.1, img_b.0 - b.0, b.1 - a.1),
            ] {
                if r.right > r.left && r.bottom > r.top {
                    t.FillRectangle(&r, &g.brush);
                }
            }
            g.brush.SetColor(&rgba(255, 255, 255, 0.95));
            t.DrawRectangle(
                &rectf(a.0, a.1, b.0 - a.0, b.1 - a.1),
                &g.brush,
                1.5 * self.s,
                None,
            );
            // Rule of thirds.
            g.brush.SetColor(&rgba(255, 255, 255, 0.45));
            for k in [1.0 / 3.0, 2.0 / 3.0] {
                let x = a.0 + (b.0 - a.0) * k;
                let y = a.1 + (b.1 - a.1) * k;
                t.DrawLine(point(x, a.1), point(x, b.1), &g.brush, 1.0, None);
                t.DrawLine(point(a.0, y), point(b.0, y), &g.brush, 1.0, None);
            }
            g.brush.SetColor(&rgba(255, 255, 255, 1.0));
            let l = 16.0 * self.s;
            let w = 4.0 * self.s;
            for (x, y, sx, sy) in [
                (a.0, a.1, 1.0, 1.0),
                (b.0, a.1, -1.0, 1.0),
                (a.0, b.1, 1.0, -1.0),
                (b.0, b.1, -1.0, -1.0),
            ] {
                t.FillRectangle(&rectf(x.min(x + sx * l), y - w / 2.0, l, w), &g.brush);
                t.FillRectangle(&rectf(x - w / 2.0, y.min(y + sy * l), w, l), &g.brush);
            }
            // Size and the way out.
            let (cw_, ch_) = (
                (c.right - c.left).round() as i32,
                (c.bottom - c.top).round() as i32,
            );
            let label = crate::i18n::trs(
                format!("{cw_} × {ch_}   ·   Intro para recortar, Esc para cancelar"),
                format!("{cw_} × {ch_}   ·   Enter to crop, Esc to cancel"),
            );
            // Below the crop, but never under the bottom bar: a crop to the
            // picture's foot put it there, out of sight.
            let canvas = self.canvas();
            let y = (b.1 + 22.0 * self.s).min(canvas.y + canvas.h - 20.0 * self.s);
            self.pill_text(t, g, &label, (a.0 + b.0) / 2.0, y);
        }
    }

    unsafe fn pill_text(&self, t: &ID2D1RenderTarget, g: &Gpu, text: &str, cx: f32, cy: f32) {
        let units: Vec<u16> = text.encode_utf16().collect();
        let w = 8.0 * self.s * text.chars().count() as f32 * 0.82 + 24.0 * self.s;
        let h = 26.0 * self.s;
        let r = rectf(cx - w / 2.0, cy - h / 2.0, w, h);
        unsafe {
            g.brush.SetColor(&rgba(20, 20, 22, 0.82));
            t.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: r,
                    radiusX: h / 2.0,
                    radiusY: h / 2.0,
                },
                &g.brush,
            );
            g.brush.SetColor(&rgba(255, 255, 255, 1.0));
            let _ = g.ui.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            crate::gfx::draw_text(t, &g.brush, &g.ui, &units, &r);
            let _ = g.ui.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        }
    }

    fn icons<'a>(&'a self, round: &'a ID2D1StrokeStyle) -> Icons<'a> {
        Icons {
            factory: &self.painter.factory,
            dwrite: &self.painter.dwrite,
            round,
        }
    }

    unsafe fn paint_chrome(&self, t: &ID2D1RenderTarget, g: &Gpu) {
        let (cw, ch) = self.client();
        let s = self.s;
        let p = &self.pal;
        let icons = self.icons(&g.round);
        unsafe {
            // Bars.
            g.brush.SetColor(&p.chrome);
            t.FillRectangle(&rectf(0.0, 0.0, cw, self.top_h()), &g.brush);
            t.FillRectangle(
                &rectf(0.0, ch - self.bottom_h(), cw, self.bottom_h()),
                &g.brush,
            );
            g.brush.SetColor(&p.chrome_line);
            t.FillRectangle(&rectf(0.0, self.top_h() - 1.0, cw, 1.0), &g.brush);
            t.FillRectangle(&rectf(0.0, ch - self.bottom_h(), cw, 1.0), &g.brush);
            if self.sidebar {
                g.brush.SetColor(&p.chrome);
                t.FillRectangle(
                    &rectf(
                        0.0,
                        self.top_h(),
                        self.sidebar_w(),
                        ch - self.top_h() - self.bottom_h(),
                    ),
                    &g.brush,
                );
                g.brush.SetColor(&p.chrome_line);
                t.FillRectangle(
                    &rectf(
                        self.sidebar_w() - 1.0,
                        self.top_h(),
                        1.0,
                        ch - self.top_h() - self.bottom_h(),
                    ),
                    &g.brush,
                );
                self.paint_sidebar(t, g);
            }

            // The grey pills behind the crop/background pair and the tools.
            let group = |acts: &[Act]| -> Option<R> {
                let rs: Vec<R> = self
                    .items
                    .iter()
                    .filter(|i| acts.contains(&i.act))
                    .map(|i| i.r)
                    .collect();
                let first = rs.first()?;
                let last = rs.last()?;
                Some(R {
                    x: first.x - 4.0 * s,
                    y: first.y - 4.0 * s,
                    w: last.x + last.w - first.x + 8.0 * s,
                    h: first.h + 8.0 * s,
                })
            };
            let tool_acts: Vec<Act> = TOOLS.iter().map(|t| Act::Tool(t.0)).collect();
            for r in [
                group(&[Act::Tool(Tool::Crop), Act::Background]),
                group(&tool_acts),
                group(&[
                    Act::Turn(Turn::CounterClockwise),
                    Act::Turn(Turn::Clockwise),
                    Act::Turn(Turn::MirrorH),
                    Act::Turn(Turn::MirrorV),
                    Act::CropSnap,
                ]),
            ]
            .into_iter()
            .flatten()
            {
                g.brush.SetColor(&p.pill);
                t.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: r.d2d(),
                        radiusX: r.h / 2.0,
                        radiusY: r.h / 2.0,
                    },
                    &g.brush,
                );
            }

            for item in &self.items {
                let hover = item.r.contains(self.mouse);
                let active = match item.act {
                    Act::Tool(tool) => self.tool == tool,
                    Act::Background => self.sidebar,
                    Act::CropSnap => self.crop_snap,
                    _ => false,
                };
                let r = item.r;
                let pill = D2D1_ROUNDED_RECT {
                    rect: r.d2d(),
                    radiusX: r.h / 2.0,
                    radiusY: r.h / 2.0,
                };
                match &item.face {
                    Face::Icon(icon) => {
                        let is_ctl = matches!(item.act, Act::Close | Act::Max | Act::Min);
                        if is_ctl {
                            if hover {
                                g.brush.SetColor(&if item.act == Act::Close {
                                    rgba(232, 17, 35, 1.0)
                                } else {
                                    p.pill_hover
                                });
                                t.FillRectangle(&r.d2d(), &g.brush);
                            }
                            let color = if hover && item.act == Act::Close {
                                rgba(255, 255, 255, 1.0)
                            } else {
                                p.icon
                            };
                            icons.draw(t, &g.brush, *icon, r.cx(), r.cy(), 12.0 * s, color);
                            continue;
                        }
                        let enabled = match item.act {
                            Act::Undo => !self.undo.is_empty(),
                            Act::Redo => !self.redo.is_empty(),
                            _ => true,
                        };
                        if active {
                            g.brush.SetColor(&p.accent);
                            t.FillRoundedRectangle(&pill, &g.brush);
                        } else if hover && enabled {
                            g.brush.SetColor(&p.pill_hover);
                            t.FillRoundedRectangle(&pill, &g.brush);
                        }
                        let mut color = if active { p.on_accent } else { p.icon };
                        if !enabled {
                            color.a = 0.3;
                        }
                        icons.draw(t, &g.brush, *icon, r.cx(), r.cy(), 18.0 * s, color);
                    }
                    // A pill like the other dropdowns beside it, the colour
                    // a dot with a hairline ring so a dark one reads on the
                    // dark chrome.
                    Face::Swatch => {
                        g.brush
                            .SetColor(if hover { &p.pill_hover } else { &p.pill });
                        t.FillRoundedRectangle(&pill, &g.brush);
                        let (cr, cg, cb) = self.current_color();
                        let dot = D2D1_ELLIPSE {
                            point: point(r.x + 16.0 * s, r.cy()),
                            radiusX: 7.0 * s,
                            radiusY: 7.0 * s,
                        };
                        g.brush.SetColor(&rgba(cr, cg, cb, 1.0));
                        t.FillEllipse(&dot, &g.brush);
                        let mut ring = p.text;
                        ring.a = 0.25;
                        g.brush.SetColor(&ring);
                        t.DrawEllipse(&dot, &g.brush, 1.0, None);
                        icons.draw(
                            t,
                            &g.brush,
                            Icon::Chevron,
                            r.x + r.w - 13.0 * s,
                            r.cy(),
                            12.0 * s,
                            p.text_dim,
                        );
                    }
                    Face::Dropdown(label) => {
                        g.brush
                            .SetColor(if hover { &p.pill_hover } else { &p.pill });
                        t.FillRoundedRectangle(&pill, &g.brush);
                        self.text(
                            t,
                            g,
                            label,
                            R {
                                x: r.x + 10.0 * s,
                                y: r.y,
                                w: r.w - 26.0 * s,
                                h: r.h,
                            },
                            p.text,
                            false,
                            false,
                        );
                        icons.draw(
                            t,
                            &g.brush,
                            Icon::Chevron,
                            r.x + r.w - 13.0 * s,
                            r.cy(),
                            12.0 * s,
                            p.text_dim,
                        );
                    }
                    Face::Primary(label) => {
                        let mut c = p.accent;
                        if hover {
                            c.r *= 0.9;
                            c.g *= 0.9;
                        }
                        g.brush.SetColor(&c);
                        t.FillRoundedRectangle(&pill, &g.brush);
                        self.text(t, g, label, r, p.on_accent, true, true);
                    }
                    Face::IconDropdown(icon) => {
                        g.brush
                            .SetColor(if hover { &p.pill_hover } else { &p.pill });
                        t.FillRoundedRectangle(&pill, &g.brush);
                        icons.draw(t, &g.brush, *icon, r.x + 17.0 * s, r.cy(), 17.0 * s, p.icon);
                        icons.draw(
                            t,
                            &g.brush,
                            Icon::Chevron,
                            r.x + r.w - 13.0 * s,
                            r.cy(),
                            12.0 * s,
                            p.text_dim,
                        );
                    }
                    Face::Secondary(label) => {
                        g.brush
                            .SetColor(if hover { &p.pill_hover } else { &p.button });
                        t.FillRoundedRectangle(&pill, &g.brush);
                        self.text(t, g, label, r, p.text, true, true);
                    }
                    Face::Grip(label) => {
                        g.brush
                            .SetColor(if hover { &p.pill_hover } else { &p.pill });
                        t.FillRoundedRectangle(&pill, &g.brush);
                        icons.draw(
                            t,
                            &g.brush,
                            Icon::Grip,
                            r.x + 16.0 * s,
                            r.cy(),
                            14.0 * s,
                            p.text_dim,
                        );
                        icons.draw(
                            t,
                            &g.brush,
                            Icon::Grip,
                            r.x + r.w - 16.0 * s,
                            r.cy(),
                            14.0 * s,
                            p.text_dim,
                        );
                        self.text(t, g, label, r, p.text, true, true);
                    }
                    Face::Label(label) => self.text(t, g, label, r, p.text, false, false),
                    Face::Hidden => {}
                }
            }

            // A flash message ("Copiado") over the bottom bar's centre.
            if let Some(msg) = &self.flash {
                self.pill_text(t, g, msg, cw / 2.0, ch - self.bottom_h() - 24.0 * s);
            }

            // The name of the button under the pointer: below a toolbar
            // button, above one in the bottom bar.
            if self.popover.is_none() && self.drag == Drag::None {
                let hovered = self.items.iter().find(|i| i.r.contains(self.mouse));
                if let Some((item, name)) = hovered.and_then(|i| button_name(i.act).map(|n| (i, n)))
                {
                    let below = item.r.y < ch / 2.0;
                    let y = if below {
                        item.r.y + item.r.h + 20.0 * s
                    } else {
                        item.r.y - 20.0 * s
                    };
                    // Kept inside the window at its edges.
                    let half = 8.0 * s * name.chars().count() as f32 * 0.82 / 2.0 + 24.0 * s;
                    let x = item.r.cx().clamp(half, cw - half);
                    self.pill_text(t, g, name, x, y);
                }
            }
        }
    }

    unsafe fn paint_sidebar(&self, t: &ID2D1RenderTarget, g: &Gpu) {
        let s = self.s;
        let p = &self.pal;
        let pad = 16.0 * s;
        let current = self.background.map(|b| b.fill);
        unsafe {
            let heading = |label: &str, y: f32| {
                self.text(
                    t,
                    g,
                    label,
                    R {
                        x: pad,
                        y,
                        w: 200.0 * s,
                        h: 18.0 * s,
                    },
                    p.text_dim,
                    true,
                    false,
                )
            };
            let first_grad = self
                .items
                .iter()
                .find(|i| matches!(i.act, Act::BgFill(Fill::Gradient(0))))
                .map(|i| i.r);
            let first_col = self
                .items
                .iter()
                .find(|i| matches!(i.act, Act::BgFill(Fill::Color(0))))
                .map(|i| i.r);
            if let Some(r) = first_grad {
                heading(crate::i18n::tr("Degradados", "Gradients"), r.y - 24.0 * s);
            }
            if let Some(r) = first_col {
                heading(crate::i18n::tr("Colores", "Colors"), r.y - 24.0 * s);
            }
            for item in &self.items {
                match item.act {
                    Act::BgFill(fill) => {
                        let r = item.r;
                        let shape = D2D1_ROUNDED_RECT {
                            rect: r.d2d(),
                            radiusX: 8.0 * s,
                            radiusY: 8.0 * s,
                        };
                        let shape = if matches!(fill, Fill::Color(_)) {
                            D2D1_ROUNDED_RECT {
                                rect: r.d2d(),
                                radiusX: r.w / 2.0,
                                radiusY: r.h / 2.0,
                            }
                        } else {
                            shape
                        };
                        if let Some(brush) = background::brush(t, fill, r.d2d()) {
                            t.FillRoundedRectangle(&shape, &brush);
                        }
                        g.brush.SetColor(&rgba(0, 0, 0, 0.12));
                        t.DrawRoundedRectangle(&shape, &g.brush, 1.0, None);
                        if current == Some(fill) {
                            g.brush.SetColor(&p.accent);
                            let ring = D2D1_ROUNDED_RECT {
                                rect: rectf(
                                    r.x - 3.0 * s,
                                    r.y - 3.0 * s,
                                    r.w + 6.0 * s,
                                    r.h + 6.0 * s,
                                ),
                                radiusX: shape.radiusX + 3.0 * s,
                                radiusY: shape.radiusY + 3.0 * s,
                            };
                            t.DrawRoundedRectangle(&ring, &g.brush, 2.0 * s, None);
                        }
                    }
                    Act::Slider(slider) => {
                        let bg = self.background.unwrap_or_default();
                        let (label, value, max) = match slider {
                            Slider::Padding => {
                                (crate::i18n::tr("Margen", "Padding"), bg.padding, 240.0)
                            }
                            Slider::Radius => {
                                (crate::i18n::tr("Redondeo", "Rounding"), bg.radius, 48.0)
                            }
                            Slider::Shadow => (crate::i18n::tr("Sombra", "Shadow"), bg.shadow, 1.0),
                        };
                        let r = item.r;
                        self.text(
                            t,
                            g,
                            label,
                            R {
                                x: r.x,
                                y: r.y - 20.0 * s,
                                w: r.w,
                                h: 18.0 * s,
                            },
                            p.text_dim,
                            true,
                            false,
                        );
                        let track = rectf(r.x, r.cy() - 2.0 * s, r.w, 4.0 * s);
                        g.brush.SetColor(&p.pill_hover);
                        t.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: track,
                                radiusX: 2.0 * s,
                                radiusY: 2.0 * s,
                            },
                            &g.brush,
                        );
                        let f = (value / max).clamp(0.0, 1.0);
                        g.brush.SetColor(&p.accent);
                        t.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: rectf(r.x, r.cy() - 2.0 * s, r.w * f, 4.0 * s),
                                radiusX: 2.0 * s,
                                radiusY: 2.0 * s,
                            },
                            &g.brush,
                        );
                        let knob = D2D1_ELLIPSE {
                            point: point(r.x + r.w * f, r.cy()),
                            radiusX: 8.0 * s,
                            radiusY: 8.0 * s,
                        };
                        g.brush.SetColor(&rgba(255, 255, 255, 1.0));
                        t.FillEllipse(&knob, &g.brush);
                        g.brush.SetColor(&rgba(0, 0, 0, 0.25));
                        t.DrawEllipse(&knob, &g.brush, 1.0, None);
                    }
                    _ => {}
                }
            }
        }
    }

    unsafe fn paint_popover(&self, t: &ID2D1RenderTarget, g: &Gpu) {
        let Some(frame) = self.popover_frame() else {
            return;
        };
        let s = self.s;
        let p = &self.pal;
        unsafe {
            background::shadow(t, &g.brush, frame.d2d(), 10.0 * s, 0.7, 10.0 * s);
            g.brush.SetColor(&p.popover);
            let shape = D2D1_ROUNDED_RECT {
                rect: frame.d2d(),
                radiusX: 10.0 * s,
                radiusY: 10.0 * s,
            };
            t.FillRoundedRectangle(&shape, &g.brush);
            g.brush.SetColor(&p.chrome_line);
            t.DrawRoundedRectangle(&shape, &g.brush, 1.0, None);
            for item in &self.pop_items {
                let r = item.r;
                let hover = r.contains(self.mouse);
                match item.act {
                    Act::PickColor(i) => {
                        let (cr, cg, cb) = mark(i);
                        let e = D2D1_ELLIPSE {
                            point: point(r.cx(), r.cy()),
                            radiusX: r.w / 2.0 - 2.0 * s,
                            radiusY: r.h / 2.0 - 2.0 * s,
                        };
                        g.brush.SetColor(&rgba(cr, cg, cb, 1.0));
                        t.FillEllipse(&e, &g.brush);
                        // A ring in the text colour: black on the dark
                        // popover and white on the light one were invisible.
                        let mut ring = p.text;
                        ring.a = 0.25;
                        g.brush.SetColor(&ring);
                        t.DrawEllipse(&e, &g.brush, 1.0, None);
                        if i == self.color || hover {
                            g.brush.SetColor(&p.accent);
                            let ring = D2D1_ELLIPSE {
                                point: e.point,
                                radiusX: r.w / 2.0 + 1.0 * s,
                                radiusY: r.h / 2.0 + 1.0 * s,
                            };
                            t.DrawEllipse(&ring, &g.brush, 2.0 * s, None);
                        }
                    }
                    _ => {
                        let selected = match item.act {
                            Act::PickSize(v) => {
                                (if self.active_kind() == Some(Kind::Text) {
                                    self.text_size
                                } else {
                                    self.stroke
                                }) == v
                            }
                            Act::PickTextStyle(v) => self.text_style == v,
                            Act::PickObscure(v) => self.obscure == v,
                            Act::PickArrowCurved(v) => self.arrow_curved == v,
                            Act::PickZoom(v) => self.zoom == v,
                            Act::PickAspect(v) => self.crop_aspect == v,
                            _ => false,
                        };
                        if hover || selected {
                            g.brush
                                .SetColor(if selected { &p.accent } else { &p.pill_hover });
                            t.FillRoundedRectangle(
                                &D2D1_ROUNDED_RECT {
                                    rect: r.d2d(),
                                    radiusX: 6.0 * s,
                                    radiusY: 6.0 * s,
                                },
                                &g.brush,
                            );
                        }
                        if let Face::Label(label) = &item.face {
                            let color = if selected { p.on_accent } else { p.text };
                            let inner = R {
                                x: r.x + 10.0 * s,
                                y: r.y,
                                w: r.w - 20.0 * s,
                                h: r.h,
                            };
                            let (name, keys) = label.split_once('\t').unwrap_or((label, ""));
                            // A row that names a way of hiding shows its sign.
                            let sign = match item.act {
                                Act::PickObscure(o) => Some(obscure_icon(o)),
                                _ => None,
                            };
                            let mut text_at = inner;
                            if let Some(icon) = sign {
                                let ink = if selected { color } else { p.text_dim };
                                self.icons(&g.round).draw(
                                    t,
                                    &g.brush,
                                    icon,
                                    inner.x + 8.0 * s,
                                    r.cy(),
                                    16.0 * s,
                                    ink,
                                );
                                text_at.x += 26.0 * s;
                                text_at.w -= 26.0 * s;
                            }
                            self.text(t, g, name, text_at, color, false, false);
                            if !keys.is_empty() {
                                let _ = g.ui.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING);
                                g.brush
                                    .SetColor(if selected { &p.on_accent } else { &p.text_dim });
                                crate::gfx::draw_str(t, &g.brush, &g.ui, keys, &inner.d2d());
                                let _ = g.ui.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
                            }
                        }
                    }
                }
            }
        }
    }

    unsafe fn text(
        &self,
        t: &ID2D1RenderTarget,
        g: &Gpu,
        label: &str,
        r: R,
        color: D2D1_COLOR_F,
        bold: bool,
        centred: bool,
    ) {
        let units: Vec<u16> = label.encode_utf16().collect();
        let format = if bold { &g.ui_bold } else { &g.ui };
        unsafe {
            let _ = format.SetTextAlignment(if centred {
                DWRITE_TEXT_ALIGNMENT_CENTER
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            });
            g.brush.SetColor(&color);
            crate::gfx::draw_text(t, &g.brush, format, &units, &r.d2d());
        }
    }

    // --- Export -------------------------------------------------------------

    /// The finished picture: background, capture and annotations, at the
    /// capture's own resolution.
    fn export(&self) -> Result<Image, String> {
        let (cw, ch) = self.content_size();
        let (w, h) = (cw.round() as u32, ch.round() as u32);
        let pad = self.padding();
        let (iw, ih) = (self.base.width as f32, self.base.height as f32);
        // A picture that could not be uploaded must fail the export, not
        // come out blank: «Listo» would write the blank over the capture.
        let mut uploaded = true;
        let out = crate::render::offscreen(&self.painter.factory, w, h, |t, brush| unsafe {
            t.Clear(Some(&rgba(0, 0, 0, 0.0)));
            let Ok(bitmap) = gfx::bitmap(t, &self.effective) else {
                uploaded = false;
                return;
            };
            let img = rectf(pad, pad, iw, ih);
            match self.background {
                Some(bg) => {
                    if let Some(b) = background::brush(t, bg.fill, rectf(0.0, 0.0, cw, ch)) {
                        t.FillRectangle(&rectf(0.0, 0.0, cw, ch), &b);
                    }
                    background::shadow(t, brush, img, bg.radius, bg.shadow, (pad * 0.35).max(6.0));
                    if bg.radius > 0.5 {
                        if let Ok(b) = t.CreateBitmapBrush(
                            &bitmap,
                            Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                            }),
                            None,
                        ) {
                            b.SetTransform(&Matrix3x2 {
                                M11: 1.0,
                                M12: 0.0,
                                M21: 0.0,
                                M22: 1.0,
                                M31: pad,
                                M32: pad,
                            });
                            t.FillRoundedRectangle(
                                &D2D1_ROUNDED_RECT {
                                    rect: img,
                                    radiusX: bg.radius,
                                    radiusY: bg.radius,
                                },
                                &b,
                            );
                        }
                    } else {
                        t.DrawBitmap(
                            &bitmap,
                            Some(&img),
                            1.0,
                            D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                            None,
                        );
                    }
                }
                None => t.DrawBitmap(
                    &bitmap,
                    Some(&img),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                    None,
                ),
            }
            t.SetTransform(&Matrix3x2 {
                M11: 1.0,
                M12: 0.0,
                M21: 0.0,
                M22: 1.0,
                M31: pad,
                M32: pad,
            });
            t.PushAxisAlignedClip(
                &rectf(0.0, 0.0, iw, ih),
                windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
            self.painter.draw_all(t, brush, &self.shapes, (iw, ih));
            t.PopAxisAlignedClip();
        })
        .map_err(|e| e.to_string())?;
        if !uploaded {
            return Err(crate::i18n::tr(
                "la imagen es demasiado grande para dibujarla",
                "the image is too large to draw",
            )
            .into());
        }
        Ok(out)
    }
}

// --- Actions --------------------------------------------------------------------

/// What an action leaves to do once the editor is no longer borrowed:
/// anything that resizes the window or runs a modal loop sends messages
/// back to it, and those find the state still borrowed otherwise (the
/// Maximize button left the layout at the old size).
enum After {
    Nothing,
    Close,
    AskClose,
    Minimize,
    Maximize,
    SaveAs,
}

fn act(hwnd: HWND, action: Act) {
    let after = with(hwnd, |ed| ed.act(action)).unwrap_or(After::Nothing);
    match after {
        After::Nothing => {}
        After::Close => close(hwnd),
        After::AskClose => ask_close(hwnd),
        After::Minimize => unsafe {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        },
        After::Maximize => unsafe {
            let zoomed = IsZoomed(hwnd).as_bool();
            let _ = ShowWindow(hwnd, if zoomed { SW_RESTORE } else { SW_MAXIMIZE });
        },
        After::SaveAs => save_as(hwnd),
    }
    invalidate(hwnd);
}

fn close(hwnd: HWND) {
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_EDITOR_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// Closing with unsaved annotations asks first; the question is a modal
/// loop, so it runs with the editor released.
fn ask_close(hwnd: HWND) {
    let dirty = with(hwnd, |ed| {
        ed.commit_text();
        ed.dirty
    })
    .unwrap_or(false);
    if !dirty {
        close(hwnd);
        return;
    }
    let question = windows::core::HSTRING::from(crate::i18n::tr(
        "¿Guardar las anotaciones en la captura antes de cerrar?",
        "Save the annotations in the capture before closing?",
    ));
    let answer = unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(question.as_ptr()),
            w!("ScreenShotCM"),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        )
    };
    if answer == IDNO {
        close(hwnd);
    } else if answer == IDYES {
        match with(hwnd, |ed| ed.finish()) {
            Some(Ok(())) => close(hwnd),
            Some(Err(e)) => agent::notify(
                crate::i18n::tr("No se pudo guardar", "Could not save"),
                &e,
                true,
            ),
            None => {}
        }
    }
}

impl Editor {
    fn act(&mut self, action: Act) -> After {
        let s = self.s;
        match action {
            Act::Tool(tool) => self.set_tool(tool),
            Act::Background => {
                self.sidebar = !self.sidebar;
                if self.sidebar && self.background.is_none() {
                    self.checkpoint();
                    self.background = Some(agent::settings().background.unwrap_or_default());
                }
                self.layout();
            }
            Act::BgNone => {
                if self.background.is_some() {
                    self.checkpoint();
                    self.background = None;
                }
            }
            Act::BgFill(fill) => {
                self.checkpoint();
                let mut bg = self.background.unwrap_or_default();
                bg.fill = fill;
                self.background = Some(bg);
            }
            Act::Color | Act::Size | Act::Style | Act::Zoom | Act::Resize | Act::CropAspect => {
                let pop = match action {
                    Act::Color => Popover::Color,
                    Act::Size => Popover::Size,
                    Act::Style => Popover::Style,
                    Act::Resize => Popover::Resize,
                    Act::CropAspect => Popover::Aspect,
                    _ => Popover::Zoom,
                };
                if pop == Popover::Resize {
                    self.resize_text = self.base.width.to_string();
                    self.resize_fresh = true;
                }
                self.popover = if self.popover == Some(pop) {
                    None
                } else {
                    Some(pop)
                };
                self.layout_popover();
            }
            Act::PickColor(i) => {
                self.color = i;
                let c = mark(i);
                self.restyle_selected(|sh| sh.color = c);
                self.popover = None;
            }
            Act::PickSize(v) => {
                if self.active_kind() == Some(Kind::Text) {
                    self.text_size = v;
                } else {
                    self.stroke = v;
                }
                self.restyle_selected(|sh| sh.size = v);
                self.popover = None;
                self.layout();
            }
            Act::PickTextStyle(v) => {
                self.text_style = v;
                self.restyle_selected(|sh| sh.text_style = v);
                self.popover = None;
                self.layout();
            }
            Act::PickArrowCurved(v) => {
                self.arrow_curved = v;
                self.restyle_selected(|sh| {
                    if sh.kind == Kind::Arrow {
                        sh.ctrl = v.then(|| crate::annot::default_bend(sh.a, sh.b));
                    }
                });
                self.popover = None;
                self.layout();
            }
            Act::PickObscure(v) => {
                self.obscure = v;
                self.restyle_selected(|sh| sh.obscure = v);
                self.popover = None;
                self.layout();
            }
            Act::PickZoom(z) => {
                self.zoom = z;
                self.pan = (0.0, 0.0);
                self.popover = None;
                self.layout();
            }
            Act::ResizeTo(w) => self.resize_to(w),
            Act::RevertOriginal => self.revert_to_original(),
            Act::Turn(turn) => self.turn(turn),
            Act::PickAspect(a) => {
                self.crop_aspect = a;
                self.fit_crop_aspect();
                self.popover = None;
                self.layout();
            }
            Act::CropSnap => {
                self.crop_snap = !self.crop_snap;
                self.layout();
            }
            Act::CropApply => self.apply_crop(),
            Act::ResizeTyped => {
                if let Ok(w) = self.resize_text.parse::<u32>() {
                    self.resize_to(w);
                }
            }
            Act::ZoomStep(closer) => {
                let c = self.canvas();
                self.zoom_about(
                    if closer { ZOOM_STEP } else { 1.0 / ZOOM_STEP },
                    (c.x + c.w / 2.0, c.y + c.h / 2.0),
                );
                self.layout();
            }
            Act::Undo => {
                self.undo();
                self.layout();
            }
            Act::Redo => {
                self.redo();
                self.layout();
            }
            Act::Min => return After::Minimize,
            Act::Max => return After::Maximize,
            Act::Close => return After::AskClose,
            Act::Done => {
                self.commit_text();
                if self.tool == Tool::Crop {
                    self.apply_crop();
                }
                return match self.finish() {
                    Ok(()) => After::Close,
                    Err(e) => {
                        agent::notify(
                            crate::i18n::tr("No se pudo guardar", "Could not save"),
                            &e,
                            true,
                        );
                        After::Nothing
                    }
                };
            }
            Act::SaveAs => return After::SaveAs,
            // A drag starts on a move (`drag_started`); released where it
            // was pressed, it was a click, and the button says how.
            Act::Drag => self.flash(crate::i18n::tr(
                "Mantén pulsado y arrastra a otra aplicación",
                "Hold and drag into another app",
            )),
            Act::Copy => match self.export() {
                Ok(img) => match clipboard::copy_image(agent::hwnd(), &img) {
                    Ok(()) => self.flash(crate::i18n::tr(
                        "Copiado al portapapeles",
                        "Copied to the clipboard",
                    )),
                    Err(e) => self.flash(&crate::i18n::trs(
                        format!("No se pudo copiar: {e}"),
                        format!("Could not copy: {e}"),
                    )),
                },
                Err(e) => self.flash(&crate::i18n::trs(
                    format!("No se pudo exportar: {e}"),
                    format!("Could not export: {e}"),
                )),
            },
            Act::Pin => {
                if let Ok(img) = self.export() {
                    crate::pin::open(img, None);
                }
            }
            Act::Save => match self.export() {
                Ok(img) => match flow::save_to_folder(&img, &agent::settings()) {
                    Ok(path) => {
                        self.dirty = false;
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        self.flash(&crate::i18n::trs(
                            format!("Guardado: {name}"),
                            format!("Saved: {name}"),
                        ));
                    }
                    Err(e) => self.flash(&crate::i18n::trs(
                        format!("No se pudo guardar: {e}"),
                        format!("Could not save: {e}"),
                    )),
                },
                Err(e) => self.flash(&crate::i18n::trs(
                    format!("No se pudo exportar: {e}"),
                    format!("Could not export: {e}"),
                )),
            },
            Act::Slider(_) => {}
        }
        let _ = s;
        After::Nothing
    }

    /// Whether the pointer, pressed on «Arrastrar», has moved far enough
    /// to start dragging the picture out. The drag must start while the
    /// button is down: started on release, Windows ended it at once.
    fn drag_started(&mut self, p: (f32, f32)) -> bool {
        if self.pressed != Some(Act::Drag) {
            return false;
        }
        let (dx, dy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
        let moved =
            (p.0 - self.press_at.0).abs() > dx as f32 || (p.1 - self.press_at.1).abs() > dy as f32;
        if moved {
            self.pressed = None;
        }
        moved
    }

    fn flash(&mut self, msg: &str) {
        self.flash = Some(msg.to_string());
        unsafe {
            SetTimer(Some(self.hwnd), FLASH_TIMER, 1800, None);
        }
    }

    /// «Listo»: the annotated capture goes to the clipboard when the user
    /// copies captures and to its quick-access card, and replaces the
    /// original in the history — or joins it, when the history is kept.
    /// With the history off nothing is written.
    fn finish(&mut self) -> Result<(), String> {
        if !self.dirty && self.source.is_some() {
            return Ok(());
        }
        let img = self.export()?;
        let settings = agent::settings();
        if settings.copy_after_capture {
            let _ = clipboard::copy_image(agent::hwnd(), &img);
        }
        let path = match &self.source {
            Some(p) => Some(p.clone()),
            None if settings.keep_history => {
                let p = crate::history::new_path();
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                Some(p)
            }
            None => None,
        };
        if let Some(path) = &path {
            crate::image::save(&img, path, ImageFormat::Png, 100).map_err(|e| e.to_string())?;
        }
        crate::quick::replaced(path.as_deref(), img);
        self.dirty = false;
        Ok(())
    }
}

/// «Guardar como…»: the common save dialog, PNG or JPEG.
fn save_as(hwnd: HWND) {
    let Some(img) = with(hwnd, |ed| {
        ed.commit_text();
        ed.export()
    }) else {
        return;
    };
    let img = match img {
        Ok(i) => i,
        Err(e) => {
            agent::notify(
                crate::i18n::tr("No se pudo exportar", "Could not export"),
                &e,
                true,
            );
            return;
        }
    };
    let settings = agent::settings();
    let suggested = flow::save_path(&settings);
    let mut buffer = [0u16; 1024];
    let name: Vec<u16> = suggested
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .encode_utf16()
        .collect();
    buffer[..name.len().min(1000)].copy_from_slice(&name[..name.len().min(1000)]);
    let (png, jpeg) = (
        crate::i18n::tr("Imagen PNG", "PNG image"),
        crate::i18n::tr("Imagen JPEG", "JPEG image"),
    );
    let filter: Vec<u16> = format!("{png} (*.png)\0*.png\0{jpeg} (*.jpg)\0*.jpg;*.jpeg\0\0")
        .encode_utf16()
        .collect();
    let dir = wide(&settings.save_dir);
    let _ = std::fs::create_dir_all(&settings.save_dir);
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        nFilterIndex: if settings.image_format == ImageFormat::Jpeg {
            2
        } else {
            1
        },
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrInitialDir: PCWSTR(dir.as_ptr()),
        lpstrDefExt: w!("png"),
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if !unsafe { GetSaveFileNameW(&mut ofn) }.as_bool() {
        return;
    }
    let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    let path = PathBuf::from(String::from_utf16_lossy(&buffer[..len]));
    let jpeg = matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("jpg" | "jpeg")
    );
    let format = if jpeg {
        ImageFormat::Jpeg
    } else {
        ImageFormat::Png
    };
    match crate::image::save(&img, &path, format, settings.jpeg_quality) {
        Ok(()) => {
            with(hwnd, |ed| {
                ed.dirty = false;
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ed.flash(&crate::i18n::trs(
                    format!("Guardado: {name}"),
                    format!("Saved: {name}"),
                ));
            });
        }
        Err(e) => agent::notify(
            crate::i18n::tr("No se pudo guardar", "Could not save"),
            &e.to_string(),
            true,
        ),
    }
}

/// «Arrastrar»: the picture as a file, dragged into whatever takes files.
/// Writes the finished picture where «Arrastrar» hands it over from.
fn prepare_drag(hwnd: HWND) -> Option<PathBuf> {
    let img = match with(hwnd, |ed| {
        ed.commit_text();
        ed.export()
    })? {
        Ok(img) => img,
        Err(e) => {
            log::error(&format!("drag out: {e}"));
            return None;
        }
    };
    let dir = dragdrop::staging();
    let name = flow::save_path(&agent::settings())
        .file_name()
        .map(|n| n.to_owned())
        .unwrap_or_default();
    let path = dir.join(name).with_extension("png");
    match crate::image::save(&img, &path, ImageFormat::Png, 100) {
        Ok(()) => Some(path),
        Err(e) => {
            log::error(&format!("drag out: {e}"));
            None
        }
    }
}

/// Keeps the background an editor closed with for the next capture: in
/// the settings file, so it outlives the agent.
fn remember_background(background: Option<Background>) {
    let mut s = agent::settings();
    if s.background == background {
        return;
    }
    s.background = background;
    match s.save() {
        Ok(()) => agent::replace_settings(s),
        Err(e) => log::error(&format!("remember the background: {e}")),
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

// --- Input -------------------------------------------------------------------------

fn shift() -> bool {
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
}

fn ctrl() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

/// Space held turns any tool into the hand, as in image editors.
fn space() -> bool {
    unsafe { GetKeyState(VK_SPACE.0 as i32) < 0 }
}

/// Shift snaps a line to 45° steps and a box to a square.
fn constrain(kind: Kind, a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    match kind {
        Kind::Line | Kind::Arrow => {
            let len = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx);
            let step = std::f32::consts::FRAC_PI_4;
            let snapped = (angle / step).round() * step;
            (a.0 + len * snapped.cos(), a.1 + len * snapped.sin())
        }
        _ => {
            let side = dx.abs().max(dy.abs());
            (a.0 + side * dx.signum(), a.1 + side * dy.signum())
        }
    }
}

impl Editor {
    fn on_down(&mut self, p: (f32, f32), double: bool) {
        self.mouse = p;
        // A press that closes a popover through its own button only closes it.
        self.pressed = None;
        // A popover takes the click first, and closes on a click outside it.
        if self.popover.is_some() {
            if let Some(item) = self.pop_items.iter().find(|i| i.r.contains(p)).cloned() {
                let _ = self.act(item.act);
                self.layout();
                return;
            }
            self.popover = None;
            self.pop_items.clear();
            if self.items.iter().any(|i| {
                i.r.contains(p)
                    && matches!(
                        i.act,
                        Act::Color
                            | Act::Size
                            | Act::Style
                            | Act::Zoom
                            | Act::Resize
                            | Act::CropAspect
                    )
            }) {
                return;
            }
        }
        if let Some(item) = self.items.iter().find(|i| i.r.contains(p)).cloned() {
            if let Act::Slider(slider) = item.act {
                self.checkpoint();
                self.drag = Drag::Slider(slider);
                self.slide(slider, p.0);
            } else {
                self.pressed = Some(item.act);
                self.press_at = p;
            }
            return; // Buttons act on release; «Arrastrar» on a move.
        }
        if !self.canvas().contains(p) {
            return;
        }
        let ip = self.to_image(p);

        self.edit_on_release = None;
        // The frame of the text being edited takes it as an object: typing
        // ends, the text is selected — so Delete removes it whole — and a
        // drag moves it. Inside, a press still places the caret.
        if let Some(i) = self.editing
            && self.text_grip(i, p)
        {
            self.finish_edit();
            if self.selected == Some(i) {
                self.checkpoint();
                self.drag = Drag::Move { index: i, last: ip };
            }
            return;
        }

        // Inside the text being edited, a click places the caret (Shift
        // extends the selection, a double click takes the word) and a drag
        // selects.
        if let Some(i) = self.editing
            && self.text_hit(i, p)
        {
            let pos = self.text_pos_at(i, p);
            if double {
                let (a, b) = crate::annot::textedit::word_at(&self.shapes[i].text, pos);
                self.anchor = a;
                self.caret = b;
            } else {
                self.caret = pos;
                if !shift() {
                    self.anchor = pos;
                }
                self.drag = Drag::TextSelect { index: i };
            }
            return;
        }

        // The hand: Space or Ctrl held, whatever the tool. Not while a text
        // is being typed, where Space is a space.
        if ctrl() || (space() && self.editing.is_none()) {
            self.drag = Drag::Pan { last: p };
            return;
        }

        if self.tool == Tool::Crop {
            if let Some(h) = self.crop_handle_at(p) {
                self.drag = Drag::CropHandle { handle: h };
            } else if self.crop.is_some_and(|c| {
                ip.0 >= c.left && ip.0 <= c.right && ip.1 >= c.top && ip.1 <= c.bottom
            }) {
                self.drag = Drag::CropMove { last: ip };
            } else {
                self.drag = Drag::CropNew { start: ip };
            }
            return;
        }

        if let Some(h) = self.handle_at(p) {
            self.checkpoint();
            self.drag = Drag::Handle {
                index: self.selected.unwrap(),
                handle: h,
            };
            return;
        }

        // A click on an existing annotation selects and moves it, whatever
        // the tool. A text is edited by a double click, or by a click with
        // the text tool — the caret lands where it was clicked.
        if self.editing.is_some() && self.shape_at(p) == self.editing {
            return;
        }
        // Ending an edit can remove an empty text and shift every later
        // index: look for the shape under the pointer only afterwards.
        self.commit_text();
        if let Some(i) = self.shape_at(p) {
            let text = self.shapes.get(i).is_some_and(|s| s.kind == Kind::Text);
            if text && double {
                self.begin_edit(i, Some(p));
                self.drag = Drag::TextSelect { index: i };
                self.layout();
                return;
            }
            // With the text tool, a text is moved by a drag and edited by
            // a click: editing on the press left no way to move it.
            if text && self.tool == Tool::Draw(Kind::Text) {
                self.edit_on_release = Some((i, p));
            }
            self.selected = Some(i);
            self.checkpoint();
            self.drag = Drag::Move { index: i, last: ip };
            self.layout();
            return;
        }

        self.commit_text();
        self.selected = None;
        let Tool::Draw(kind) = self.tool else {
            // With the arrow, a press on empty canvas grabs the picture
            // when it is larger than the window.
            if self.can_pan() {
                self.drag = Drag::Pan { last: p };
            }
            self.layout();
            return;
        };
        self.checkpoint();
        let size = if kind == Kind::Text {
            self.text_size
        } else {
            self.stroke
        };
        let mut shape = Shape::new(kind, ip, self.current_color(), size);
        shape.text_style = self.text_style;
        shape.obscure = self.obscure;
        match kind {
            Kind::Counter => {
                shape.number = self.next_counter();
                self.shapes.push(shape);
                self.selected = Some(self.shapes.len() - 1);
            }
            Kind::Text => {
                self.shapes.push(shape);
                self.editing = Some(self.shapes.len() - 1);
                self.caret = 0;
                self.anchor = 0;
            }
            _ => {
                self.raw_stroke = shape.points.clone();
                self.shapes.push(shape);
                self.drag = Drag::Create;
            }
        }
        self.layout();
    }

    fn slide(&mut self, slider: Slider, x: f32) {
        let Some(item) = self.items.iter().find(|i| i.act == Act::Slider(slider)) else {
            return;
        };
        let f = ((x - item.r.x) / item.r.w).clamp(0.0, 1.0);
        let mut bg = self.background.unwrap_or_default();
        match slider {
            Slider::Padding => bg.padding = (f * 240.0).round(),
            Slider::Radius => bg.radius = (f * 48.0).round(),
            Slider::Shadow => bg.shadow = (f * 100.0).round() / 100.0,
        }
        self.background = Some(bg);
    }

    fn on_move(&mut self, p: (f32, f32)) {
        let before = self.mouse;
        self.mouse = p;
        let ip = self.to_image(p);
        match self.drag {
            Drag::None => {
                let _ = before;
            }
            Drag::Create => {
                let curved = self.arrow_curved;
                if let Some(s) = self.shapes.last_mut() {
                    match s.kind {
                        // Shown straight while the hand keeps it nearly so,
                        // and as drawn the moment it does not. A pen line is
                        // thin, so it is judged as if it were three times
                        // as wide, and never under 12 px: the hand wavers by
                        // more than half of 4 px.
                        Kind::Pen | Kind::Highlighter => {
                            let width = if s.kind == Kind::Pen {
                                (s.size * 3.0).max(12.0)
                            } else {
                                s.stroke_width()
                            };
                            let raw = &mut self.raw_stroke;
                            let last = raw.last().copied().unwrap_or(ip);
                            if raw.is_empty()
                                || (last.0 - ip.0).abs() + (last.1 - ip.1).abs() >= 1.0
                            {
                                raw.push(ip);
                            }
                            s.points = match crate::annot::straight_stroke(raw, width) {
                                Some(ends) => ends.to_vec(),
                                None => raw.clone(),
                            };
                        }
                        k => {
                            s.b = if shift() { constrain(k, s.a, ip) } else { ip };
                            if k == Kind::Arrow && curved {
                                s.ctrl = Some(crate::annot::default_bend(s.a, s.b));
                            }
                        }
                    }
                }
            }
            Drag::Move { index, last } => {
                if let Some(s) = self.shapes.get_mut(index) {
                    s.translate(ip.0 - last.0, ip.1 - last.1);
                    self.drag = Drag::Move { index, last: ip };
                }
            }
            Drag::Handle { index, handle } => {
                if let Some(s) = self.shapes.get_mut(index) {
                    s.drag_handle(handle, ip);
                }
            }
            Drag::CropNew { start } => {
                let (w, h) = (self.base.width as f32, self.base.height as f32);
                let c = |v: f32, max: f32| v.clamp(0.0, max);
                let to = (c(ip.0, w), c(ip.1, h));
                self.crop = Some(match self.crop_aspect.ratio(w, h) {
                    Some(r) => aspect_corner(start, to, r, w, h),
                    None => {
                        let span_v = (start.1.min(to.1), start.1.max(to.1));
                        let span_h = (start.0.min(to.0), start.0.max(to.0));
                        let x = self.snap(true, to.0, span_v);
                        let y = self.snap(false, to.1, span_h);
                        D2D_RECT_F {
                            left: start.0.min(x),
                            top: start.1.min(y),
                            right: start.0.max(x),
                            bottom: start.1.max(y),
                        }
                    }
                });
            }
            Drag::CropMove { last } => {
                if let Some(c) = &mut self.crop {
                    let (w, h) = (self.base.width as f32, self.base.height as f32);
                    let dx = (ip.0 - last.0).clamp(-c.left, w - c.right);
                    let dy = (ip.1 - last.1).clamp(-c.top, h - c.bottom);
                    c.left += dx;
                    c.right += dx;
                    c.top += dy;
                    c.bottom += dy;
                    self.drag = Drag::CropMove { last: ip };
                }
            }
            Drag::CropHandle { handle } => {
                let (w, h) = (self.base.width as f32, self.base.height as f32);
                let ratio = self.crop_aspect.ratio(w, h);
                if let (Some(c), Some(r)) = (self.crop, ratio) {
                    // The opposite corner stays where it is.
                    let fixed = match handle {
                        0 => (c.right, c.bottom),
                        1 => (c.left, c.bottom),
                        2 => (c.right, c.top),
                        _ => (c.left, c.top),
                    };
                    let to = (ip.0.clamp(0.0, w), ip.1.clamp(0.0, h));
                    self.crop = Some(aspect_corner(fixed, to, r, w, h));
                } else if let Some(c0) = self.crop {
                    let x = self.snap(true, ip.0.clamp(0.0, w), (c0.top, c0.bottom));
                    let y = self.snap(false, ip.1.clamp(0.0, h), (c0.left, c0.right));
                    let c = self.crop.as_mut().expect("checked above");
                    match handle {
                        0 => (c.left, c.top) = (x.min(c.right - 1.0), y.min(c.bottom - 1.0)),
                        1 => (c.right, c.top) = (x.max(c.left + 1.0), y.min(c.bottom - 1.0)),
                        2 => (c.left, c.bottom) = (x.min(c.right - 1.0), y.max(c.top + 1.0)),
                        _ => (c.right, c.bottom) = (x.max(c.left + 1.0), y.max(c.top + 1.0)),
                    }
                }
            }
            Drag::Slider(slider) => self.slide(slider, p.0),
            Drag::Pan { last } => {
                self.pan = (self.pan.0 + p.0 - last.0, self.pan.1 + p.1 - last.1);
                self.clamp_pan();
                self.drag = Drag::Pan { last: p };
            }
            Drag::TextSelect { index } => {
                if index < self.shapes.len() {
                    self.caret = self.text_pos_at(index, p);
                }
            }
        }
    }

    fn on_up(&mut self, p: (f32, f32)) -> Option<Act> {
        self.mouse = p;
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        match drag {
            Drag::Create => {
                if let Some(s) = self.shapes.last() {
                    if s.is_degenerate() {
                        // Nothing was drawn, so no undo step (`settle`).
                        self.shapes.pop();
                    } else {
                        self.selected = Some(self.shapes.len() - 1);
                        if s.kind == Kind::Pixelate {
                            self.refresh_pixels();
                        }
                    }
                }
                self.layout();
                None
            }
            Drag::Move { index, .. } | Drag::Handle { index, .. } => {
                if self
                    .shapes
                    .get(index)
                    .is_some_and(|s| s.kind == Kind::Pixelate)
                {
                    self.refresh_pixels();
                }
                if let Some((i, at)) = self.edit_on_release.take() {
                    let (dx, dy) =
                        unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
                    let still = (p.0 - at.0).abs() <= dx as f32 && (p.1 - at.1).abs() <= dy as f32;
                    if still && i == index {
                        self.begin_edit(i, Some(p));
                        self.layout();
                    }
                }
                None
            }
            Drag::None => {
                // A button acts when released over the button it was
                // pressed on.
                let pressed = self.pressed.take()?;
                self.items
                    .iter()
                    .find(|i| i.r.contains(p))
                    .map(|i| i.act)
                    .filter(|a| *a == pressed)
                    // Clicking the active tool again puts it down: back to
                    // selecting, so a click no longer draws.
                    .map(|a| match a {
                        Act::Tool(t) if t == self.tool && t != Tool::Select => {
                            Act::Tool(Tool::Select)
                        }
                        other => other,
                    })
            }
            _ => None,
        }
    }

    fn cursor(&self, p: (f32, f32)) -> PCWSTR {
        if self.items.iter().any(|i| i.r.contains(p))
            || self.pop_items.iter().any(|i| i.r.contains(p))
            || !self.canvas().contains(p)
        {
            return IDC_ARROW;
        }
        if matches!(self.drag, Drag::Pan { .. })
            || ((space() && self.editing.is_none()) && self.can_pan())
        {
            return IDC_HAND;
        }
        if self.tool == Tool::Crop {
            return match self.crop_handle_at(p) {
                Some(0 | 3) => IDC_SIZENWSE,
                Some(_) => IDC_SIZENESW,
                None => IDC_CROSS,
            };
        }
        if self.handle_at(p).is_some() {
            return IDC_SIZEALL;
        }
        if let Some(i) = self.editing
            && self.text_grip(i, p)
        {
            return IDC_SIZEALL;
        }
        if let Some(i) = self.shape_at(p) {
            return if self.editing == Some(i) {
                IDC_IBEAM
            } else {
                IDC_SIZEALL
            };
        }
        match self.tool {
            Tool::Select if self.can_pan() => IDC_HAND,
            Tool::Select => IDC_ARROW,
            Tool::Draw(Kind::Text) => IDC_IBEAM,
            _ => IDC_CROSS,
        }
    }

    /// Keys; returns an action to run after the borrow ends.
    fn on_key(&mut self, key: u16) -> Option<Act> {
        if let Some(i) = self.editing {
            self.edit_key(i, key);
            return None;
        }
        let c = ctrl();
        let sh = shift();
        // While the resize menu is open, digits, Backspace and Enter belong
        // to its width field.
        if self.popover == Some(Popover::Resize) && !c {
            let digit = match key {
                0x30..=0x39 => Some(key - 0x30),
                0x60..=0x69 => Some(key - 0x60),
                _ => None,
            };
            if let Some(d) = digit {
                if self.resize_fresh {
                    self.resize_text.clear();
                    self.resize_fresh = false;
                }
                if self.resize_text.len() < 5 {
                    self.resize_text.push(char::from(b'0' + d as u8));
                }
                self.layout_popover();
                return None;
            }
            if key == VK_BACK.0 {
                if self.resize_fresh {
                    self.resize_text.clear();
                    self.resize_fresh = false;
                } else {
                    self.resize_text.pop();
                }
                self.layout_popover();
                return None;
            }
            if key == VK_RETURN.0 {
                return Some(Act::ResizeTyped);
            }
        }
        match key {
            k if k == VK_ESCAPE.0 => {
                if self.popover.is_some() {
                    self.popover = None;
                    self.pop_items.clear();
                } else if self.tool == Tool::Crop {
                    self.set_tool(Tool::Select);
                } else if self.selected.is_some() {
                    self.selected = None;
                    self.layout();
                } else {
                    return Some(Act::Close);
                }
            }
            k if k == VK_RETURN.0 => {
                if self.tool == Tool::Crop {
                    self.apply_crop();
                } else {
                    return Some(Act::Done);
                }
            }
            k if k == VK_DELETE.0 || k == VK_BACK.0 => {
                if let Some(i) = self.selected.take() {
                    self.checkpoint();
                    let removed = self.shapes.remove(i);
                    if removed.kind == Kind::Pixelate {
                        self.refresh_pixels();
                    }
                    self.layout();
                }
            }
            k if [VK_LEFT.0, VK_RIGHT.0, VK_UP.0, VK_DOWN.0].contains(&k) => {
                if let Some(i) = self.selected {
                    let step = if sh { 10.0 } else { 1.0 };
                    let (dx, dy) = match k {
                        k if k == VK_LEFT.0 => (-step, 0.0),
                        k if k == VK_RIGHT.0 => (step, 0.0),
                        k if k == VK_UP.0 => (0.0, -step),
                        _ => (0.0, step),
                    };
                    self.checkpoint();
                    self.shapes[i].translate(dx, dy);
                    if self.shapes[i].kind == Kind::Pixelate {
                        self.refresh_pixels();
                    }
                }
            }
            0x5A if c && sh => return Some(Act::Redo),
            0x5A if c => return Some(Act::Undo),
            0x59 if c => return Some(Act::Redo),
            0x43 if c => return Some(Act::Copy),
            0x53 if c && sh => return Some(Act::SaveAs),
            0x53 if c => return Some(Act::Save),
            0x30 if c => return Some(Act::PickZoom(None)),
            0x31 if c => return Some(Act::PickZoom(Some(1.0))),
            // The "+" and "−" keys of the main block and of the keypad.
            0xBB | 0x6B if c => return Some(Act::ZoomStep(true)),
            0xBD | 0x6D if c => return Some(Act::ZoomStep(false)),
            k if !c => {
                let tool = match k {
                    0x56 => Some(Tool::Select),
                    0x52 => Some(Tool::Draw(Kind::Rect)),
                    0x46 => Some(Tool::Draw(Kind::FilledRect)),
                    0x4F => Some(Tool::Draw(Kind::Ellipse)),
                    0x4C => Some(Tool::Draw(Kind::Line)),
                    0x41 => Some(Tool::Draw(Kind::Arrow)),
                    0x54 => Some(Tool::Draw(Kind::Text)),
                    0x42 => Some(Tool::Draw(Kind::Pixelate)),
                    0x53 => Some(Tool::Draw(Kind::Spotlight)),
                    0x4E => Some(Tool::Draw(Kind::Counter)),
                    0x50 => Some(Tool::Draw(Kind::Pen)),
                    0x48 => Some(Tool::Draw(Kind::Highlighter)),
                    0x43 => Some(Tool::Crop),
                    _ => None,
                };
                if let Some(t) = tool {
                    return Some(Act::Tool(t));
                }
            }
            _ => {}
        }
        None
    }

    fn on_char(&mut self, unit: u16) {
        let Some(i) = self.editing else { return };
        if unit < 0x20 && unit != 0x0D {
            return;
        }
        let ch = if (0xD800..0xDC00).contains(&unit) {
            self.high_surrogate = Some(unit);
            return;
        } else if (0xDC00..0xE000).contains(&unit) {
            let Some(high) = self.high_surrogate.take() else {
                return;
            };
            char::decode_utf16([high, unit]).next().and_then(|r| r.ok())
        } else if unit == 0x0D {
            Some('\n')
        } else {
            char::from_u32(unit as u32)
        };
        if let Some(ch) = ch {
            self.insert_text(i, ch.encode_utf8(&mut [0u8; 4]));
        }
    }

    /// Resizes the image to width `w`, keeping its proportion, and the
    /// marks on it with it. One undo step; a size the app cannot hold does
    /// nothing (the menu says so before).
    fn resize_to(&mut self, w: u32) {
        let (w0, h0) = (self.base.width, self.base.height);
        let Some((w, h)) = resized_size(w0, h0, w).filter(|&(w, _)| w != w0) else {
            return;
        };
        self.commit_text();
        let image = match self.base.resized(w, h) {
            Ok(image) => image,
            Err(e) => {
                log::error(&format!("editor resize: {e}"));
                return;
            }
        };
        self.checkpoint();
        let (kx, ky) = (w as f32 / w0 as f32, h as f32 / h0 as f32);
        for s in &mut self.shapes {
            s.scale(kx, ky);
        }
        self.base = Rc::new(image);
        self.to_original = self.to_original.resized(kx, ky);
        self.crop = None;
        self.refresh_pixels();
        self.popover = None;
        self.pop_items.clear();
        self.zoom = None;
        self.pan = (0.0, 0.0);
        self.layout();
    }

    /// «Volver al original»: every crop and resize undone in one step, the
    /// marks carried back to where they lie on the picture as it opened.
    /// Carries every mark through `f`, sizes times `k`. A text keeps
    /// reading left to right: its box's centre goes through `f`, not its
    /// corner, or a turned text would land beside where it was.
    fn map_shapes(&mut self, f: impl Fn(Pt) -> Pt, k: f32) {
        for i in 0..self.shapes.len() {
            if self.shapes[i].kind == Kind::Text {
                let b = self.shapes[i].bounds(self.text_measure(&self.shapes[i]));
                let s = &mut self.shapes[i];
                let centre = ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
                let reach = (centre.0 - s.a.0, centre.1 - s.a.1);
                let to = f(centre);
                s.a = (to.0 - reach.0 * k, to.1 - reach.1 * k);
                s.b = s.a;
                s.size *= k;
            } else {
                self.shapes[i].map(&f, k);
            }
        }
    }

    /// Turns or mirrors the whole picture, the marks with it: one undo
    /// step, and «Volver al original» undoes it with the rest.
    fn turn(&mut self, turn: Turn) {
        self.commit_text();
        self.checkpoint();
        let (w, h) = (self.base.width as f32, self.base.height as f32);
        let image = match turn {
            Turn::Clockwise => self.base.rotated(true),
            Turn::CounterClockwise => self.base.rotated(false),
            Turn::MirrorH => self.base.flipped(true),
            Turn::MirrorV => self.base.flipped(false),
        };
        self.map_shapes(turn.forward(w, h), 1.0);
        self.base = Rc::new(image);
        self.to_original = self.to_original.turned(turn, w, h);
        // A crop being set out is set out again on the new picture, in the
        // proportion chosen — it fell back to the whole picture, and
        // «Recortar» then cropped nothing.
        if self.crop.is_some() {
            self.crop = Some(D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: self.base.width as f32,
                bottom: self.base.height as f32,
            });
            self.fit_crop_aspect();
        }
        self.selected = None;
        self.refresh_pixels();
        self.zoom = None;
        self.pan = (0.0, 0.0);
        self.layout();
    }

    fn revert_to_original(&mut self) {
        if Rc::ptr_eq(&self.base, &self.original) {
            return;
        }
        self.commit_text();
        self.checkpoint();
        let t = self.to_original;
        self.map_shapes(|p| t.apply(p), t.scale());
        self.base = self.original.clone();
        self.to_original = ToOriginal::IDENTITY;
        self.crop = None;
        self.selected = None;
        self.refresh_pixels();
        self.popover = None;
        self.pop_items.clear();
        self.zoom = None;
        self.pan = (0.0, 0.0);
        self.layout();
    }

    /// Multiplies the zoom by `factor`, keeping the image point under `p`
    /// where it is.
    fn zoom_about(&mut self, factor: f32, p: (f32, f32)) {
        let before = self.to_image(p);
        self.zoom = Some((self.z() * factor).clamp(MIN_ZOOM, MAX_ZOOM));
        let after = self.to_view(before);
        self.pan = (self.pan.0 + p.0 - after.0, self.pan.1 + p.1 - after.1);
    }

    fn on_wheel(&mut self, delta: i16, p: (f32, f32)) {
        if ctrl() {
            self.zoom_about(if delta > 0 { 1.15 } else { 1.0 / 1.15 }, p);
        } else {
            let step = delta as f32 / 120.0 * 60.0 * self.s;
            if shift() {
                self.pan.0 += step;
            } else {
                self.pan.1 += step;
            }
        }
        self.layout();
    }

    fn hit_test(&self, p: (f32, f32)) -> u32 {
        let (w, h) = self.client();
        let edge = 6.0 * self.s;
        let zoomed = unsafe { IsZoomed(self.hwnd).as_bool() };
        if !zoomed {
            let (l, r, t, b) = (p.0 < edge, p.0 >= w - edge, p.1 < edge, p.1 >= h - edge);
            match (l, r, t, b) {
                (true, _, true, _) => return HTTOPLEFT,
                (_, true, true, _) => return HTTOPRIGHT,
                (true, _, _, true) => return HTBOTTOMLEFT,
                (_, true, _, true) => return HTBOTTOMRIGHT,
                (true, ..) => return HTLEFT,
                (_, true, ..) => return HTRIGHT,
                (_, _, true, _) => return HTTOP,
                (_, _, _, true) => return HTBOTTOM,
                _ => {}
            }
        }
        // The toolbar's empty space moves the window, like a title bar.
        if p.1 < self.top_h()
            && !self.items.iter().any(|i| i.r.contains(p))
            && self.popover.is_none()
        {
            return HTCAPTION;
        }
        HTCLIENT
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                LRESULT(0)
            }
            WM_NCCALCSIZE if wparam.0 != 0 => {
                // No caption: the client area is the whole window. When
                // maximised, Windows puts the frame off screen; keep the
                // content inside the monitor.
                if IsZoomed(hwnd).as_bool() {
                    let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                    let frame = 8 * GetDpiForWindow(hwnd) as i32 / 96;
                    params.rgrc[0].left += frame;
                    params.rgrc[0].top += frame;
                    params.rgrc[0].right -= frame;
                    params.rgrc[0].bottom -= frame;
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let (x, y) = lparam_point(lparam.0);
                let mut pt = POINT { x, y };
                let _ = ScreenToClient(hwnd, &mut pt);
                match with(hwnd, |ed| ed.hit_test((pt.x as f32, pt.y as f32))) {
                    Some(h) => LRESULT(h as isize),
                    None => DefWindowProcW(hwnd, msg, wparam, lparam),
                }
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                let s = GetDpiForWindow(hwnd).max(96) as i32;
                info.ptMinTrackSize = POINT {
                    x: MIN_W as i32 * s / 96,
                    y: 420 * s / 96,
                };
                LRESULT(0)
            }
            WM_SIZE => {
                with(hwnd, |ed| {
                    let (w, h) = ed.client();
                    if let Some(g) = &ed.gpu {
                        let _ = g.target.Resize(
                            &windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U {
                                width: w as u32,
                                height: h as u32,
                            },
                        );
                    }
                    ed.layout();
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let r = &*(lparam.0 as *const RECT);
                let shown = with(hwnd, |ed| {
                    ed.s = (wparam.0 & 0xFFFF) as f32 / 96.0;
                    ed.gpu = None;
                    ed.layout();
                    ed.shown
                })
                .unwrap_or(false);
                // Before it is shown, `place` decides the size and position.
                if !shown {
                    return LRESULT(0);
                }
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                with(hwnd, |ed| ed.paint());
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(hwnd, &mut pt);
                let id =
                    with(hwnd, |ed| ed.cursor((pt.x as f32, pt.y as f32))).unwrap_or(IDC_ARROW);
                SetCursor(LoadCursorW(None, id).ok());
                LRESULT(1)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = lparam_point(lparam.0);
                SetCapture(hwnd);
                with(hwnd, |ed| {
                    ed.on_down((x as f32, y as f32), msg == WM_LBUTTONDBLCLK)
                });
                if with(hwnd, |ed| ed.pressed == Some(Act::Drag)) == Some(true) {
                    let file = prepare_drag(hwnd);
                    with(hwnd, |ed| ed.drag_ready = file);
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            // The middle button drags the picture with any tool.
            WM_MBUTTONDOWN => {
                let (x, y) = lparam_point(lparam.0);
                SetCapture(hwnd);
                let grabbed = with(hwnd, |ed| {
                    let grab = ed.drag == Drag::None && ed.can_pan();
                    if grab {
                        ed.drag = Drag::Pan {
                            last: (x as f32, y as f32),
                        };
                    }
                    grab
                });
                // A captured pointer gets no WM_SETCURSOR: show the hand now.
                if grabbed == Some(true) {
                    SetCursor(LoadCursorW(None, IDC_HAND).ok());
                }
                LRESULT(0)
            }
            WM_MBUTTONUP => {
                let _ = ReleaseCapture();
                with(hwnd, |ed| {
                    if matches!(ed.drag, Drag::Pan { .. }) {
                        ed.drag = Drag::None;
                    }
                });
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_point(lparam.0);
                let p = (x as f32, y as f32);
                with(hwnd, |ed| ed.on_move(p));
                let file = with(hwnd, |ed| {
                    if ed.drag_started(p) {
                        ed.drag_ready.take()
                    } else {
                        None
                    }
                })
                .flatten();
                if let Some(file) = file {
                    // The drag runs its own loop and takes the pointer.
                    let _ = ReleaseCapture();
                    let dropped = dragdrop::drag_file(hwnd, &file);
                    log::info(&format!("drag out: dropped {dropped}"));
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = lparam_point(lparam.0);
                let _ = ReleaseCapture();
                // A press on «Arrastrar» released without a drag: its file goes.
                if let Some(Some(unused)) = with(hwnd, |ed| ed.drag_ready.take()) {
                    let _ = std::fs::remove_file(unused);
                }
                if let Some(Some(action)) = with(hwnd, |ed| ed.on_up((x as f32, y as f32))) {
                    act(hwnd, action);
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
                let (x, y) = lparam_point(lparam.0);
                let mut pt = POINT { x, y };
                let _ = ScreenToClient(hwnd, &mut pt);
                with(hwnd, |ed| ed.on_wheel(delta, (pt.x as f32, pt.y as f32)));
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if let Some(Some(action)) = with(hwnd, |ed| ed.on_key(wparam.0 as u16)) {
                    act(hwnd, action);
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_CHAR => {
                with(hwnd, |ed| ed.on_char(wparam.0 as u16));
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == FLASH_TIMER => {
                let _ = KillTimer(Some(hwnd), FLASH_TIMER);
                with(hwnd, |ed| ed.flash = None);
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_CLOSE => {
                act(hwnd, Act::Close);
                LRESULT(0)
            }
            WM_EDITOR_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<Option<Editor>>;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if !ptr.is_null() {
                    let changed = (*ptr).try_borrow().ok().and_then(|ed| {
                        let ed = ed.as_ref()?;
                        (ed.background != ed.opened_with).then_some(ed.background)
                    });
                    drop(Box::from_raw(ptr));
                    if let Some(background) = changed {
                        remember_background(background);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_that_fits_stays_centred_and_a_tall_one_stops_at_its_edges() {
        // Fits: no movement at all.
        assert_eq!(pan_limit(800.0, 500.0, 24.0), 0.0);
        // 2000 long on 800 with 24 each side: 1248 to share, 624 each way.
        assert_eq!(pan_limit(800.0, 2000.0, 24.0), 624.0);
    }

    #[test]
    fn a_proportion_fits_inside_the_crop_and_a_drag_keeps_it() {
        let full = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: 1000.0,
            bottom: 800.0,
        };
        let r = aspect_fit(full, 16.0 / 9.0, 1000.0, 800.0);
        assert_eq!((r.left, r.right), (0.0, 1000.0));
        assert!(((r.bottom - r.top) - 562.5).abs() < 0.01);
        assert!((r.top - 118.75).abs() < 0.01, "centred: {r:?}");
        let sq = aspect_fit(full, 1.0, 1000.0, 800.0);
        assert_eq!((sq.right - sq.left, sq.bottom - sq.top), (800.0, 800.0));
        // Dragged from (100, 100) towards (900, 300) at 1:1: the wider move
        // leads and the square is cut to the picture's bottom.
        let c = aspect_corner((100.0, 100.0), (900.0, 300.0), 1.0, 1000.0, 800.0);
        assert_eq!((c.left, c.top), (100.0, 100.0));
        assert!(
            (c.right - c.left - (c.bottom - c.top)).abs() < 0.01,
            "square: {c:?}"
        );
        assert!(c.bottom <= 800.0 && c.right <= 1000.0);
        // Up and to the left of the fixed corner.
        let up = aspect_corner((500.0, 500.0), (200.0, 450.0), 4.0 / 3.0, 1000.0, 800.0);
        assert_eq!((up.right, up.bottom), (500.0, 500.0));
        assert!(((up.right - up.left) / (up.bottom - up.top) - 4.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn every_turn_is_undone_by_its_way_back() {
        let (w, h) = (400.0, 300.0);
        for turn in [
            Turn::Clockwise,
            Turn::CounterClockwise,
            Turn::MirrorH,
            Turn::MirrorV,
        ] {
            for p in [(0.0, 0.0), (300.0, 200.0), (400.0, 300.0), (12.5, 287.0)] {
                let there = turn.forward(w, h)(p);
                let back = turn.back(w, h).apply(there);
                assert!(
                    (back.0 - p.0).abs() < 1e-3 && (back.1 - p.1).abs() < 1e-3,
                    "{turn:?} {p:?}"
                );
            }
        }
        // Turned clockwise, then mirrored, then cropped: still found.
        let drawn = (300.0, 200.0);
        let turned = Turn::Clockwise.forward(w, h)(drawn); // on 300 × 400
        let mirrored = Turn::MirrorH.forward(h, w)(turned);
        let now = (mirrored.0 - 50.0, mirrored.1 - 60.0);
        let t = ToOriginal::IDENTITY
            .turned(Turn::Clockwise, w, h)
            .turned(Turn::MirrorH, h, w)
            .cropped(50.0, 60.0);
        let back = t.apply(now);
        assert!(
            (back.0 - drawn.0).abs() < 1e-3 && (back.1 - drawn.1).abs() < 1e-3,
            "{back:?}"
        );
        assert!((t.scale() - 1.0).abs() < 1e-6, "a turn changes no length");
    }

    #[test]
    fn back_to_original_undoes_crops_and_resizes_in_any_order() {
        // A point drawn at (300, 200) on the picture as it opened.
        let drawn = (300.0, 200.0);
        // Crop at (100, 50), halve, crop at (20, 10), double.
        let crop1 = (drawn.0 - 100.0, drawn.1 - 50.0);
        let half = (crop1.0 * 0.5, crop1.1 * 0.5);
        let crop2 = (half.0 - 20.0, half.1 - 10.0);
        let now = (crop2.0 * 2.0, crop2.1 * 2.0);
        let t = ToOriginal::IDENTITY
            .cropped(100.0, 50.0)
            .resized(0.5, 0.5)
            .cropped(20.0, 10.0)
            .resized(2.0, 2.0);
        let back = t.apply(now);
        assert!(
            (back.0 - drawn.0).abs() < 1e-3 && (back.1 - drawn.1).abs() < 1e-3,
            "{back:?}"
        );
    }

    #[test]
    fn a_resize_keeps_the_proportion_and_the_bounds() {
        assert_eq!(resized_size(1894, 13297, 947), Some((947, 6649)));
        assert_eq!(resized_size(1000, 3, 10), Some((10, 1)));
        assert_eq!(resized_size(1894, 13297, 0), None);
        // Twice a tall scrolling capture would pass 20 000 pixels a side.
        assert_eq!(resized_size(1894, 13297, 3788), None);
    }

    #[test]
    fn the_frame_never_exceeds_a_small_or_portrait_work_area() {
        // Portrait 1080-px monitor at 125 %, and 1920 px at 200 %: the
        // minimum width is more than the room there is.
        for (work, s) in [
            ((1080.0, 1872.0), 1.25),
            ((1920.0, 1032.0), 2.0),
            ((800.0, 560.0), 1.0),
        ] {
            let (w, h) = frame_size((4000.0, 3000.0), work, s);
            assert!(w <= work.0 && h <= work.1, "{work:?} at {s}: {w}x{h}");
            let (w, h) = frame_size((10.0, 10.0), work, s);
            assert!(
                w <= work.0 && h <= work.1,
                "small image, {work:?} at {s}: {w}x{h}"
            );
        }
        // A roomy screen: the minimum width for the toolbar, 100 % otherwise.
        assert_eq!(frame_size((200.0, 100.0), (2560.0, 1400.0), 1.0).0, 1080.0);
        assert_eq!(
            frame_size((1600.0, 700.0), (2560.0, 1400.0), 1.0),
            (1680.0, 880.0)
        );
    }

    #[test]
    fn shift_snaps_a_line_to_the_nearest_45_degrees() {
        let (x, y) = constrain(Kind::Arrow, (0.0, 0.0), (100.0, 10.0));
        assert!((y).abs() < 0.01 && (x - 100.5).abs() < 1.0, "{x},{y}");
        let (x, y) = constrain(Kind::Line, (0.0, 0.0), (100.0, 90.0));
        assert!(
            (x - y).abs() < 0.01,
            "a near-diagonal becomes the diagonal: {x},{y}"
        );
    }

    #[test]
    fn shift_makes_a_box_square_towards_the_pointer() {
        assert_eq!(
            constrain(Kind::Rect, (10.0, 10.0), (40.0, 20.0)),
            (40.0, 40.0)
        );
        assert_eq!(
            constrain(Kind::Ellipse, (10.0, 10.0), (-20.0, 0.0)),
            (-20.0, -20.0)
        );
    }
}
