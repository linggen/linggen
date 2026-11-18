//! The lock: a ChatGPT model id (`gpt-` and a digit) or a Codex client
//! version (`0.NNN.N`) written as a string literal lives in this module and
//! its seed only. Everything else asks the catalog, so a new generation is a
//! change to the snapshot, not to the code. Comments and test code are free
//! to name ids.

use std::path::{Path, PathBuf};

/// What a literal must not say outside the catalog.
fn offence(literal: &str) -> Option<&'static str> {
    if names_gpt_id(literal) {
        return Some("a ChatGPT model id");
    }
    if names_codex_version(literal) {
        return Some("a Codex client version");
    }
    None
}

/// `gpt-` followed by a digit, anywhere in the literal.
fn names_gpt_id(s: &str) -> bool {
    s.match_indices("gpt-").any(|(at, _)| {
        s[at + 4..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    })
}

/// Three dotted numbers with a zero major and a minor of three digits — the
/// shape of a Codex client version.
fn names_codex_version(s: &str) -> bool {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .any(|tok| {
            let parts: Vec<&str> = tok.split('.').collect();
            parts.len() == 3
                && parts[0] == "0"
                && parts[1].len() == 3
                && parts.iter().all(|p| !p.is_empty())
        })
}

/// The string literals on one line of code, comments left out.
fn literals(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match current.as_mut() {
            Some(buf) if c == '\\' => {
                buf.push(c);
                if let Some(n) = chars.next() {
                    buf.push(n);
                }
            }
            Some(_) if c == '"' => out.push(current.take().unwrap_or_default()),
            Some(buf) => buf.push(c),
            None if c == '/' && chars.peek() == Some(&'/') => break,
            None if c == '"' => current = Some(String::new()),
            None => {}
        }
    }
    out
}

/// A test-only file by its name or place.
fn is_test_file(rel: &str) -> bool {
    rel.contains("/tests/") || rel.ends_with("/tests.rs") || rel.ends_with("_tests.rs")
}

/// The lines of [text] outside `#[cfg(test)] mod … { … }` blocks.
fn non_test_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == "#[cfg(test)]" && opens_test_mod(&lines, i + 1) {
            i = end_of_block(&lines, i + 1);
            continue;
        }
        out.push((i + 1, lines[i]));
        i += 1;
    }
    out
}

fn opens_test_mod(lines: &[&str], at: usize) -> bool {
    lines.get(at).is_some_and(|l| {
        let t = l.trim_start();
        t.starts_with("mod ") || t.starts_with("pub mod ") || t.starts_with("pub(crate) mod ")
    })
}

/// The index after the block starting at [start] (a `mod x;` is one line).
fn end_of_block(lines: &[&str], start: usize) -> usize {
    if lines[start].trim_end().ends_with(';') {
        return start + 1;
    }
    let mut depth = 0i32;
    for (i, line) in lines.iter().enumerate().skip(start) {
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if depth <= 0 && line.contains('}') {
            return i + 1;
        }
    }
    lines.len()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every offending literal in the engine's non-test source outside the
/// catalog, as `file:line: what — literal`.
fn offences() -> Vec<String> {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize().expect("crate dir");
    let repo = crate_dir.join("../..").canonicalize().expect("repo root");
    let mut files = Vec::new();
    rust_files(&crate_dir.join("src"), &mut files);
    rust_files(&repo.join("src"), &mut files);
    let mut found = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&repo)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.contains("provider/models/chatgpt_catalog/") || is_test_file(&rel) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in non_test_lines(&text) {
            for lit in literals(line) {
                if let Some(what) = offence(&lit) {
                    found.push(format!("{rel}:{n}: {what} — \"{lit}\""));
                }
            }
        }
    }
    found.sort();
    found
}

#[test]
fn chatgpt_ids_and_the_client_version_are_named_only_in_the_catalog() {
    let found = offences();
    assert!(
        found.is_empty(),
        "ask provider::models::chatgpt_catalog instead of naming these:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_scan_sees_what_it_is_meant_to() {
    assert!(names_gpt_id("gpt-6.2-sol"));
    assert!(names_gpt_id("openai/gpt-7"));
    assert!(!names_gpt_id("gpt-"));
    assert!(!names_gpt_id("chatgpt"));
    assert!(names_codex_version("0.169.0"));
    assert!(!names_codex_version("2.3.1"));
    assert!(!names_codex_version("0.3.309"));
    assert_eq!(literals(r#"let a = "x"; // "gpt-6""#), vec!["x".to_string()]);
    assert_eq!(literals(r#"f("a\"b", "c")"#), vec![r#"a\"b"#.to_string(), "c".to_string()]);
    let text = "a\n#[cfg(test)]\nmod tests {\n    \"gpt-6\"\n}\nb\n";
    let kept: Vec<&str> = non_test_lines(text).into_iter().map(|(_, l)| l).collect();
    assert_eq!(kept, vec!["a", "b"]);
}
