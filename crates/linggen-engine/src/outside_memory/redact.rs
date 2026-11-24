//! The one exit for text that came from another agent's files.
//!
//! Everything this module hands to a caller — the index block, a note, a
//! search hit — is a [`Clean`], and a `Clean` can be made only here, by
//! [`clean`]. Key-shaped strings (a provider key, a token, a private key, a
//! `password=…`) are replaced before the text leaves; the shapes are the
//! prefixes and forms real credentials have, never a length rule, so a long
//! file name or an id in an index line is left alone.

use regex::Regex;
use std::sync::LazyLock;

/// Text that has been through [`clean`]. The field is private to this file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clean(String);

impl Clean {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for Clean {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

const MASK: &str = "[redacted]";

/// Shapes that are a secret wherever they stand.
static WHOLE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)",
        r"\b(?:sk|pk|rk)-[A-Za-z0-9_-]{16,}",
        r"\bgh[pousr]_[A-Za-z0-9]{20,}",
        r"\bgithub_pat_[A-Za-z0-9_]{20,}",
        r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        r"\bxox[abeprs]-[A-Za-z0-9-]{10,}",
        r"\bAIza[0-9A-Za-z_-]{30,}",
        r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
        r"(?i)\bbearer[ \t]+[A-Za-z0-9._~+/-]{16,}=*",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("a redaction shape"))
    .collect()
});

/// `name = value` where the name says it is a secret: the value goes, the
/// name stays so the line still reads.
static ASSIGNED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(\b[A-Za-z0-9_]*(?:api[_-]?key|secret|token|passw(?:or)?d|credential)[A-Za-z0-9_]*[ \t]*[:=][ \t]*)["']?[^\s"',;]{8,}"#,
    )
    .expect("the assignment shape")
});

/// `text` with every key-shaped string replaced by `[redacted]`.
pub(crate) fn clean(text: &str) -> Clean {
    let mut out = text.to_string();
    for shape in WHOLE.iter() {
        out = shape.replace_all(&out, MASK).into_owned();
    }
    out = ASSIGNED
        .replace_all(&out, format!("${{1}}{MASK}").as_str())
        .into_owned();
    Clean(out)
}
