//! What the source-scan locks read with (`#[cfg(test)]` only): the engine's
//! code as text, its test items cut out, literals and comments told apart
//! from code. A file it cannot read to the end panics — a lock never
//! silently skips production code.

/// For each char of `text`: true when it is code, false inside a comment,
/// a string, a raw string or a char literal (the quotes included).
fn code_mask(chars: &[char]) -> Vec<bool> {
    let n = chars.len();
    let mut mask = vec![true; n];
    let mut i = 0;
    let at = |k: usize| chars.get(k).copied().unwrap_or('\0');
    while i < n {
        let start = i;
        let c = chars[i];
        if c == '/' && at(i + 1) == '/' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 0;
            loop {
                assert!(i < n, "lock scan: a block comment never closes");
                if chars[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if let Some(end) = raw_string_end(chars, i) {
            i = end;
        } else if c == '"' || (c == 'b' && at(i + 1) == '"') {
            i += if c == 'b' { 2 } else { 1 };
            loop {
                assert!(i < n, "lock scan: a string never closes");
                match chars[i] {
                    '\\' => i += 2,
                    '"' => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
        } else if c == '\'' {
            match char_literal_end(chars, i) {
                Some(end) => i = end,
                None => i += 1, // a lifetime
            }
        } else {
            i += 1;
            continue;
        }
        for m in &mut mask[start..i.min(n)] {
            *m = false;
        }
    }
    mask
}

/// The end of a raw string (`r"…"`, `r#"…"#`, `br"…"`) starting at `i`.
fn raw_string_end(chars: &[char], i: usize) -> Option<usize> {
    let mut k = i;
    if chars.get(k) == Some(&'b') {
        k += 1;
    }
    if chars.get(k) != Some(&'r') {
        return None;
    }
    // Not the tail of a longer word (`bar"`).
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        return None;
    }
    k += 1;
    let mut hashes = 0;
    while chars.get(k) == Some(&'#') {
        hashes += 1;
        k += 1;
    }
    if chars.get(k) != Some(&'"') {
        return None;
    }
    k += 1;
    while k < chars.len() {
        if chars[k] == '"' && (0..hashes).all(|h| chars.get(k + 1 + h) == Some(&'#')) {
            return Some(k + 1 + hashes);
        }
        k += 1;
    }
    panic!("lock scan: a raw string never closes");
}

/// The end of a char literal (`'x'`, `'\n'`, `'{'`) starting at `i`; `None`
/// for a lifetime.
fn char_literal_end(chars: &[char], i: usize) -> Option<usize> {
    match (chars.get(i + 1), chars.get(i + 2)) {
        (Some('\\'), _) => {
            let close = (i + 2..chars.len().min(i + 14)).find(|&k| chars[k] == '\'')?;
            Some(close + 1)
        }
        (Some(c), Some('\'')) if *c != '\'' => Some(i + 3),
        _ => None,
    }
}

/// The code of `text` without its test items: each `#[cfg(test)]` item (a
/// real attribute, not one inside a string or comment) is cut out whole, to
/// its matching closing brace or its `;`. Braces inside literals and
/// comments are not counted. Panics when an item never ends.
pub(crate) fn without_tests(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mask = code_mask(&chars);
    let marker: Vec<char> = "#[cfg(test)]".chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let is_marker = mask[i] && chars[i..].starts_with(&marker);
        if !is_marker {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut depth = 0usize;
        let mut k = i + marker.len();
        loop {
            assert!(k < chars.len(), "lock scan: a test item never ends");
            if mask[k] {
                match chars[k] {
                    '{' => depth += 1,
                    '}' if depth == 1 => break,
                    '}' => depth -= 1,
                    ';' if depth == 0 => break,
                    _ => {}
                }
            }
            k += 1;
        }
        i = k + 1;
    }
    out
}

/// Every `.rs` file under `dir`.
pub(crate) fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Each file under `dirs` (relative to the package root) with its path and
/// its code without test items.
pub(crate) fn production_code(dirs: &[&str]) -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in dirs {
        rust_files(&root.join(dir), &mut files);
    }
    files
        .into_iter()
        .map(|file| {
            let rel = file.strip_prefix(root).unwrap().to_string_lossy().to_string();
            let code = without_tests(&std::fs::read_to_string(&file).unwrap());
            (rel, code)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::without_tests;

    #[test]
    fn braces_in_literals_do_not_end_a_test_item_early() {
        let src = "fn a() {}\n#[cfg(test)]\nmod t { fn x() { let _ = \"}\"; let _ = '}'; let _ = r#\"}\"#; /* } */ } }\nfn b() {}";
        let code = without_tests(src);
        assert!(code.contains("fn a()") && code.contains("fn b()"), "{code}");
        assert!(!code.contains("mod t"), "{code}");
    }

    #[test]
    fn an_attribute_inside_a_string_cuts_nothing() {
        let code = without_tests("const S: &str = \"#[cfg(test)]\";\nfn real() {}");
        assert!(code.contains("fn real()"), "{code}");
    }

    #[test]
    fn a_lifetime_is_not_a_char_literal() {
        let code = without_tests("fn f<'a>(x: &'a str) -> &'a str { x }\n#[cfg(test)]\nmod t {}\nfn g() {}");
        assert!(code.contains("fn g()") && !code.contains("mod t"), "{code}");
    }

    #[test]
    #[should_panic(expected = "never ends")]
    fn a_test_item_that_never_ends_fails_loudly() {
        without_tests("#[cfg(test)]\nmod t { fn x() {");
    }
}
