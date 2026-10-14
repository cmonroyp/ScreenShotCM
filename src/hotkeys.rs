//! Global hotkeys through `RegisterHotKey`: Windows delivers `WM_HOTKEY` to the
//! agent's window, so a hotkey costs nothing while it is not pressed — no
//! keyboard hook, no polling.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};

use crate::commands::Command;
use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: u32,
    pub vk: u32,
}

const VK_SNAPSHOT: u32 = 0x2C;

/// Parses `Ctrl+Shift+PrintScreen`. Case and spaces do not matter; an empty
/// string is "no hotkey".
pub fn parse(text: &str) -> Result<Option<Chord>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let mut modifiers = 0u32;
    let mut vk = None;
    for part in text.split('+').map(str::trim) {
        let lower = part.to_ascii_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL.0,
            "alt" => modifiers |= MOD_ALT.0,
            "shift" => modifiers |= MOD_SHIFT.0,
            "win" | "windows" => modifiers |= MOD_WIN.0,
            _ => {
                if vk.is_some() {
                    return Err(crate::i18n::trs(
                        format!("«{text}» tiene más de una tecla"),
                        format!("«{text}» has more than one key"),
                    ));
                }
                vk = Some(key_code(&lower).ok_or_else(|| {
                    crate::i18n::trs(
                        format!("tecla desconocida «{part}» en «{text}»"),
                        format!("unknown key «{part}» in «{text}»"),
                    )
                })?);
            }
        }
    }
    match vk {
        Some(vk) => Ok(Some(Chord { modifiers, vk })),
        None => Err(crate::i18n::trs(
            format!("«{text}» no tiene tecla, solo modificadores"),
            format!("«{text}» has no key, only modifiers"),
        )),
    }
}

fn key_code(key: &str) -> Option<u32> {
    match key {
        "printscreen" | "prtsc" | "prtscn" | "print" => return Some(VK_SNAPSHOT),
        _ => {}
    }
    let bytes = key.as_bytes();
    if bytes.len() == 1 && (bytes[0].is_ascii_alphanumeric()) {
        return Some(bytes[0].to_ascii_uppercase() as u32);
    }
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok())
        && (1..=24).contains(&n)
    {
        return Some(0x70 + n - 1);
    }
    None
}

/// The commands a hotkey can trigger, in the order of their ids.
const BOUND: [Command; 5] = [
    Command::CaptureArea,
    Command::CaptureFullscreen,
    Command::CaptureWindow,
    Command::CapturePreviousArea,
    Command::CaptureScrolling,
];

fn text_for(settings: &Settings, command: Command) -> &str {
    match command {
        Command::CaptureArea => &settings.hotkey_area,
        Command::CaptureFullscreen => &settings.hotkey_fullscreen,
        Command::CaptureWindow => &settings.hotkey_window,
        Command::CapturePreviousArea => &settings.hotkey_previous_area,
        Command::CaptureScrolling => &settings.hotkey_scrolling,
        _ => "",
    }
}

pub fn command_for(id: i32) -> Option<Command> {
    BOUND.get(usize::try_from(id - 1).ok()?).copied()
}

/// Registers every configured hotkey and returns, for the operator, each one
/// that could not be registered and why — most often another program holds it.
pub fn register_all(hwnd: HWND, settings: &Settings) -> Vec<String> {
    unregister_all(hwnd);
    let mut problems = Vec::new();
    for (i, command) in BOUND.iter().enumerate() {
        let text = text_for(settings, *command);
        match parse(text) {
            Ok(None) => {}
            Ok(Some(chord)) => unsafe {
                let mods = HOT_KEY_MODIFIERS(chord.modifiers | MOD_NOREPEAT.0);
                if RegisterHotKey(Some(hwnd), i as i32 + 1, mods, chord.vk).is_err() {
                    problems.push(crate::i18n::trs(
                        format!("{text}: otro programa ya usa este atajo"),
                        format!("{text}: another program already uses this shortcut"),
                    ));
                }
            },
            Err(e) => problems.push(e),
        }
    }
    problems
}

pub fn unregister_all(hwnd: HWND) {
    for i in 0..BOUND.len() {
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), i as i32 + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_in_any_case_and_order() {
        let c = parse("shift + CTRL+PrintScreen").unwrap().unwrap();
        assert_eq!(c.modifiers, MOD_CONTROL.0 | MOD_SHIFT.0);
        assert_eq!(c.vk, VK_SNAPSHOT);
    }

    #[test]
    fn parses_letters_digits_and_function_keys() {
        assert_eq!(parse("Alt+a").unwrap().unwrap().vk, b'A' as u32);
        assert_eq!(parse("Ctrl+4").unwrap().unwrap().vk, b'4' as u32);
        assert_eq!(parse("F12").unwrap().unwrap().vk, 0x7B);
    }

    #[test]
    fn rejects_what_cannot_be_a_hotkey() {
        assert!(parse("Ctrl+Shift").is_err());
        assert!(parse("Ctrl+A+B").is_err());
        assert!(parse("Ctrl+F25").is_err());
        assert_eq!(parse("  ").unwrap(), None);
    }

    #[test]
    fn ids_map_back_to_their_commands() {
        assert_eq!(command_for(1), Some(Command::CaptureArea));
        assert_eq!(command_for(0), None);
        assert_eq!(command_for(99), None);
    }
}
