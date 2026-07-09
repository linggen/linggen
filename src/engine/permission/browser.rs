//! Browser-action permission class — see `doc/browser-control-spec.md`.
//!
//! Browser actions gate on *site trust*, not filesystem paths: read-class
//! ops run free, mutating ops prompt per action until the user grants the
//! origin for the session, and a short hard floor (payment, credentials,
//! deletion, posting as the user) always confirms — even on a trusted site.
//!
//! Pure classification only. The gate that prompts and grants lives in
//! `engine/browser_gate.rs`; origin lookup in `engine/tools/browser_tool.rs`.

use crate::engine::tools::browser_tool::BrowserNodeMeta;
use serde_json::Value;

/// True for every `Browser_*` engine tool.
pub fn is_browser_tool(tool: &str) -> bool {
    tool.starts_with("Browser_")
}

/// True when the action can change site state and must pass the site-trust
/// gate. Reads (`readPage`, `screenshot`, `scroll`, `wait`, `readConsole`,
/// `tabs list`) never mutate and run free.
pub fn browser_action_mutating(tool: &str, args: &Value) -> bool {
    match tool {
        "Browser_navigate" | "Browser_click" | "Browser_type" | "Browser_key" => true,
        // `open` navigates somewhere; list/switch/close only touch the
        // agent's own controlled tab, never site state.
        "Browser_tabs" => {
            args.get("action").and_then(Value::as_str).unwrap_or("list") == "open"
        }
        _ => false,
    }
}

/// Hard-floor categories: actions that never auto-execute, even on a
/// trusted site. Matched heuristically against the target element's
/// accessible name from the last `Browser_readPage` — a coordinate click
/// with no ref has no name and cannot be recognized, which is one reason
/// refs are the preferred targeting mode.
const FLOOR_PAYMENT: &[&str] = &[
    "pay", "buy", "purchase", "checkout", "place order", "order now", "subscribe",
    "add card", "billing",
];
const FLOOR_SECURITY: &[&str] = &["password", "passkey", "two-factor", "2fa", "security key"];
const FLOOR_DESTRUCTIVE: &[&str] = &["delete", "remove", "deactivate", "erase", "uninstall"];
const FLOOR_PUBLISH: &[&str] = &["post", "tweet", "send", "reply", "publish", "share"];

/// Whole-word / whole-phrase match: "Post" hits, "postpone" doesn't.
fn matches_word(name: &str, needle: &str) -> bool {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let needle_words: Vec<&str> = needle.split(' ').collect();
    words
        .windows(needle_words.len())
        .any(|w| w == needle_words.as_slice())
}

/// The hard-floor category for a target element, if any.
pub fn browser_hard_floor(meta: &BrowserNodeMeta) -> Option<&'static str> {
    let name = meta.name.to_lowercase();
    if name.is_empty() {
        return None;
    }
    for (category, needles) in [
        ("payment", FLOOR_PAYMENT),
        ("security", FLOOR_SECURITY),
        ("destructive", FLOOR_DESTRUCTIVE),
        ("posting", FLOOR_PUBLISH),
    ] {
        if needles.iter().any(|n| matches_word(&name, n)) {
            return Some(category);
        }
    }
    None
}

/// One-line description of the action for the confirmation prompt.
pub fn browser_action_summary(tool: &str, args: &Value, meta: Option<&BrowserNodeMeta>) -> String {
    let arg = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    let target = || match meta {
        Some(m) if !m.name.is_empty() => format!("{} \"{}\"", m.role, m.name),
        Some(m) => m.role.clone(),
        None => match args.get("coordinate").and_then(Value::as_array) {
            Some(c) => format!("coordinate {:?}", c),
            None => arg("ref").to_string(),
        },
    };
    match tool {
        "Browser_navigate" => format!("Navigate to {}", arg("url")),
        "Browser_click" => format!("Click {}", target()),
        "Browser_type" => {
            let text: String = arg("text").chars().take(60).collect();
            format!("Type \"{}\" into {}", text, target())
        }
        "Browser_key" => format!("Press {}", arg("keys")),
        "Browser_tabs" => match arg("action") {
            "open" => format!("Open {}", arg("url")),
            other => format!("Browser tabs: {other}"),
        },
        _ => tool.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(role: &str, name: &str) -> BrowserNodeMeta {
        BrowserNodeMeta { role: role.into(), name: name.into() }
    }

    #[test]
    fn read_ops_are_not_mutating() {
        for tool in [
            "Browser_readPage", "Browser_screenshot", "Browser_scroll",
            "Browser_wait", "Browser_readConsole",
        ] {
            assert!(!browser_action_mutating(tool, &json!({})), "{tool} must be read-class");
        }
        assert!(!browser_action_mutating("Browser_tabs", &json!({"action": "list"})));
        assert!(!browser_action_mutating("Browser_tabs", &json!({"action": "close"})));
    }

    #[test]
    fn mutating_ops_are_gated() {
        for tool in ["Browser_navigate", "Browser_click", "Browser_type", "Browser_key"] {
            assert!(browser_action_mutating(tool, &json!({})), "{tool} must be gated");
        }
        assert!(browser_action_mutating(
            "Browser_tabs",
            &json!({"action": "open", "url": "https://x.com"}),
        ));
    }

    #[test]
    fn hard_floor_matches_whole_words() {
        assert_eq!(browser_hard_floor(&meta("button", "Post")), Some("posting"));
        assert_eq!(browser_hard_floor(&meta("button", "Delete account")), Some("destructive"));
        assert_eq!(browser_hard_floor(&meta("button", "Pay now")), Some("payment"));
        assert_eq!(browser_hard_floor(&meta("button", "Place order")), Some("payment"));
        assert_eq!(browser_hard_floor(&meta("textbox", "Password")), Some("security"));
        assert_eq!(browser_hard_floor(&meta("button", "Send message")), Some("posting"));
        // Substrings must NOT trip the floor.
        assert_eq!(browser_hard_floor(&meta("link", "Postpone")), None);
        assert_eq!(browser_hard_floor(&meta("link", "Removed items archive")), None);
        assert_eq!(browser_hard_floor(&meta("button", "Compose")), None);
        assert_eq!(browser_hard_floor(&meta("button", "Search")), None);
        assert_eq!(browser_hard_floor(&meta("button", "")), None);
    }

    #[test]
    fn action_summary_is_readable() {
        assert_eq!(
            browser_action_summary("Browser_navigate", &json!({"url": "https://x.com"}), None),
            "Navigate to https://x.com",
        );
        assert_eq!(
            browser_action_summary(
                "Browser_click",
                &json!({"ref": "n3"}),
                Some(&meta("button", "Post")),
            ),
            "Click button \"Post\"",
        );
        assert_eq!(
            browser_action_summary("Browser_key", &json!({"keys": "Enter"}), None),
            "Press Enter",
        );
    }
}
