//! Shared language preference (port of server/language.py).
//!
//! Message catalogs are the extension's `_locales/*/messages.json`. They are
//! read from the repository at run time when it can be found, so both servers
//! print the same text; a copy compiled into the executable is the fallback.

use crate::ffi;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const LANGUAGES: [&str; 5] = ["en", "zh-TW", "zh-CN", "ja", "ko"];

const EMBEDDED: [(&str, &str); 5] = [
    ("en", include_str!("../../extension/_locales/en/messages.json")),
    ("zh-TW", include_str!("../../extension/_locales/zh_TW/messages.json")),
    ("zh-CN", include_str!("../../extension/_locales/zh_CN/messages.json")),
    ("ja", include_str!("../../extension/_locales/ja/messages.json")),
    ("ko", include_str!("../../extension/_locales/ko/messages.json")),
];

/// The GeminiForge folder: `GF_ROOT`, else the nearest folder above the
/// executable that holds `extension/_locales`.
pub fn repo_root() -> Option<PathBuf> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        if let Ok(v) = std::env::var("GF_ROOT") {
            if !v.is_empty() {
                return Some(PathBuf::from(v));
            }
        }
        let exe = std::env::current_exe().ok()?;
        let mut dir = exe.parent();
        while let Some(d) = dir {
            if d.join("extension").join("_locales").is_dir() {
                return Some(d.to_path_buf());
            }
            dir = d.parent();
        }
        None
    })
    .clone()
}

/// `~/.config/GeminiForge/language.txt` (or under XDG_CONFIG_HOME).
#[cfg(not(windows))]
fn preference_file() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(".config"));
    base.join("GeminiForge").join("language.txt")
}

#[cfg(windows)]
fn preference_file() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok())
        .unwrap_or_else(|| ".".to_string());
    Path::new(&base).join("GeminiForge").join("language.txt")
}

/// Map a locale tag such as zh_Hant_TW or ja-JP to a supported language (English otherwise).
pub fn normalize(tag: &str) -> &'static str {
    let lower = tag.to_lowercase().replace('_', "-");
    let parts: Vec<&str> = lower.split('-').collect();
    if parts[0] == "zh" {
        if parts.contains(&"hant") {
            return "zh-TW";
        }
        if parts.contains(&"hans") {
            return "zh-CN";
        }
        return if parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) { "zh-TW" } else { "zh-CN" };
    }
    LANGUAGES.iter().find(|l| **l == parts[0]).copied().unwrap_or("en")
}

pub fn saved_language() -> Option<&'static str> {
    let text = std::fs::read(preference_file()).ok()?;
    let text = String::from_utf8(text).ok()?;
    let value = crate::py::strip(&text);
    LANGUAGES.iter().find(|l| **l == value).copied()
}

/// The display language: on macOS the first of the preferred languages,
/// else the locale variables (LC_ALL, LC_MESSAGES, LANG).
#[cfg(not(windows))]
pub fn system_language() -> &'static str {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(out) = out {
            // ( "zh-Hant-TW", "en-US" )
            let text = String::from_utf8_lossy(&out.stdout);
            let first = text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')).find(|s| !s.is_empty());
            if let Some(tag) = first {
                return normalize(tag);
            }
        }
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let tag = v.split(['.', '@']).next().unwrap_or("");
            if !tag.is_empty() && tag != "C" && tag != "POSIX" {
                return normalize(tag);
            }
        }
    }
    "en"
}

/// The Windows display language.
#[cfg(windows)]
pub fn system_language() -> &'static str {
    let lcid = unsafe { ffi::GetUserDefaultUILanguage() } as u32;
    let mut buf = [0u16; 85];
    let n = unsafe { ffi::LCIDToLocaleName(lcid, buf.as_mut_ptr(), buf.len() as i32, 0) };
    if n <= 1 {
        return "en";
    }
    normalize(&String::from_utf16_lossy(&buf[..(n as usize - 1)]))
}

