//! sed and awk programs given on the command line: do they run commands,
//! or reach a file they name themselves?
//!
//! Both are edit-tier programs; a script that runs a shell command (sed's
//! `e`, awk's `system()` or a pipe) is admin. So is one that reads or
//! writes a file named inside it (sed's `r`/`w`, awk's `print > file` or
//! `getline < file`): the script is text, so that file is no target of the
//! command. A script that cannot be seen (`-f file`) or read is admin too.

use super::model::BashClass::{self, Admin, Read};

pub(super) fn sed_class(args: &[String]) -> BashClass {
    match sed_scripts(args) {
        Some(scripts) if !scripts.iter().any(|s| sed_script_runs(s)) => Read,
        _ => Admin,
    }
}

pub(super) fn awk_class(args: &[String]) -> BashClass {
    match awk_programs(args) {
        Some(programs) if !programs.iter().any(|p| awk_program_runs(p)) => Read,
        _ => Admin,
    }
}

/// `--exp` names `--expression`, as GNU getopt accepts.
fn long_is(name: &str, full: &str) -> bool {
    !name.is_empty() && full.starts_with(name)
}

/// A long option's value: glued with `=` or the next word.
fn long_value<'a>(
    value: Option<&'a str>,
    rest: &mut impl Iterator<Item = &'a String>,
) -> Option<&'a str> {
    value.or_else(|| rest.next().map(String::as_str))
}

// ---------------------------------------------------------------------------
// sed
// ---------------------------------------------------------------------------

/// sed's scripts, or None when one is unseen (`-f`) or an option is unknown.
fn sed_scripts(args: &[String]) -> Option<Vec<&str>> {
    let mut scripts = Vec::new();
    let mut operands = Vec::new();
    let mut options_done = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if options_done || a == "-" || !a.starts_with('-') {
            operands.push(a.as_str());
        } else if a == "--" {
            options_done = true;
        } else if let Some(long) = a.strip_prefix("--") {
            let (name, value) = split_long(long);
            if long_is(name, "expression") {
                scripts.push(long_value(value, &mut it)?);
            } else if long_is(name, "file") {
                return None;
            } else if long_is(name, "line-length") && value.is_none() {
                it.next();
            }
        } else {
            sed_short(&a[1..], &mut it, &mut scripts)?;
        }
    }
    if scripts.is_empty() {
        // BSD `sed -i ''` leaves an empty suffix word before the script.
        scripts.extend(operands.into_iter().find(|o| !o.is_empty()));
    }
    Some(scripts)
}

fn sed_short<'a>(
    cluster: &'a str,
    rest: &mut impl Iterator<Item = &'a String>,
    scripts: &mut Vec<&'a str>,
) -> Option<()> {
    for (i, c) in cluster.char_indices() {
        let glued = &cluster[i + 1..];
        match c {
            'e' => {
                scripts.push(if glued.is_empty() {
                    rest.next()?.as_str()
                } else {
                    glued
                });
                return Some(());
            }
            'l' => {
                if glued.is_empty() {
                    rest.next();
                }
                return Some(());
            }
            // `-i` takes an optional glued suffix.
            'i' => return Some(()),
            'n' | 's' | 'E' | 'r' | 'z' | 'u' => {}
            _ => return None,
        }
    }
    Some(())
}

fn split_long(long: &str) -> (&str, Option<&str>) {
    match long.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (long, None),
    }
}

/// True when the script has an `e` command or `s///e`, names a file
/// (`r`, `w`, `s///w`), or cannot be read.
fn sed_script_runs(script: &str) -> bool {
    let c: Vec<char> = script.chars().collect();
    let mut i = 0;
    loop {
        i = skip_while(&c, i, |ch| {
            ch.is_whitespace() || matches!(ch, ';' | '{' | '}')
        });
        if i >= c.len() {
            return false;
        }
        let Some(next) = skip_addresses(&c, i) else {
            return true;
        };
        i = skip_while(&c, next, |ch| matches!(ch, '!' | ' ' | '\t'));
        if i >= c.len() {
            return false;
        }
        match sed_command(&c, i) {
            Some(next) => i = next,
            None => return true,
        }
    }
}

