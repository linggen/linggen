//! Heredoc bodies: text a Bash command carries, not paths it touches
//! (pattern operands are in [`super::pattern_text`]).
//!
//! Best effort, like the rest of the gate, and failing closed: text is
//! dropped only on a positive match of a shape understood here. Anything
//! else — an unknown reader, a pipe, grouping, a comment, a flag form not
//! listed — keeps every word, and the command is judged as before.

use super::classify::dequote;
use super::model::{command_tokens, Joint, Token};

/// The command with its heredoc bodies removed, along with their `<<`
/// operator and delimiter, when every heredoc in it is inert data (see
/// [`is_inert`]). Otherwise the command unchanged: one body that might
/// be run, expanded or read as paths keeps them all.
pub(super) fn strip_heredoc_bodies(cmd: &str) -> String {
    if !cmd.contains("<<") {
        return cmd.to_string();
    }
    stripped(cmd).unwrap_or_else(|| cmd.to_string())
}

fn stripped(cmd: &str) -> Option<String> {
    let mut out = String::with_capacity(cmd.len());
    let mut quote = None;
    let mut lines = cmd.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let (kept, docs) = split_heredocs(line, &mut quote);
        if docs.is_empty() {
            out.push_str(line);
            continue;
        }
        if quote.is_some() {
            return None;
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

/// A body is data when its line is plain (no pipe, grouping, comment,
/// substitution or continuation — anything that could take the body
/// elsewhere), every command on that line is a [`DATA_READERS`] program,
/// and the shell would not expand anything in the body.
fn is_inert(line: &str, docs: &[Heredoc], bodies: &[String]) -> bool {
    let expands = docs
        .iter()
        .zip(bodies)
        .any(|(d, b)| !d.quoted && b.contains(['$', '`']));
    !expands && is_plain(line) && only_data_readers(line)
}

/// No continuation and, outside quotes, none of `# $ ` ( ) { }`; no `$`
/// or backtick inside double quotes either.
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
            None if "#$`(){}\\".contains(c) => return false,
            None => {}
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

/// The line without its heredoc operators, and the heredocs it opens.
/// `quote` carries an open quote across lines; `<<<` is a here-string.
fn split_heredocs(line: &str, quote: &mut Option<char>) -> (String, Vec<Heredoc>) {
    let mut kept = String::with_capacity(line.len());
    let mut docs = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && *quote != Some('\'') {
            kept.push(c);
            kept.extend(chars.next());
            continue;
        }
        if let Some(q) = *quote {
            kept.push(c);
            if c == q {
                *quote = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            *quote = Some(c);
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
        let mut probe = chars.clone();
        match read_delimiter(&mut probe) {
            Some(doc) => {
                docs.push(doc);
                chars = probe;
            }
            None => kept.push_str("<<"),
        }
    }
    (kept, docs)
}

/// The delimiter word after `<<` (`EOF`, `'EOF'`, `"EOF"`, `\EOF`, `-EOF`).
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
    let delimiter: String = raw.chars().filter(|c| !"'\"\\".contains(*c)).collect();
    (!delimiter.is_empty()).then(|| Heredoc {
        quoted: raw.contains(['\'', '"', '\\']),
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
