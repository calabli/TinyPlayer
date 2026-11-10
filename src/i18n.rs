//! Interface language. The English text in the code is the key: `tr` shows
//! it as is in English, or its entry in `locales/zh-CN.tsv` in Chinese.
//!
//! A key may end in "|context" to tell apart texts that read the same in
//! English but not in Chinese ("Off" and "Off|short"). That part never shows.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Shown in each language's own name, so they're never translated.
pub const LANGUAGE_NAMES: [(&str, &str); 2] = [("zh", "\u{4E2D}\u{6587}"), ("en", "English")];

/// The language row is labeled in both, so it can be found from either.
pub const LANGUAGE_LABEL: &str = "\u{8BED}\u{8A00} / Language";

/// Chinese until the settings say otherwise, as Chinese is the default.
static CHINESE: AtomicBool = AtomicBool::new(true);

const ZH: &str = include_str!("../locales/zh-CN.tsv");

pub fn set_language(code: &str) {
    CHINESE.store(code != "en", Ordering::Relaxed);
}

pub fn is_english() -> bool {
    !CHINESE.load(Ordering::Relaxed)
}

/// The table's lines: a key, a tab and its Chinese. `#` starts a comment.
fn entries(table: &'static str) -> impl Iterator<Item = (&'static str, &'static str)> {
    table.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t'))
}

fn lookup_zh(key: &str) -> Option<&'static str> {
    static TABLE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    TABLE.get_or_init(|| entries(ZH).collect()).get(key).copied()
}

/// The English text of a key: the key without its context.
fn english(key: &str) -> &str {
    key.split_once('|').map_or(key, |(text, _)| text)
}

/// Translates `key`. Text without an entry, like a user's own collection
/// name, comes back unchanged.
pub fn tr(key: &str) -> &str {
    if CHINESE.load(Ordering::Relaxed)
        && let Some(zh) = lookup_zh(key) {
            return zh;
        }
    english(key)
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
    use std::collections::HashSet;
    use std::path::Path;

    /// Keys kept in `const`s or passed around before `tr` sees them.
    const SHOWN_ELSEWHERE: [&str; 32] = [
            "Off", "Bass", "Voice", "Treble", "Cinema", "Favorites", "Brightness", "Contrast", "Saturation", "Gamma", "Hue",
            // Reasons for a failed update, translated in the dialog.
            "Download failed. Check your network", "No update package for this system", "Couldn't write the update files", "Update checksum unavailable", "The download failed its check and was deleted", "Couldn't prepare the update",
            // Duplicate check levels and dialog errors in the library's Manage page.
            "Strict", "Normal", "Loose", "Enter a set name", "Choose at least one folder", "Choose a folder", "The videos are already here", "Enter a name without special characters", "That name is already taken", "Couldn't create a folder here", "A folder can't move into itself", "Couldn't rename this file", "Enter a tag name", "Use at least 4 characters", "The two entries don't match",
    ];

    /// Every source file, with its test code cut off.
    fn sources() -> Vec<(String, String)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((path.file_name().unwrap().to_string_lossy().into_owned(), text));
            }
        }
        out
    }

    fn code(text: &str) -> &str {
        text.split("#[cfg(test)]").next().unwrap_or(text)
    }

    /// String literals in `text`, with `\"`, `\\`, `\n` and `\t` read as the
    /// characters they stand for.
    fn literals(text: &str) -> Vec<String> {
        let c: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < c.len() {
            match c[i] {
                '/' if c.get(i + 1) == Some(&'/') => {
                    while i < c.len() && c[i] != '\n' {
                        i += 1;
                    }
                }
                // A char literal ('x' or '\n'); a lifetime has no closing quote.
                '\'' if c.get(i + 1) == Some(&'\\') => {
                    i += 2;
                    while i < c.len() && c[i] != '\'' {
                        i += 1;
                    }
                }
                '\'' if c.get(i + 2) == Some(&'\'') => i += 2,
                '"' => {
                    let mut lit = String::new();
                    i += 1;
                    while i < c.len() && c[i] != '"' {
                        if c[i] == '\\' {
                            i += 1;
                            match c.get(i) {
                                Some('n') => lit.push('\n'),
                                Some('t') => lit.push('\t'),
                                Some(e) => lit.push(*e),
                                None => {}
                            }
                        } else {
                            lit.push(c[i]);
                        }
                        i += 1;
                    }
                    out.push(lit);
                }
                _ => {}
            }
            i += 1;
        }
        out
    }

    /// Keys passed to `tr` or `tf` as a literal.
    fn keys() -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (name, text) in sources() {
            let code = code(&text);
            for call in ["tr(\"", "tf(\""] {
                for (i, _) in code.match_indices(call) {
                    if code[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let rest = &code[i + call.len() - 1..];
                    if let Some(lit) = literals(rest).into_iter().next() {
                        out.push((lit, name.clone()));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn the_code_has_no_chinese() {
        // Chinese lives in locales/; code that needs a Chinese character
        // (matching file names) writes it as an escape.
        let cjk = |c: char| ('\u{3000}'..='\u{303F}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{FF00}'..='\u{FFEF}').contains(&c);
        let found: Vec<_> = sources()
            .iter()
            .flat_map(|(name, text)| text.lines().enumerate().filter(|(_, l)| l.chars().any(cjk)).map(move |(i, _)| format!("{name}:{}", i + 1)))
            .collect();
        assert!(found.is_empty(), "Chinese in {found:#?}");
    }

    #[test]
    fn every_key_has_a_chinese_entry() {
        let mut missing: Vec<_> = keys().into_iter().filter(|(k, _)| lookup_zh(k).is_none()).collect();
        missing.extend(SHOWN_ELSEWHERE.iter().filter(|k| lookup_zh(k).is_none()).map(|k| (k.to_string(), "SHOWN_ELSEWHERE".into())));
        assert!(missing.is_empty(), "no Chinese for {missing:#?}");
    }

    #[test]
    fn table_has_no_duplicate_or_unused_keys() {
        let used: HashSet<String> = sources().iter().flat_map(|(_, text)| literals(code(text))).collect();
        let mut seen = HashSet::new();
        for (key, _) in entries(ZH) {
            assert!(seen.insert(key), "duplicate {key}");
            assert!(used.contains(key), "unused {key}");
        }
    }

    #[test]
    fn translations_keep_their_slots() {
        for (key, zh) in entries(ZH) {
            assert!(!zh.trim().is_empty(), "{key}");
            assert_eq!(english(key).matches("{}").count(), zh.matches("{}").count(), "{key}");
        }
    }

    #[test]
    fn context_never_shows() {
        assert_eq!(english("Off|short"), "Off");
        assert_eq!(english("Off"), "Off");
    }

    #[test]
    fn fill_replaces_slots_in_order() {
        assert_eq!(fill("{} of {}", &[&1, &"two"]), "1 of two");
        assert_eq!(fill("no slots", &[&1]), "no slots");
        assert_eq!(fill("{} and {}", &[&1]), "1 and {}");
    }
}
