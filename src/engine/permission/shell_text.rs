//! Heredoc bodies: text a Bash command carries, not paths it touches
//! (pattern operands are in [`super::pattern_text`]).
//!
//! Best effort, like the rest of the gate, and failing closed: text is
//! dropped only when the whole command is of a shape read here exactly as
//! the shell reads it. Anything else — an unknown reader, a pipe, grouping,
//! a comment, a substitution, a backslash, a compound command — keeps every
//! word, and the command is judged as before.

use super::classify::dequote;
use super::model::{command_tokens, is_odd_space, Joint, Token};

/// The command with its heredoc bodies removed, along with their `<<`
/// operator and delimiter, when every heredoc in it is inert data (see
/// [`is_inert`]) and every other line is plain. Otherwise the command
/// unchanged: one body that might be run, expanded or read as paths keeps
/// them all.
pub(super) fn strip_heredoc_bodies(cmd: &str) -> String {
    if !cmd.contains("<<") {
        return cmd.to_string();
    }
    stripped(cmd).unwrap_or_else(|| cmd.to_string())
}

/// Every line outside a body must be plain and ordinary, heredoc or not:
/// a body is found by reading the lines around it as the shell does, and
/// one construct read differently (a comment's apostrophe, `$'…'`, an
/// escaped delimiter) moves where a body starts or ends — commands the
/// shell runs would be dropped as text.
fn stripped(cmd: &str) -> Option<String> {
    let mut out = String::with_capacity(cmd.len());
    let mut lines = cmd.split_inclusive('\n');
    while let Some(line) = lines.next() {
        if !is_plain(line) || !is_ordinary(line) {
            return None;
        }
        let (kept, docs) = split_heredocs(line)?;
        if docs.is_empty() {
            out.push_str(line);
            continue;
        }
        let bodies = docs
            .iter()
            .map(|d| take_body(&mut lines, d))
            .collect::<Option<Vec<_>>>()?;
        if !is_inert(&kept, &docs, &bodies) {
            return None;
        }
        out.push_str(&kept);
    }
    Some(out)
}

struct Heredoc {
    delimiter: String,
    /// `<<-`: the terminator may be indented by tabs.
    dash: bool,
    /// A quoted delimiter turns off expansion in the body.
    quoted: bool,
}

/// Programs whose standard input is only data: it goes to their output
/// or to the files they name, and those stay targets.
const DATA_READERS: &[&str] = &["cat", "tee"];

/// Words that open, close or continue a compound command, whose lines
/// share one output (`if …; then` / `cat <<EOF` / `fi | sh`).
const COMPOUND_WORDS: &[&str] = &[
    "!", "[[", "case", "coproc", "do", "done", "elif", "else", "esac", "fi", "for", "function",
    "if", "in", "select", "then", "time", "until", "while",
];

/// Builtins that change what a later word means or where output goes.
const SHELL_STATE_WORDS: &[&str] = &[
    ".", "alias", "builtin", "command", "declare", "enable", "eval", "exec", "export", "hash",
    "local", "readonly", "set", "setopt", "shopt", "source", "trap", "typeset", "unalias", "unset",
];

/// A body is data when every command on its line is a [`DATA_READERS`]
/// program and the shell would not expand anything in the body. (That the
/// line is plain — no pipe into a group, comment, substitution or
/// continuation that could take the body elsewhere — holds for every line.)
fn is_inert(line: &str, docs: &[Heredoc], bodies: &[String]) -> bool {
    let expands = docs
        .iter()
        .zip(bodies)
        .any(|(d, b)| !d.quoted && b.contains(['$', '`']));
    !expands && only_data_readers(line)
}

/// No continuation and, outside quotes, none of `# $ ` ( ) { } \` and no
/// whitespace but space and tab; no `$`, backtick or backslash inside
/// double quotes either. Such a line's quotes and words are read here
/// exactly as the shell reads them.
fn is_plain(line: &str) -> bool {
    if line.trim_end_matches(['\n', '\r']).ends_with('\\') {
        return false;
    }
    let mut quote = None;
    for c in line.chars() {
        match quote {
            Some('\'') if c == '\'' => quote = None,
            Some('\'') => {}
            Some(_) if c == '"' => quote = None,
            Some(_) if matches!(c, '$' | '`' | '\\') => return false,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if "#$`(){}\\".contains(c) || is_odd_space(c) => return false,
            None => {}
        }
    }
    true
}