pub fn current_language() -> &'static str {
    saved_language().unwrap_or_else(system_language)
}

/// Save the language picked in the extension panel. `Err` means "Unsupported language".
pub fn save_language(value: &str) -> Result<(), std::io::Error> {
    let file = preference_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_file_name(format!("tmp{}", ffi::token_hex(4)));
    std::fs::write(&tmp, value.as_bytes())?;
    let res = std::fs::rename(&tmp, &file);
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn parse_catalog(text: &str) -> Option<HashMap<String, String>> {
    let v: Value = serde_json::from_str(text).ok()?;
    let obj = v.as_object()?;
    let mut out = HashMap::new();
    for (k, item) in obj {
        out.insert(k.clone(), item.get("message")?.as_str()?.to_string());
    }
    Some(out)
}

fn catalog(language: &'static str) -> &'static HashMap<String, String> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, &'static HashMap<String, String>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = cache.get(language) {
        return c;
    }
    let from_repo = repo_root().and_then(|root| {
        let path = root.join("extension").join("_locales").join(language.replace('-', "_")).join("messages.json");
        parse_catalog(&std::fs::read_to_string(path).ok()?)
    });
    let loaded = from_repo.unwrap_or_else(|| {
        let text = EMBEDDED.iter().find(|(l, _)| *l == language).map(|(_, t)| *t).unwrap_or(EMBEDDED[0].1);
        parse_catalog(text).unwrap_or_default()
    });
    let leaked: &'static HashMap<String, String> = Box::leak(Box::new(loaded));
    cache.insert(language, leaked);
    leaked
}

/// `template.format(**values)` for the simple `{name}` placeholders the catalogs use.
pub fn format(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            if chars.get(i + 1) == Some(&'{') {
                out.push('{');
                i += 2;
                continue;
            }
            if let Some(end) = chars[i + 1..].iter().position(|x| *x == '}') {
                let key: String = chars[i + 1..i + 1 + end].iter().collect();
                match values.iter().find(|(k, _)| *k == key) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push('{');
                        out.push_str(&key);
                        out.push('}');
                    }
                }
                i += end + 2;
                continue;
            }
        }
        if c == '}' && chars.get(i + 1) == Some(&'}') {
            out.push('}');
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Translate a catalog key into the current language, filling {placeholders}.
pub fn t(key: &str, values: &[(&str, &str)]) -> String {
    t_in(current_language(), key, values)
}

pub fn t_in(language: &'static str, key: &str, values: &[(&str, &str)]) -> String {
    let template = catalog(language)
        .get(key)
        .or_else(|| catalog("en").get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string());
    format(&template, values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_tags_map_to_supported_languages() {
        assert_eq!(normalize("zh_Hant_TW"), "zh-TW");
        assert_eq!(normalize("zh-HK"), "zh-TW");
        assert_eq!(normalize("zh_CN"), "zh-CN");
        assert_eq!(normalize("zh-Hans-SG"), "zh-CN");
        assert_eq!(normalize("ja-JP"), "ja");
        assert_eq!(normalize("ko_KR"), "ko");
        assert_eq!(normalize("fr-FR"), "en");
        assert_eq!(normalize(""), "en");
    }

    #[test]
    fn catalogs_have_matching_keys_and_placeholders() {
        let english = parse_catalog(EMBEDDED[0].1).unwrap();
        for (language, text) in EMBEDDED {
            let cat = parse_catalog(text).unwrap();
            let mut keys: Vec<_> = cat.keys().collect();
            let mut en_keys: Vec<_> = english.keys().collect();
            keys.sort();
            en_keys.sort();
            assert_eq!(keys, en_keys, "{language}");
        }
        assert_eq!(t_in("en", "bkSaved", &[("id", "X")]), " (backed up as X; restore with restore_backup)");
        assert_eq!(format("{a} and {{b}} {c}", &[("a", "1")]), "1 and {b} {c}");
    }
}
