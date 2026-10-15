//! The palette: a light chrome with grey pills, the active tool a
//! filled blue button, the primary action blue. Dark when Windows' apps are
//! dark. Every window that draws takes its colours from here.

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::BOOL;
use windows::core::w;

use crate::gfx::rgba;

#[derive(Clone, Copy)]
pub struct Palette {
    /// The toolbar and bottom bar.
    pub chrome: D2D1_COLOR_F,
    pub chrome_line: D2D1_COLOR_F,
    /// Behind the image.
    pub canvas: D2D1_COLOR_F,
    /// The rounded group behind a set of tools.
    pub pill: D2D1_COLOR_F,
    pub pill_hover: D2D1_COLOR_F,
    pub icon: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    pub text_dim: D2D1_COLOR_F,
    pub accent: D2D1_COLOR_F,
    pub on_accent: D2D1_COLOR_F,
    /// A secondary button ("Guardar como…").
    pub button: D2D1_COLOR_F,
    pub popover: D2D1_COLOR_F,
    /// A field one types into, and a rule between sections: for the
    /// settings window, which is drawn with GDI and so takes them opaque.
    pub field: D2D1_COLOR_F,
    pub hairline: D2D1_COLOR_F,
    pub dark: bool,
}

/// A palette colour as GDI takes it: opaque, blue in the high byte.
pub fn gdi(c: D2D1_COLOR_F) -> COLORREF {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(b(c.r) | (b(c.g) << 8) | (b(c.b) << 16))
}

/// Paints a framed window's title bar in the palette's shade (Windows 10
/// 20H1 and later; older ones leave it light). The settings and history
/// windows keep the system frame and were light-framed in the dark palette.
pub fn frame_window(hwnd: HWND) {
    let dark = BOOL(current().dark as i32);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        );
    }
}

pub const ACCENT: (u8, u8, u8) = (10, 122, 255);

/// The palette the settings ask for: 0 Windows' own, 1 light, 2 dark.
static CHOICE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Takes the settings' theme; every window opened from now on follows it.
pub fn apply(choice: crate::settings::ThemeChoice) {
    use crate::settings::ThemeChoice;
    let v = match choice {
        ThemeChoice::Auto => 0,
        ThemeChoice::Light => 1,
        ThemeChoice::Dark => 2,
    };
    CHOICE.store(v, std::sync::atomic::Ordering::Relaxed);
}

pub fn current() -> Palette {
    // A debug build takes `SCM_THEME=light|dark`, so both palettes can be
    // photographed without touching the user's Windows setting.
    if cfg!(debug_assertions)
        && let Some(forced) = std::env::var_os("SCM_THEME")
    {
        return if forced == "dark" { dark() } else { light() };
    }
    match CHOICE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => light(),
        2 => dark(),
        _ if apps_use_dark() => dark(),
        _ => light(),
    }
}

pub fn light() -> Palette {
    Palette {
        chrome: rgba(250, 250, 252, 1.0),
        chrome_line: rgba(0, 0, 0, 0.08),
        canvas: rgba(236, 236, 240, 1.0),
        pill: rgba(0, 0, 0, 0.055),
        pill_hover: rgba(0, 0, 0, 0.09),
        icon: rgba(28, 28, 30, 1.0),
        text: rgba(28, 28, 30, 1.0),
        text_dim: rgba(110, 110, 118, 1.0),
        accent: rgba(ACCENT.0, ACCENT.1, ACCENT.2, 1.0),
        on_accent: rgba(255, 255, 255, 1.0),
        button: rgba(0, 0, 0, 0.06),
        popover: rgba(255, 255, 255, 1.0),
        field: rgba(255, 255, 255, 1.0),
        hairline: rgba(229, 229, 229, 1.0),
        dark: false,
    }
}

pub fn dark() -> Palette {
    Palette {
        chrome: rgba(40, 40, 44, 1.0),
        chrome_line: rgba(255, 255, 255, 0.08),
        canvas: rgba(28, 28, 31, 1.0),
        pill: rgba(255, 255, 255, 0.07),
        pill_hover: rgba(255, 255, 255, 0.12),
        icon: rgba(236, 236, 240, 1.0),
        text: rgba(236, 236, 240, 1.0),
        text_dim: rgba(160, 160, 168, 1.0),
        accent: rgba(ACCENT.0, ACCENT.1, ACCENT.2, 1.0),
        on_accent: rgba(255, 255, 255, 1.0),
        button: rgba(255, 255, 255, 0.10),
        popover: rgba(52, 52, 57, 1.0),
        field: rgba(30, 30, 34, 1.0),
        hairline: rgba(64, 64, 70, 1.0),
        dark: true,
    }
}

/// Windows' own "app mode" setting; light when it cannot be read.
fn apps_use_dark() -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as *mut _),
            Some(&mut size),
        )
    };
    ok.is_ok() && value == 0
}

/// The annotation colours, red first.
pub const MARK_COLORS: [(u8, u8, u8); 10] = [
    (255, 45, 85),
    (255, 149, 0),
    (255, 204, 0),
    (52, 199, 89),
    (0, 199, 190),
    (10, 122, 255),
    (88, 86, 214),
    (175, 82, 222),
    (0, 0, 0),
    (255, 255, 255),
];