/// No command on the line is part of a compound command, an assignment, or
/// a builtin that changes the shell: each is a simple command of its own.
fn is_ordinary(line: &str) -> bool {
    let mut at_command = true;
    for token in command_tokens(line) {
        match token {
            Token::Op(_) => at_command = true,
            Token::Word(w) if at_command => {
                let odd = w.contains('=')
                    || COMPOUND_WORDS.contains(&w.as_str())
                    || SHELL_STATE_WORDS.contains(&dequote(&w).as_str());
                if odd {
                    return false;
                }
                at_command = false;
            }
            Token::Word(_) => {}
        }
    }
    true
}

/// Each command on the line is `cat` or `tee`, joined only by `;` or `&&`.
fn only_data_readers(line: &str) -> bool {
    let mut at_command = true;
    for token in command_tokens(line) {
        match token {
            Token::Op(Joint::Break) => return false,
            Token::Op(_) => at_command = true,
            Token::Word(w) if at_command => {
                if !DATA_READERS.contains(&dequote(&w).as_str()) {
                    return false;
                }
                at_command = false;
            }
            Token::Word(_) => {}
        }
    }
    true
}

/// The (plain) line without its heredoc operators, and the heredocs it
/// opens; `<<<` is a here-string. `None` when a quote stays open at the
/// line's end or a `<<` has no delimiter that can be read: where its body
/// ends is then not known.
fn split_heredocs(line: &str) -> Option<(String, Vec<Heredoc>)> {
    let mut kept = String::with_capacity(line.len());
    let mut docs = Vec::new();
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            kept.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            kept.push(c);
            continue;
        }
        if c != '<' || chars.peek() != Some(&'<') {
            kept.push(c);
            continue;
        }
        chars.next();
        if chars.next_if_eq(&'<').is_some() {
            kept.push_str("<<<");
            continue;
        }
        docs.push(read_delimiter(&mut chars)?);
    }
    quote.is_none().then_some((kept, docs))
}

/// The delimiter word after `<<` (`EOF`, `'EOF'`, `"EOF"`, `-EOF`); `None`
/// for an empty one (`<<''` ends at the first empty line) or one whose
/// quote does not close.
fn read_delimiter(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<Heredoc> {
    let dash = chars.next_if_eq(&'-').is_some();
    while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
    let mut raw = String::new();
    let mut quote = None;
    while let Some(&c) = chars.peek() {
        let ends = quote.is_none() && (c.is_whitespace() || ";|&<>()".contains(c));
        if ends {
            break;
        }
        if c == '\'' || c == '"' {
            quote = if quote == Some(c) {
                None
            } else {
                quote.or(Some(c))
            };
        }
        raw.push(c);
        chars.next();
    }
    // Quote removal as the shell does it: a quote inside the other kind
    // of quotes is part of the delimiter (`<<'E"F'` ends at `E"F`).
    let delimiter = dequote(&raw);
    (quote.is_none() && !delimiter.is_empty()).then(|| Heredoc {
        quoted: raw.contains(['\'', '"']),
        delimiter,
        dash,
    })
}

/// The body lines and the terminator line, consumed from `lines`; `None`
/// when no terminator follows (what the shell then does is not judged here).
fn take_body<'a>(lines: &mut impl Iterator<Item = &'a str>, doc: &Heredoc) -> Option<String> {
    let mut body = String::new();
    for line in lines.by_ref() {
        body.push_str(line);
        let text = line.strip_suffix('\n').unwrap_or(line);
        let text = if doc.dash {
            text.trim_start_matches('\t')
        } else {
            text
        };
        if text == doc.delimiter {
            return Some(body);
        }
    }
    None
}
