//! The language Linggen writes in for the person — one answer for the whole
//! engine. The setting (`Config.language`, a BCP-47 tag) wins; unset means
//! Auto: the Mac's first preferred language, then English. Skill tools get
//! the answer as `LINGGEN_LANGUAGE`, pages ask `GET /api/user/language`, and
//! the prompt tells the model which language to write in.

use std::sync::{OnceLock, RwLock};

/// The person's explicit choice, mirrored from config so sync callers (tool
/// env, prompt build) never wait on the config lock.
static SETTING: RwLock<Option<String>> = RwLock::new(None);

/// The env var a skill tool's command reads the effective language from.
pub const ENV_KEY: &str = "LINGGEN_LANGUAGE";

const FALLBACK: &str = "en";

/// Record the configured language (None = Auto). Called when config loads
/// and whenever it is applied.
pub fn set_setting(setting: Option<String>) {
    let clean = setting
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Ok(mut guard) = SETTING.write() {
        *guard = clean;
    }
}

/// The configured language, or None for Auto.
pub fn setting() -> Option<String> {
    SETTING.read().ok().and_then(|g| g.clone())
}

/// The language in effect: the setting, else the Mac's, else English.
pub fn effective() -> String {
    setting()
        .or_else(system_language)
        .unwrap_or_else(|| FALLBACK.to_string())
}

/// The language's English name, for the prompt ("Simplified Chinese").
/// An unknown tag names itself.
pub fn display_name(tag: &str) -> String {
    const NAMES: &[(&str, &str)] = &[
        ("en", "English"),
        ("zh-Hans", "Simplified Chinese"),
        ("zh-Hant", "Traditional Chinese"),
        ("zh", "Chinese"),
        ("ja", "Japanese"),
        ("ko", "Korean"),
        ("fr", "French"),
        ("de", "German"),
        ("es", "Spanish"),
        ("pt", "Portuguese"),
        ("it", "Italian"),
        ("ru", "Russian"),
    ];
    NAMES
        .iter()
        .find(|(t, _)| t.eq_ignore_ascii_case(tag))
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| tag.to_string())
}

/// The Mac's first preferred language, normalized. Read once per process:
/// the preference rarely changes, and a shell-out per request is waste.
fn system_language() -> Option<String> {
    static SYSTEM: OnceLock<Option<String>> = OnceLock::new();
    SYSTEM.get_or_init(read_apple_languages).clone()
}

fn read_apple_languages() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    first_apple_language(&String::from_utf8_lossy(&out.stdout)).map(|t| normalize(&t))
}

/// The first entry of `defaults read -g AppleLanguages` — a plist array
/// like `(\n    "zh-Hans-CA",\n    "en-CA"\n)`.
fn first_apple_language(plist: &str) -> Option<String> {
    plist
        .lines()
        .map(|l| l.trim().trim_end_matches(',').trim_matches('"').trim())
        .find(|l| !l.is_empty() && *l != "(" && *l != ")")
        .map(str::to_string)
}

/// A preferred-language tag cut to what Linggen writes by: the language,
/// plus its script when it has one; the region is dropped ("zh-Hans-CA" →
/// "zh-Hans", "en-CA" → "en"). Chinese without a script takes it from the
/// region ("zh-TW" → "zh-Hant", "zh-CN" → "zh-Hans").
pub fn normalize(tag: &str) -> String {
    let mut parts = tag.trim().split(['-', '_']).filter(|p| !p.is_empty());
    let lang = parts.next().unwrap_or(FALLBACK).to_ascii_lowercase();
    let rest: Vec<&str> = parts.collect();
    let script = rest.iter().find(|p| is_script(p)).map(|s| title_case(s));
    match (lang.as_str(), script) {
        (_, Some(script)) => format!("{lang}-{script}"),
        ("zh", None) => format!("zh-{}", chinese_script_for(&rest)),
        _ => lang,
    }
}

fn is_script(subtag: &str) -> bool {
    subtag.len() == 4 && subtag.chars().all(|c| c.is_ascii_alphabetic())
}

fn title_case(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let mut chars = lower.chars();
    chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

fn chinese_script_for(subtags: &[&str]) -> &'static str {
    const TRADITIONAL: &[&str] = &["TW", "HK", "MO"];
    let traditional = subtags
        .iter()
        .any(|r| TRADITIONAL.iter().any(|t| t.eq_ignore_ascii_case(r)));
    if traditional {
        "Hant"
    } else {
        "Hans"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_is_dropped_and_a_script_kept() {
        assert_eq!(normalize("zh-Hans-CA"), "zh-Hans");
        assert_eq!(normalize("zh-Hant-TW"), "zh-Hant");
        assert_eq!(normalize("en-CA"), "en");
        assert_eq!(normalize("en"), "en");
        assert_eq!(normalize("ja_JP"), "ja");
    }

    #[test]
    fn chinese_without_a_script_takes_it_from_the_region() {
        assert_eq!(normalize("zh-TW"), "zh-Hant");
        assert_eq!(normalize("zh-HK"), "zh-Hant");
        assert_eq!(normalize("zh-CN"), "zh-Hans");
        assert_eq!(normalize("zh"), "zh-Hans");
    }

    #[test]
    fn the_first_apple_language_is_read_from_the_plist() {
        let plist = "(\n    \"zh-Hans-CA\",\n    \"en-CA\"\n)\n";
        assert_eq!(first_apple_language(plist).as_deref(), Some("zh-Hans-CA"));
        assert_eq!(first_apple_language("(\n)\n"), None);
    }

    #[test]
    fn a_language_is_named_in_english() {
        assert_eq!(display_name("zh-Hans"), "Simplified Chinese");
        assert_eq!(display_name("en"), "English");
        assert_eq!(display_name("xx-Yyyy"), "xx-Yyyy");
    }
}
