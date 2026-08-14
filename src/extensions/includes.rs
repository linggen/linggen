//! `{{#include path.md}}` resolution for extension markdown bodies.
//!
//! mdBook's include syntax: a line whose trimmed content is exactly
//! `{{#include relative/path.md}}` is replaced by that file's content,
//! resolved relative to the directory of the including file. Includes
//! nest; the ancestor chain is cycle-guarded and a missing file is a
//! hard error so a broken spec fails loudly instead of silently
//! dropping its shared sections.

use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 8;

/// Replace every `{{#include ...}}` line in `content` with the named
/// file's content, resolved relative to `base_dir`.
pub fn resolve_md_includes(content: &str, base_dir: &Path) -> Result<String> {
    let mut ancestors = HashSet::new();
    resolve_inner(content, base_dir, &mut ancestors, 0)
}

/// Extract the path from a line that is exactly an include directive.
fn parse_include_line(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix("{{#include")?;
    let rest = rest.strip_suffix("}}")?;
    let path = rest.trim();
    if path.is_empty() {
        return None;
    }
    Some(path)
}

fn resolve_inner(
    content: &str,
    base_dir: &Path,
    ancestors: &mut HashSet<PathBuf>,
    depth: usize,
) -> Result<String> {
    if depth > MAX_DEPTH {
        bail!("include nesting exceeds {MAX_DEPTH} levels");
    }

    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        let Some(rel) = parse_include_line(line) else {
            out.push_str(line);
            out.push('\n');
            continue;
        };

        let full = base_dir.join(rel);
        let canon = full
            .canonicalize()
            .with_context(|| format!("include not found: {}", full.display()))?;
        if !ancestors.insert(canon.clone()) {
            bail!("include cycle through {}", canon.display());
        }

        let included = fs::read_to_string(&canon)
            .with_context(|| format!("cannot read include {}", canon.display()))?;
        let included_dir = canon.parent().unwrap_or(base_dir).to_path_buf();
        let resolved = resolve_inner(&included, &included_dir, ancestors, depth + 1)?;
        out.push_str(resolved.trim_end());
        out.push('\n');

        ancestors.remove(&canon);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!("linggen-{prefix}-{}-{ts}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn no_includes_passes_through() {
        let dir = temp_dir("inc-plain");
        let out = resolve_md_includes("hello\nworld", &dir).unwrap();
        assert_eq!(out, "hello\nworld\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_simple_include() {
        let dir = temp_dir("inc-simple");
        fs::create_dir_all(dir.join("shared")).unwrap();
        fs::write(dir.join("shared/voice.md"), "## Voice\nbe plain\n").unwrap();
        let out = resolve_md_includes("intro\n{{#include shared/voice.md}}\noutro", &dir).unwrap();
        assert_eq!(out, "intro\n## Voice\nbe plain\noutro\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_nested_include_relative_to_included_file() {
        let dir = temp_dir("inc-nested");
        fs::create_dir_all(dir.join("shared")).unwrap();
        fs::write(dir.join("shared/outer.md"), "{{#include inner.md}}\n").unwrap();
        fs::write(dir.join("shared/inner.md"), "deep\n").unwrap();
        let out = resolve_md_includes("{{#include shared/outer.md}}", &dir).unwrap();
        assert_eq!(out, "deep\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_include_is_an_error() {
        let dir = temp_dir("inc-missing");
        let err = resolve_md_includes("{{#include nope.md}}", &dir).unwrap_err();
        assert!(err.to_string().contains("include not found"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cycle_is_an_error() {
        let dir = temp_dir("inc-cycle");
        fs::write(dir.join("a.md"), "{{#include b.md}}\n").unwrap();
        fs::write(dir.join("b.md"), "{{#include a.md}}\n").unwrap();
        let err = resolve_md_includes("{{#include a.md}}", &dir).unwrap_err();
        assert!(err.to_string().contains("include cycle"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_file_twice_sequentially_is_allowed() {
        let dir = temp_dir("inc-twice");
        fs::write(dir.join("v.md"), "once\n").unwrap();
        let out =
            resolve_md_includes("{{#include v.md}}\n{{#include v.md}}", &dir).unwrap();
        assert_eq!(out, "once\nonce\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_directive_braces_pass_through() {
        let dir = temp_dir("inc-braces");
        let src = "use {{name}} here\nprefix {{#include x.md}} suffix";
        let out = resolve_md_includes(src, &dir).unwrap();
        assert_eq!(out, format!("{src}\n"));
        let _ = fs::remove_dir_all(&dir);
    }
}
