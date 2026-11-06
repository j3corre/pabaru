//! Minimal localisation of the GUI (English + Czech).
//!
//! The language is detected once from the operating system locale
//! (`sys-locale`) and can be overridden with the `PABARU_LANG` environment
//! variable (`en` / `cs`). Texts are written inline next to each other:
//!
//! ```ignore
//! ui.label(tr("Restore", "Obnovit"));
//! ui.label(format!("{} {n}", tr("Pages:", "Stránky:")));
//! ```
//!
//! Keeping both variants at the call site makes it obvious which texts are
//! translated and avoids a separate (easily outdated) message catalogue.

use std::sync::atomic::{AtomicU8, Ordering};

use pabaru_core::{Printer, Profile};

/// Supported UI language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Cs,
}

/// Currently active language (global – the UI and worker threads share it).
static LANG: AtomicU8 = AtomicU8::new(0);

/// Sets the active language.
pub fn set_lang(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

/// Returns the active language.
pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Cs,
        _ => Lang::En,
    }
}

/// Picks the language from a locale tag such as `cs-CZ`, `cs_CZ.UTF-8` or `en-US`.
/// Czech for `cs*`, English for everything else.
pub fn lang_from_locale(tag: &str) -> Lang {
    if tag.trim().to_ascii_lowercase().starts_with("cs") {
        Lang::Cs
    } else {
        Lang::En
    }
}

/// Detects the language: `PABARU_LANG` overrides the system locale.
pub fn detect() -> Lang {
    if let Ok(v) = std::env::var("PABARU_LANG") {
        return lang_from_locale(&v);
    }
    sys_locale::get_locale()
        .map(|l| lang_from_locale(&l))
        .unwrap_or(Lang::En)
}

/// Returns the text in the active language.
pub fn tr(en: &'static str, cs: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        Lang::Cs => cs,
    }
}

/// Formats a number with `digits` decimal places using the decimal separator
/// of the active language (`1.5` in English, `1,5` in Czech).
pub fn decimal(value: f64, digits: usize) -> String {
    let s = format!("{value:.digits$}");
    match lang() {
        Lang::En => s,
        Lang::Cs => s.replace('.', ","),
    }
}

/// Localised printer name.
pub fn printer_label(p: Printer) -> &'static str {
    match p {
        Printer::Laser600 => tr("Laser 600 dpi", "Laserová 600 dpi"),
        Printer::Laser1200 => tr("Laser 1200 dpi", "Laserová 1200 dpi"),
        Printer::Inkjet => tr("Inkjet", "Inkoustová"),
    }
}

/// Localised profile name.
pub fn profile_label(p: Profile) -> &'static str {
    match p {
        Profile::Compact => tr("Compact", "Kompaktní"),
        Profile::Balanced => tr("Balanced", "Vyvážený"),
        Profile::Archival => tr("Archival", "Archivní"),
    }
}

/// Localised profile description.
pub fn profile_description(p: Profile) -> &'static str {
    match p {
        Profile::Compact => tr(
            "Least paper. Needs good print and scan quality.",
            "Nejméně papíru. Vyžaduje kvalitní tisk a sken.",
        ),
        Profile::Balanced => tr(
            "A sensible balance between page count and robustness.",
            "Rozumný kompromis mezi počtem stran a odolností.",
        ),
        Profile::Archival => tr(
            "For long-term storage. Survives the loss of a whole page.",
            "Pro dlouhodobé uložení. Přežije ztrátu celé stránky.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_tags() {
        assert_eq!(lang_from_locale("cs-CZ"), Lang::Cs);
        assert_eq!(lang_from_locale("cs_CZ.UTF-8"), Lang::Cs);
        assert_eq!(lang_from_locale("CS"), Lang::Cs);
        assert_eq!(lang_from_locale("en-US"), Lang::En);
        assert_eq!(lang_from_locale("de-DE"), Lang::En);
        assert_eq!(lang_from_locale(""), Lang::En);
    }
}
