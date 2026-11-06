//! Interface language. The Chinese text in the code is the key: `tr` returns
//! it as is, or its entry in [`crate::i18n_en::EN`] when English is on.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Shown in each language's own name, so they're never translated.
pub const LANGUAGE_NAMES: [(&str, &str); 2] = [("zh", "\u{4E2D}\u{6587}"), ("en", "English")];

/// The language row is labeled in both, so it can be found from either.
pub const LANGUAGE_LABEL: &str = "\u{8BED}\u{8A00} / Language";

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_language(code: &str) {
    ENGLISH.store(code == "en", Ordering::Relaxed);
}

fn lookup_en(key: &str) -> Option<&'static str> {
    static TABLE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    TABLE.get_or_init(|| crate::i18n_en::EN.iter().copied().collect()).get(key).copied()
}

/// Translates `key`. Text without an entry, like a user's own collection
/// name, comes back unchanged.
pub fn tr(key: &str) -> &str {
    if ENGLISH.load(Ordering::Relaxed) { lookup_en(key).unwrap_or(key) } else { key }
}

/// Translates `key` and fills its `{}` slots with `args`, in order.
pub fn tf(key: &str, args: &[&dyn Display]) -> String {
    fill(tr(key), args)
}

fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut rest = template;
    for arg in args {
        let Some((before, after)) = rest.split_once("{}") else { break };
        out.push_str(before);
        out.push_str(&arg.to_string());
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCES: [(&str, &str); 12] = [
        ("app", include_str!("app.rs")),
        ("audio_fx", include_str!("audio_fx.rs")),
        ("library", include_str!("library.rs")),
        ("library_ui", include_str!("library_ui.rs")),
        ("marks", include_str!("marks.rs")),
        ("player", include_str!("player.rs")),
        ("quick_panel", include_str!("quick_panel.rs")),
        ("settings_panel", include_str!("settings_panel.rs")),
        ("hover_thumbs", include_str!("hover_thumbs.rs")),
        ("scanner", include_str!("scanner.rs")),
        ("session", include_str!("session.rs")),
        ("update", include_str!("update.rs")),
    ];

    fn has_cjk(s: &str) -> bool {
        s.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c))
    }

    /// Every Chinese string literal in the non-test code.
    fn keys() -> Vec<(String, &'static str)> {
        let mut out = Vec::new();
        for (name, src) in SOURCES {
            let code = src.split("#[cfg(test)]").next().unwrap_or(src);
            for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
                let line = line.split(" // ").next().unwrap_or(line);
                let mut parts = line.split('"');
                parts.next();
                while let Some(lit) = parts.next() {
                    if has_cjk(lit) {
                        out.push((lit.to_string(), name));
                    }
                    parts.next();
                }
            }
        }
        out
    }

    #[test]
    fn every_chinese_string_has_an_english_entry() {
        let missing: Vec<_> = keys().into_iter().filter(|(k, _)| lookup_en(k).is_none()).collect();
        assert!(missing.is_empty(), "no translation for {missing:#?}");
    }

    /// Catches text that would stay Chinese in English mode. Strings kept in
    /// `const`s are translated where they're shown, so they're listed here.
    #[test]
    fn chinese_strings_go_through_translation() {
        const SHOWN_ELSEWHERE: [&str; 11] =
            ["关闭", "低音", "人声", "高音", "影院", "默认收藏", "亮度", "对比度", "饱和度", "伽马", "色调"];
        let mut bare = Vec::new();
        for (name, src) in SOURCES {
            let code = src.split("#[cfg(test)]").next().unwrap_or(src);
            for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
                let line = line.split(" // ").next().unwrap_or(line);
                let parts: Vec<&str> = line.split('"').collect();
                for pair in parts.chunks(2) {
                    let [before, lit] = pair else { continue };
                    let wrapped = before.ends_with(" tr(") || before.ends_with("(tr(") || before.ends_with("[tr(")
                        || before.ends_with("&tr(") || before.ends_with("tf(") || before == &"tr(";
                    if has_cjk(lit) && !wrapped && !SHOWN_ELSEWHERE.contains(lit) {
                        bare.push((name, lit.to_string()));
                    }
                }
            }
        }
        assert!(bare.is_empty(), "not translated: {bare:#?}");
    }

    #[test]
    fn translations_have_no_chinese_and_keep_their_slots() {
        for (key, en) in crate::i18n_en::EN {
            assert!(!has_cjk(en), "{key} -> {en}");
            assert_eq!(key.matches("{}").count(), en.matches("{}").count(), "{key}");
        }
    }

    #[test]
    fn table_has_no_duplicate_or_unused_keys() {
        let used: std::collections::HashSet<String> = keys().into_iter().map(|(k, _)| k).collect();
        let mut seen = std::collections::HashSet::new();
        for (key, _) in crate::i18n_en::EN {
            assert!(seen.insert(*key), "duplicate {key}");
            assert!(used.contains(*key), "unused {key}");
        }
    }

    #[test]
    fn fill_replaces_slots_in_order() {
        assert_eq!(fill("{} of {}", &[&1, &"two"]), "1 of two");
        assert_eq!(fill("no slots", &[&1]), "no slots");
        assert_eq!(fill("{} and {}", &[&1]), "1 and {}");
    }
}
