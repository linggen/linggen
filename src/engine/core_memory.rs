//! Core memory — built-in identity + working-style files.
//!
//! Layer 1 of the two-layer memory system (see `doc/memory-spec.md`).
//! Two markdown files under `~/.linggen/core/`:
//!
//! - `identity.md` — who the user is (name, role, timezone, language,
//!   universal preferences).
//! - `style.md` — how the user wants to be assisted (tone, format, pacing,
//!   universal do/don't rules).
//!
//! Both must be **universal**: true in any project, any domain. High bar
//! for entry; the whole pair should stay ~30–50 lines combined.
//!
//! Layer 2 (facts, activity, semantic retrieval) lives in the active memory
//! skill and is reached through the `Memory.*` tool family — not this module.

use std::fs;
use std::io;
use std::path::Path;

const IDENTITY_FILE: &str = "identity.md";
const STYLE_FILE: &str = "style.md";

const IDENTITY_TEMPLATE: &str = r#"---
name: identity
description: Who the user is — universal across every project and context
---

<!--
Keep this small. Only facts that stay true regardless of project or domain.
Delete this comment once you've filled the sections below.
-->

## Name

## Role

## Location & timezone

## Languages

## Core preferences
"#;

const STYLE_TEMPLATE: &str = r#"---
name: style
description: How the user wants the agent to assist — universal working style
---

<!--
Tone, format, pacing, and cross-project do/don't rules.
Project-specific conventions belong in that project's CLAUDE.md / AGENTS.md,
not here.
-->

## Tone

## Format

## Pacing

## Universal rules
"#;

/// Loaded content for the core memory block. Bodies exclude the YAML
/// frontmatter — only the markdown body the user authored.
pub(crate) struct CoreContent {
    pub identity: String,
    pub style: String,
}

/// Read `identity.md` and `style.md` from the core directory. Returns
/// `Some` only when at least one file has substantive user content beyond
/// headings and template comments. Creates the templates on first access
/// so the user always has a file to edit, but treats unedited templates
/// as absent (callers can then fall back to legacy inlining).
pub(crate) fn load_core() -> Option<CoreContent> {
    let dir = crate::paths::core_dir();
    let _ = ensure_templates(&dir);

    let identity = read_body(&dir.join(IDENTITY_FILE)).unwrap_or_default();
    let style = read_body(&dir.join(STYLE_FILE)).unwrap_or_default();

    if !has_user_content(&identity) && !has_user_content(&style) {
        return None;
    }

    Some(CoreContent { identity, style })
}

/// Ensure the core directory and both template files exist. Idempotent:
/// never overwrites user content.
pub(crate) fn ensure_templates(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    write_if_missing(&dir.join(IDENTITY_FILE), IDENTITY_TEMPLATE)?;
    write_if_missing(&dir.join(STYLE_FILE), STYLE_TEMPLATE)?;
    Ok(())
}

fn write_if_missing(path: &Path, body: &str) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, body)
}

fn read_body(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    Some(strip_frontmatter(&text).trim().to_string())
}

fn strip_frontmatter(text: &str) -> &str {
    if !text.starts_with("---") {
        return text;
    }
    let parts: Vec<&str> = text.splitn(3, "---").collect();
    if parts.len() < 3 {
        return text;
    }
    parts[2]
}

/// Treat a body as "populated" only when it has at least one line that
/// isn't a heading, blank, or HTML template comment. Stops unedited
/// scaffolding from suppressing the legacy fallback.
fn has_user_content(body: &str) -> bool {
    let mut in_comment = false;
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if in_comment {
            if line.contains("-->") {
                in_comment = false;
            }
            continue;
        }
        if line.starts_with("<!--") {
            if !line.contains("-->") {
                in_comment = true;
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_body_is_not_user_content() {
        let body = strip_frontmatter(IDENTITY_TEMPLATE);
        assert!(!has_user_content(body));
    }

    #[test]
    fn populated_body_is_user_content() {
        let body = "## Name\nLiang\n\n## Role\nFounder";
        assert!(has_user_content(body));
    }

    #[test]
    fn multiline_comment_does_not_count() {
        let body = "<!--\nHelpful\nguidance\n-->\n\n## Name";
        assert!(!has_user_content(body));
    }

    #[test]
    fn ensure_templates_creates_files() {
        let tmp = std::env::temp_dir().join("linggen_core_memory_test");
        let _ = fs::remove_dir_all(&tmp);
        ensure_templates(&tmp).unwrap();
        assert!(tmp.join(IDENTITY_FILE).exists());
        assert!(tmp.join(STYLE_FILE).exists());

        fs::write(tmp.join(IDENTITY_FILE), "overwritten").unwrap();
        ensure_templates(&tmp).unwrap();
        let after = fs::read_to_string(tmp.join(IDENTITY_FILE)).unwrap();
        assert_eq!(after, "overwritten", "ensure_templates must not overwrite");

        let _ = fs::remove_dir_all(&tmp);
    }
}
