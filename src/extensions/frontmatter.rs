//! YAML-frontmatter parsing helpers shared by skills and missions.
//!
//! The on-disk shape is `---\n<yaml>\n---\n<body>`. Use [`split`] to
//! get the two halves; deserialise the YAML half with your own struct.
//! [`parse_meta`] is a convenience for the common (`name`, `description`)
//! pair (used by the marketplace search path that doesn't need a full
//! parse). [`deserialize_string_or_vec`] supports the YAML idiom where a
//! list field may be written as either a comma-separated string or a
//! true list — used by `allowed-tools` / `allow-skills`.

use serde::{Deserialize, Deserializer};

/// Split `---\n<yaml>\n---\n<body>` into (yaml, body). Returns
/// `(None, full)` if the input doesn't start with `---` or no closing
/// delimiter is found. The yaml half is trimmed; the body is left as-is
/// (callers usually `.trim_start_matches('\n')` themselves).
///
/// Robust against `---` appearing inside the body (uses `\n---` as the
/// closing marker so a bare `---` mid-paragraph won't false-match).
pub fn split(content: &str) -> (Option<&str>, &str) {
    if !content.starts_with("---") {
        return (None, content);
    }
    let Some(end) = content[3..].find("\n---") else {
        return (None, content);
    };
    let yaml = &content[3..3 + end];
    let body = &content[3 + end + 4..];
    (Some(yaml.trim()), body)
}

/// Extract `name` and `description` from frontmatter. Convenience for
/// marketplace search and remote skill discovery — uses [`split`] then
/// a minimal deserializer that only requires those two fields.
pub fn parse_meta(text: &str) -> Option<(String, String)> {
    let (yaml, _) = split(text);
    let yaml = yaml?;

    #[derive(Deserialize)]
    struct Meta {
        name: String,
        description: String,
    }

    let meta: Meta = serde_yml::from_str(yaml).ok()?;
    Some((meta.name, meta.description))
}

/// Serde deserialiser for a list field that accepts either:
///   - a single string (`allowed-tools: "Bash, Read"` → split on `,`), or
///   - a true list (`allowed-tools: [Bash, Read]`).
///
/// Used on `allowed-tools` and `allow-skills`. Returns `None` when the
/// field is absent so callers can distinguish "empty" from "unset".
pub fn deserialize_string_or_vec<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Str(String),
        Vec(Vec<String>),
    }
    let opt: Option<StringOrVec> = Option::deserialize(deserializer)?;
    Ok(match opt {
        Some(StringOrVec::Str(s)) => Some(s.split(',').map(|s| s.trim().to_string()).collect()),
        Some(StringOrVec::Vec(v)) => Some(v),
        None => None,
    })
}
