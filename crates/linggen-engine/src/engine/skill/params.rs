//! Session parameters — values a skill session is bound to when it is made
//! (`doc/skill-spec.md` § Session parameters).
//!
//! A skill declares the names it takes and the shape of each value
//! (`session-params:` in SKILL.md). A session bound to the skill may carry
//! one value per declared name (`SessionMeta::params`), set once, at
//! creation, and checked against the declaration then and again on every
//! use. The skill's tool commands read them as `{{session.<name>}}`; its
//! `place.<agent>.absent_until` may read a state file named by them
//! (`StateFlag::session_file`). No value: exactly what the skill does
//! without one. The engine never knows what a value means.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The values one session is bound to, by declared name.
pub type SessionParams = BTreeMap<String, String>;

/// What a skill declares about one session parameter.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionParamDef {
    /// The whole value must match it (anchored by the engine). Absent:
    /// [`DEFAULT_PATTERN`].
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub description: String,
}

/// A value nobody declared a shape for: a word of letters, digits, `_`, `.`
/// and `-`, starting with a letter or digit, at most 64 long — no path
/// separator, never read as a flag.
pub const DEFAULT_PATTERN: &str = "[A-Za-z0-9][A-Za-z0-9_.-]{0,63}";

/// Longest value any pattern may admit.
const MAX_VALUE_BYTES: usize = 128;

impl SessionParamDef {
    /// Whether `value` may be bound: the declared pattern (anchored) or the
    /// default, and always non-empty, at most 128 bytes, no control
    /// character, no leading `-`.
    pub fn accepts(&self, value: &str) -> bool {
        if value.is_empty()
            || value.len() > MAX_VALUE_BYTES
            || value.starts_with('-')
            || value.chars().any(char::is_control)
        {
            return false;
        }
        let pattern = self.pattern.as_deref().unwrap_or(DEFAULT_PATTERN);
        regex::Regex::new(&format!("^(?:{pattern})$")).is_ok_and(|re| re.is_match(value))
    }
}

/// A parameter name: `[a-z][a-z0-9_]{0,31}` — it reads as `{{session.<name>}}`.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The declarations that hold: a bad name or a pattern that doesn't compile
/// is logged and dropped — nothing can be bound to it.
pub fn usable(
    skill: &str,
    declared: BTreeMap<String, SessionParamDef>,
) -> BTreeMap<String, SessionParamDef> {
    declared
        .into_iter()
        .filter(|(name, def)| {
            let pattern_ok = def
                .pattern
                .as_deref()
                .is_none_or(|p| regex::Regex::new(&format!("^(?:{p})$")).is_ok());
            let ok = valid_name(name) && pattern_ok;
            if !ok {
                tracing::warn!(
                    "skill '{skill}': session param '{name}' dropped (bad name or pattern)"
                );
            }
            ok
        })
        .collect()
}

/// Check `params` against what the skill declares: every name declared,
/// every value accepted. The message says which one failed.
pub fn check(
    declared: &BTreeMap<String, SessionParamDef>,
    params: &SessionParams,
) -> Result<(), String> {
    for (name, value) in params {
        let Some(def) = declared.get(name) else {
            return Err(format!("the skill declares no session param '{name}'"));
        };
        if !def.accepts(value) {
            return Err(format!(
                "session param '{name}' refuses {value:?}{}",
                def.pattern
                    .as_deref()
                    .map(|p| format!(" (pattern {p})"))
                    .unwrap_or_default()
            ));
        }
    }
    Ok(())
}

/// The prefix a session parameter's placeholder carries: `{{session.save}}`.
pub const PLACEHOLDER_PREFIX: &str = "session.";

/// Expand every `{{token}}` of `template` in one pass: `fill(token)` gives
/// its text, `None` leaves it as written. One pass, so a filled value that
/// itself reads like a placeholder is never expanded again.
pub fn render(template: &str, mut fill: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            out.push_str(&rest[open..]);
            return out;
        };
        let token = &after[..close];
        match fill(token) {
            Some(text) => out.push_str(&text),
            None => {
                out.push_str("{{");
                out.push_str(token);
                out.push_str("}}");
            }
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

/// The session parameter names a template reads (`{{session.<name>}}`).
pub fn names_in(template: &str) -> Vec<String> {
    let mut found = Vec::new();
    render(template, |token| {
        if let Some(name) = token.strip_prefix(PLACEHOLDER_PREFIX) {
            if !found.iter().any(|f| f == name) {
                found.push(name.to_string());
            }
        }
        None
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(pattern: Option<&str>) -> SessionParamDef {
        SessionParamDef {
            pattern: pattern.map(str::to_string),
            description: String::new(),
        }
    }

    #[test]
    fn the_default_pattern_takes_a_plain_word_only() {
        let d = def(None);
        for ok in ["test", "lx-gu3", "a_b.c", "A1"] {
            assert!(d.accepts(ok), "{ok}");
        }
        for bad in [
            "",
            "-x",
            "a/b",
            "../x",
            ".hidden",
            "a b",
            "it's",
            "a\nb",
            &"x".repeat(65),
        ] {
            assert!(!d.accepts(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_declared_pattern_is_anchored_and_the_hard_rules_still_hold() {
        let d = def(Some("[a-z]+"));
        assert!(d.accepts("abc"));
        assert!(!d.accepts("abc1"), "anchored: the whole value must match");
        let loose = def(Some(".*"));
        assert!(loose.accepts("a b/c"));
        assert!(!loose.accepts("-rf"), "never a flag");
        assert!(!loose.accepts(""), "never empty");
        assert!(!loose.accepts("a\u{0}b"), "no control chars");
        assert!(!loose.accepts(&"a".repeat(129)), "bounded");
    }

    #[test]
    fn check_names_the_undeclared_and_the_refused() {
        let declared = BTreeMap::from([("save".to_string(), def(Some("[a-z0-9-]{1,32}")))]);
        let ok = SessionParams::from([("save".into(), "lx-gu3".into())]);
        assert!(check(&declared, &ok).is_ok());
        assert!(check(&declared, &SessionParams::new()).is_ok());
        let other = SessionParams::from([("world".into(), "x".into())]);
        assert!(check(&declared, &other).unwrap_err().contains("'world'"));
        let bad = SessionParams::from([("save".into(), "x; rm -rf /".into())]);
        assert!(check(&declared, &bad).unwrap_err().contains("'save'"));
    }

    #[test]
    fn a_broken_declaration_is_dropped() {
        let declared = BTreeMap::from([
            ("save".to_string(), def(None)),
            ("Bad-Name".to_string(), def(None)),
            ("broken".to_string(), def(Some("(["))),
        ]);
        let kept = usable("zz", declared);
        assert_eq!(kept.keys().collect::<Vec<_>>(), vec!["save"]);
    }

    #[test]
    fn render_is_one_pass() {
        let out = render("a {{x}} {{y}} {{session.s}} {{", |t| match t {
            "x" => Some("{{y}}".into()),
            "session.s" => Some("S".into()),
            _ => None,
        });
        assert_eq!(out, "a {{y}} {{y}} S {{");
        assert_eq!(
            names_in("--save={{session.save}} {{a}} {{session.save}} {{session.w}}"),
            vec!["save", "w"]
        );
    }
}
