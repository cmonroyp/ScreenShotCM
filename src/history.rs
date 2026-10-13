//! Capture history: every capture is kept as a PNG in the local profile for
//! `history_days`, whatever the user does with it afterwards, so a capture
//! dismissed by mistake can always be found again.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::paths;

/// A sortable, collision-free name: local time to the millisecond.
pub fn new_path() -> PathBuf {
    let t = crate::log::timestamp().replace(':', ".");
    paths::history_dir().join(format!("{t}.png"))
}

/// Deletes history older than `days`. Zero keeps everything.
pub fn prune(days: u32) {
    if days == 0 {
        return;
    }
    let limit = Duration::from_secs(days as u64 * 86_400);
    prune_dir(&paths::history_dir(), limit, SystemTime::now());
}

fn prune_dir(dir: &Path, limit: Duration, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_image = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("png" | "jpg" | "partial")
        );
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > limit);
        if is_image && old {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_removes_only_old_images() {
        let dir = std::env::temp_dir().join(format!("scm-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        std::fs::write(dir.join("keep.txt"), b"x").unwrap();
        // "Now" is two days ahead, so the file just written is two days old.
        let later = SystemTime::now() + Duration::from_secs(2 * 86_400);
        prune_dir(&dir, Duration::from_secs(3 * 86_400), later);
        assert!(dir.join("a.png").exists());
        prune_dir(&dir, Duration::from_secs(86_400), later);
        assert!(!dir.join("a.png").exists());
        assert!(dir.join("keep.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
