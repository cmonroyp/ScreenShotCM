//! A file logger: the agent has no console, so whatever goes wrong has to be
//! readable afterwards. One file, rolled at 1 MiB into one older copy —
//! checked as it grows, not only at start: the agent stays resident for
//! weeks, and a failure that repeats can write a line every few
//! milliseconds.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

struct Log {
    path: PathBuf,
    /// Bytes written since the size was last read from disk.
    since_check: u64,
}

static FILE: Mutex<Option<Log>> = Mutex::new(None);
const MAX_BYTES: u64 = 1024 * 1024;
/// How much is written between two looks at the file's size.
const CHECK_EVERY: u64 = 64 * 1024;

pub fn init(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("agent.log");
    roll_if_large(&path);
    *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Log {
        path,
        since_check: 0,
    });

    std::panic::set_hook(Box::new(|info| {
        write("PANIC", &info.to_string());
    }));
}

fn roll_if_large(path: &std::path::Path) {
    if fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(path, path.with_file_name("agent.old.log"));
    }
}

pub fn write(level: &str, message: &str) {
    let mut guard = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(log) = guard.as_mut() else { return };
    let line = format!("{} {level} {message}\n", timestamp());
    log.since_check += line.len() as u64;
    if log.since_check > CHECK_EVERY {
        log.since_check = 0;
        roll_if_large(&log.path);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log.path) {
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn info(message: &str) {
    write("INFO", message);
}

pub fn error(message: &str) {
    write("ERROR", message);
}

/// Local time as `YYYY-MM-DD HH:MM:SS.mmm`.
pub fn timestamp() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}