fn skip_while(c: &[char], mut i: usize, keep: impl Fn(char) -> bool) -> usize {
    while i < c.len() && keep(c[i]) {
        i += 1;
    }
    i
}

fn skip_addresses(c: &[char], i: usize) -> Option<usize> {
    let i = skip_address(c, i)?;
    if c.get(i) == Some(&',') {
        return skip_address(c, i + 1);
    }
    Some(i)
}

fn skip_address(c: &[char], i: usize) -> Option<usize> {
    let digits = |j| skip_while(c, j, |ch| ch.is_ascii_digit());
    let regex_flags = |j| skip_while(c, j, |ch| matches!(ch, 'I' | 'M'));
    match c.get(i) {
        Some(d) if d.is_ascii_digit() => {
            let j = digits(i);
            Some(if c.get(j) == Some(&'~') {
                digits(j + 1)
            } else {
                j
            })
        }
        Some('$') => Some(i + 1),
        Some('+' | '~') => Some(digits(i + 1)),
        Some('/') => Some(regex_flags(skip_delimited(c, i + 1, '/')?)),
        Some('\\') => {
            let d = *c.get(i + 1)?;
            Some(regex_flags(skip_delimited(c, i + 2, d)?))
        }
        _ => Some(i),
    }
}

/// Index just past the closing delimiter.
fn skip_delimited(c: &[char], mut i: usize, delim: char) -> Option<usize> {
    while let Some(&ch) = c.get(i) {
        if ch == '\\' {
            i += 2;
        } else if ch == delim {
            return Some(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// Index past the end of the line; a backslash continues the line.
fn skip_line(c: &[char], mut i: usize) -> usize {
    while let Some(&ch) = c.get(i) {
        i += if ch == '\\' { 2 } else { 1 };
        if ch == '\n' {
            break;
        }
    }
    i.min(c.len())
}

/// Index past one command, or None when it runs something, names a file
/// or is unknown.
fn sed_command(c: &[char], i: usize) -> Option<usize> {
    let j = i + 1;
    match c[i] {
        '{' | '}' | '=' | 'd' | 'D' | 'g' | 'G' | 'h' | 'H' | 'n' | 'N' | 'p' | 'P' | 'x' | 'z'
        | 'F' => Some(j),
        'l' | 'L' | 'q' | 'Q' => Some(skip_while(c, j, |ch| ch.is_ascii_digit() || ch == ' ')),
        'a' | 'i' | 'c' | '#' => Some(skip_line(c, j)),
        'r' | 'R' | 'w' | 'W' => None,
        ':' | 'b' | 't' | 'T' | 'v' => Some(skip_while(c, j, |ch| ch != ';' && ch != '\n')),
        's' => {
            let d = *c.get(j)?;
            if d == '\n' || d == '\\' {
                return None;
            }
            let k = skip_delimited(c, skip_delimited(c, j + 1, d)?, d)?;
            sed_s_flags(c, k)
        }
        'y' => {
            let d = *c.get(j)?;
            skip_delimited(c, skip_delimited(c, j + 1, d)?, d)
        }
        _ => None,
    }
}

fn sed_s_flags(c: &[char], mut k: usize) -> Option<usize> {
    loop {
        match c.get(k) {
            None => return Some(k),
            Some(';' | '\n' | '}' | ' ' | '\t') => return Some(k),
            Some('w') => return None,
            Some(ch) if ch.is_ascii_digit() || matches!(ch, 'g' | 'p' | 'i' | 'I' | 'm' | 'M') => {
                k += 1
            }
            Some(_) => return None,
        }
    }
}

// ---------------------------------------------------------------------------
// awk
// ---------------------------------------------------------------------------

/// awk's program text, or None when it is unseen (`-f`) or an option is unknown.
fn awk_programs(args: &[String]) -> Option<Vec<&str>> {
    let mut programs = Vec::new();
    let mut options_done = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if options_done || a == "-" || !a.starts_with('-') {
            // The first operand is the program unless `-e` gave one; the
            // rest are files and assignments.
            if programs.is_empty() {
                programs.push(a.as_str());
            }
            break;
        } else if a == "--" {
            options_done = true;
        } else if let Some(long) = a.strip_prefix("--") {
            let (name, value) = split_long(long);
            if long_is(name, "source") {
                programs.push(long_value(value, &mut it)?);
            } else if long_is(name, "assign") || long_is(name, "field-separator") {
                long_value(value, &mut it)?;
            } else {
                return None;
            }
        } else {
            let mut cluster = a[1..].chars();
            let flag = cluster.next()?;
            let glued = cluster.as_str();
            let value = if glued.is_empty() {
                it.next()?.as_str()
            } else {
                glued
            };
            match flag {
                'e' => programs.push(value),
                'F' | 'v' => {}
                _ => return None,
            }
        }
    }
    Some(programs)
}

/// True when the program can run a command (`system()`, a pipe, `@load`)
/// or names a file (`print > file`, `getline < file`).
fn awk_program_runs(program: &str) -> bool {
    let code = awk_strip_literals(program);
    let c: Vec<char> = code.chars().collect();
    if code.contains("@load") || awk_names_a_file(&c) {
        return true;
    }
    for (i, &ch) in c.iter().enumerate() {
        if ch == '|' && c.get(i + 1) != Some(&'|') && (i == 0 || c[i - 1] != '|') {
            return true;
        }
    }
    let mut rest = code.as_str();
    while let Some(at) = rest.find("system") {
        let before = rest[..at].chars().next_back();
        let after = rest[at + 6..].trim_start();
        let word_start = !before.is_some_and(|b| b.is_alphanumeric() || b == '_');
        if word_start && after.starts_with('(') {
            return true;
        }
        rest = &rest[at + 6..];
    }
    false
}

/// `print … > file` (or `>>`) and `getline < file`. A `>` in a print
/// statement outside parentheses is always a redirection in awk.
fn awk_names_a_file(c: &[char]) -> bool {
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let word_at = |i: usize, word: &str| {
        let w: Vec<char> = word.chars().collect();
        c[i..].starts_with(&w)
            && !i.checked_sub(1).is_some_and(|p| is_word(c[p]))
            && !c.get(i + w.len()).is_some_and(|&n| is_word(n))
    };
    (0..c.len()).any(|i| {
        (word_at(i, "print") || word_at(i, "printf")) && statement_has(c, i, '>')
            || word_at(i, "getline") && statement_has(c, i, '<')
    })
}

/// Whether `op` stands in the statement starting at `from`, at its own
/// depth: not inside parentheses or brackets opened after `from`.
fn statement_has(c: &[char], from: usize, op: char) -> bool {
    let mut depth = 0usize;
    for &ch in &c[from..] {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' if depth == 0 => return false,
            ')' | ']' => depth -= 1,
            ';' | '}' | '\n' if depth == 0 => return false,
            _ if ch == op && depth == 0 => return true,
            _ => {}
        }
    }
    false
}

/// Code with string and regex literals blanked, so a `|` inside `"a|b"` or
/// `/a|b/` is not read as a pipe.
fn awk_strip_literals(program: &str) -> String {
    let mut out = String::with_capacity(program.len());
    let mut chars = program.chars();
    let mut prev_code: Option<char> = None;
    while let Some(c) = chars.next() {
        let regex_start = c == '/' && prev_code.is_none_or(|p| "(,~!{};&|?:=\n".contains(p));
        if c == '"' || regex_start {
            skip_literal(&mut chars, c);
            out.push_str("\"\"");
            prev_code = Some('"');
            continue;
        }
        out.push(c);
        if !c.is_whitespace() || c == '\n' {
            prev_code = Some(c);
        }
    }
    out
}

fn skip_literal(chars: &mut std::str::Chars<'_>, close: char) {
    while let Some(c) = chars.next() {
        if c == '\\' {
            chars.next();
        } else if c == close {
            return;
        }
    }
}
