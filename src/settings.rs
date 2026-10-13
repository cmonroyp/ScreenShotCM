//! The user's settings: a plain `key = value` file in the roaming profile.
//! A missing or unreadable value falls back to its default rather than
//! stopping the agent; an unknown key is kept out of the way, not an error.

use std::fs;
use std::path::PathBuf;

use crate::paths;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub save_dir: PathBuf,
    pub copy_after_capture: bool,
    pub save_after_capture: bool,
    pub show_overlay: bool,
    /// Straight to the editor instead of the quick-access card.
    pub open_editor_after_capture: bool,
    /// 0 keeps the overlay until it is dismissed.
    pub overlay_close_seconds: u32,
    pub image_format: ImageFormat,
    pub jpeg_quality: u8,
    pub show_magnifier: bool,
    pub show_crosshair: bool,
    pub history_days: u32,
    pub self_timer_seconds: u32,
    pub hotkey_area: String,
    pub hotkey_fullscreen: String,
    pub hotkey_window: String,
    pub hotkey_previous_area: String,
    pub language: Language,
}

/// The interface language: Windows' own, or one chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Auto,
    Spanish,
    English,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Language::Auto => "auto",
            Language::Spanish => "es",
            Language::English => "en",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            save_dir: paths::default_save_dir(),
            copy_after_capture: true,
            save_after_capture: false,
            show_overlay: true,
            // A capture opens straight in the editor.
            open_editor_after_capture: true,
            overlay_close_seconds: 0,
            image_format: ImageFormat::Png,
            jpeg_quality: 90,
            show_magnifier: true,
            show_crosshair: true,
            history_days: 30,
            self_timer_seconds: 5,
            hotkey_area: "PrintScreen".into(),
            hotkey_fullscreen: "Ctrl+PrintScreen".into(),
            hotkey_window: "Shift+PrintScreen".into(),
            hotkey_previous_area: "Alt+Shift+PrintScreen".into(),
            language: Language::Auto,
        }
    }
}

pub fn file() -> PathBuf {
    paths::config_dir().join("settings.ini")
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl Settings {
    pub fn load() -> Self {
        match fs::read_to_string(file()) {
            Ok(text) => Self::parse(&text),
            Err(_) => Self::default(),
        }
    }

    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "save_dir" if !v.is_empty() => s.save_dir = PathBuf::from(v),
                "copy_after_capture" => set(&mut s.copy_after_capture, parse_bool(v)),
                "save_after_capture" => set(&mut s.save_after_capture, parse_bool(v)),
                "show_overlay" => set(&mut s.show_overlay, parse_bool(v)),
                "open_editor_after_capture" => set(&mut s.open_editor_after_capture, parse_bool(v)),
                "overlay_close_seconds" => set(
                    &mut s.overlay_close_seconds,
                    v.parse().ok().filter(|n| *n <= 3600),
                ),
                "image_format" => set(
                    &mut s.image_format,
                    match v.to_ascii_lowercase().as_str() {
                        "png" => Some(ImageFormat::Png),
                        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
                        _ => None,
                    },
                ),
                "jpeg_quality" => set(
                    &mut s.jpeg_quality,
                    v.parse().ok().filter(|q| (1..=100).contains(q)),
                ),
                "show_magnifier" => set(&mut s.show_magnifier, parse_bool(v)),
                "show_crosshair" => set(&mut s.show_crosshair, parse_bool(v)),
                "history_days" => set(&mut s.history_days, v.parse().ok().filter(|d| *d <= 3650)),
                "self_timer_seconds" => set(
                    &mut s.self_timer_seconds,
                    v.parse().ok().filter(|d| (1..=60).contains(d)),
                ),
                "hotkey_area" => s.hotkey_area = v.into(),
                "hotkey_fullscreen" => s.hotkey_fullscreen = v.into(),
                "hotkey_window" => s.hotkey_window = v.into(),
                "hotkey_previous_area" => s.hotkey_previous_area = v.into(),
                "language" => set(
                    &mut s.language,
                    match v.to_ascii_lowercase().as_str() {
                        "auto" => Some(Language::Auto),
                        "es" => Some(Language::Spanish),
                        "en" => Some(Language::English),
                        _ => None,
                    },
                ),
                _ => {}
            }
        }
        s
    }

    pub fn serialise(&self) -> String {
        let b = |v: bool| if v { "true" } else { "false" };
        format!(
            "# ScreenShotCM settings. Edit while the agent is closed, or use \"Reload settings\".\n\
             # A hotkey is modifiers and a key joined by '+': Ctrl, Alt, Shift, Win;\n\
             # A-Z, 0-9, F1-F24, PrintScreen. Leave it empty to turn the hotkey off.\n\
             save_dir = {}\n\
             copy_after_capture = {}\n\
             save_after_capture = {}\n\
             show_overlay = {}\n\
             open_editor_after_capture = {}\n\
             overlay_close_seconds = {}\n\
             image_format = {}\n\
             jpeg_quality = {}\n\
             show_magnifier = {}\n\
             show_crosshair = {}\n\
             history_days = {}\n\
             self_timer_seconds = {}\n\
             hotkey_area = {}\n\
             hotkey_fullscreen = {}\n\
             hotkey_window = {}\n\
             hotkey_previous_area = {}\n\
             language = {}\n",
            self.save_dir.display(),
            b(self.copy_after_capture),
            b(self.save_after_capture),
            b(self.show_overlay),
            b(self.open_editor_after_capture),
            self.overlay_close_seconds,
            self.image_format.extension(),
            self.jpeg_quality,
            b(self.show_magnifier),
            b(self.show_crosshair),
            self.history_days,
            self.self_timer_seconds,
            self.hotkey_area,
            self.hotkey_fullscreen,
            self.hotkey_window,
            self.hotkey_previous_area,
            self.language.code(),
        )
    }

    /// Written whole through a temporary file, so a crash mid-write never
    /// leaves half a settings file.
    pub fn save(&self) -> std::io::Result<()> {
        let path = file();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("ini.tmp");
        fs::write(&tmp, self.serialise())?;
        fs::rename(&tmp, &path)
    }
}

fn set<T>(slot: &mut T, v: Option<T>) {
    if let Some(v) = v {
        *slot = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_every_value() {
        let mut s = Settings::default();
        s.copy_after_capture = false;
        s.image_format = ImageFormat::Jpeg;
        s.jpeg_quality = 70;
        s.hotkey_area = "Ctrl+Alt+A".into();
        s.save_dir = PathBuf::from(r"D:\Capturas con espacio");
        s.language = Language::English;
        assert_eq!(Settings::parse(&s.serialise()), s);
    }

    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let s = Settings::parse(
            "jpeg_quality = 400\nhistory_days = x\ncopy_after_capture = maybe\nunknown = 1",
        );
        let d = Settings::default();
        assert_eq!(s.jpeg_quality, d.jpeg_quality);
        assert_eq!(s.history_days, d.history_days);
        assert_eq!(s.copy_after_capture, d.copy_after_capture);
    }

    #[test]
    fn empty_hotkey_is_kept_empty() {
        assert_eq!(Settings::parse("hotkey_window =").hotkey_window, "");
    }
}
