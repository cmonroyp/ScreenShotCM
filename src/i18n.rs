//! Spanish and English. Every string the user sees is written as
//! `tr("español", "English")` where it is used, so the two never drift
//! apart. The language is decided once at start — the setting, or Windows'
//! own display language — and again when the settings change.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Globalization::GetUserDefaultUILanguage;

use crate::settings::Language;

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// Spanish when Windows is in Spanish, English otherwise.
fn system_is_spanish() -> bool {
    // The primary language is the low ten bits of the LANGID; 0x0A is
    // Spanish in every country.
    (unsafe { GetUserDefaultUILanguage() } & 0x3FF) == 0x0A
}

pub fn apply(language: Language) {
    let english = match language {
        Language::Spanish => false,
        Language::English => true,
        Language::Auto => !system_is_spanish(),
    };
    ENGLISH.store(english, Ordering::Relaxed);
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// The string in the current language.
pub fn tr(es: &'static str, en: &'static str) -> &'static str {
    if english() { en } else { es }
}

/// The same, for text built at run time.
pub fn trs(es: String, en: String) -> String {
    if english() { en } else { es }
}
