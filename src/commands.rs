//! What the agent can be asked to do, whether by the tray menu, a hotkey, the
//! command line (`screenshotcm.exe --capture-area`) or a URL
//! (`screenshotcm://capture-area`). A second launch forwards its command to
//! the running agent, so a shortcut or a script drives the same instance.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    CaptureArea,
    CaptureFullscreen,
    CaptureWindow,
    CapturePreviousArea,
    SelfTimer,
    OpenFromClipboard,
    OpenFile,
    OpenHistory,
    RestoreRecentlyClosed,
    UnlockPins,
    ClosePins,
    OpenSaveFolder,
    OpenSettings,
    ReloadSettings,
    ToggleAutostart,
    Quit,
}

/// A command plus the file it names, for the verbs that open a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Run(Command),
    Annotate(PathBuf),
    Pin(PathBuf),
    /// Started with no verb: nothing to do but be running.
    None,
}

const VERBS: &[(&str, Command)] = &[
    ("capture-area", Command::CaptureArea),
    ("capture-fullscreen", Command::CaptureFullscreen),
    ("capture-window", Command::CaptureWindow),
    ("capture-previous-area", Command::CapturePreviousArea),
    ("self-timer", Command::SelfTimer),
    ("open-from-clipboard", Command::OpenFromClipboard),
    ("open-file", Command::OpenFile),
    ("open-history", Command::OpenHistory),
    ("restore-recently-closed", Command::RestoreRecentlyClosed),
    ("unlock-pins", Command::UnlockPins),
    ("close-pins", Command::ClosePins),
    ("open-save-folder", Command::OpenSaveFolder),
    ("open-settings", Command::OpenSettings),
    ("reload-settings", Command::ReloadSettings),
    ("quit", Command::Quit),
];

/// Reads the arguments after the program name.
pub fn parse_args(args: &[String]) -> Request {
    let Some(first) = args.first() else {
        return Request::None;
    };
    let rest = args.get(1).map(PathBuf::from);

    // screenshotcm://capture-area or screenshotcm://pin?filepath=C:\x.png
    if let Some(url) = first.strip_prefix("screenshotcm://") {
        let (verb, query) = url.split_once('?').unwrap_or((url, ""));
        let verb = verb.trim_end_matches('/');
        let file = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("filepath="))
            .map(|v| PathBuf::from(percent_decode(v)));
        return request_for(verb, file);
    }
    match first.strip_prefix("--") {
        Some(verb) => request_for(verb, rest),
        // A bare file: Explorer's "Open with" hands us the path alone.
        None => Request::Annotate(PathBuf::from(first)),
    }
}

fn request_for(verb: &str, file: Option<PathBuf>) -> Request {
    match (verb, file) {
        ("open-annotate", Some(f)) => Request::Annotate(f),
        ("pin", Some(f)) => Request::Pin(f),
        (v, _) => VERBS
            .iter()
            .find(|(name, _)| *name == v)
            .map(|(_, c)| Request::Run(*c))
            .unwrap_or(Request::None),
    }
}

/// The inverse of `parse_args`, for forwarding to the running agent.
pub fn encode(request: &Request) -> String {
    match request {
        Request::Run(c) => VERBS
            .iter()
            .find(|(_, cmd)| cmd == c)
            .map(|(v, _)| format!("--{v}"))
            .unwrap_or_default(),
        Request::Annotate(p) => format!("--open-annotate\n{}", p.display()),
        Request::Pin(p) => format!("--pin\n{}", p.display()),
        Request::None => String::new(),
    }
}

pub fn decode(text: &str) -> Request {
    let args: Vec<String> = text.split('\n').map(str::to_owned).collect();
    if text.is_empty() {
        Request::None
    } else {
        parse_args(&args)
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Hex digits are read as bytes: slicing the str could split a
        // multi-byte character and panic.
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn command_line_verbs() {
        assert_eq!(
            parse_args(&args(&["--capture-area"])),
            Request::Run(Command::CaptureArea)
        );
        assert_eq!(
            parse_args(&args(&["--pin", r"C:\a.png"])),
            Request::Pin(r"C:\a.png".into())
        );
        assert_eq!(parse_args(&args(&["--nonsense"])), Request::None);
        assert_eq!(parse_args(&[]), Request::None);
    }

    #[test]
    fn url_verbs_with_an_encoded_path() {
        assert_eq!(
            parse_args(&args(&[
                "screenshotcm://pin?filepath=C%3A%5CMis%20capturas%5Ca.png"
            ])),
            Request::Pin(r"C:\Mis capturas\a.png".into())
        );
        assert_eq!(
            parse_args(&args(&["screenshotcm://capture-window/"])),
            Request::Run(Command::CaptureWindow)
        );
    }

    #[test]
    fn a_bare_path_opens_the_editor() {
        assert_eq!(
            parse_args(&args(&[r"C:\x.png"])),
            Request::Annotate(r"C:\x.png".into())
        );
    }

    #[test]
    fn forwarded_requests_survive_the_round_trip() {
        for r in [
            Request::Run(Command::SelfTimer),
            Request::Annotate(r"D:\with space\a.png".into()),
            Request::Pin(r"C:\b.jpg".into()),
            Request::None,
        ] {
            assert_eq!(decode(&encode(&r)), r);
        }
    }

    #[test]
    fn truncated_percent_escape_is_kept_literally() {
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz"), "a%zz");
        assert_eq!(percent_decode("ñ%ñ"), "ñ%ñ");
    }
}
