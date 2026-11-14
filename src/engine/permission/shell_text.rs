//! Text a Bash command carries that is not a path it touches: heredoc
//! bodies, and the pattern or program operand of grep, sed, awk and jq.
//!
//! Best effort, like the rest of the gate. A body the shell would expand
//! (`$`, backticks under an unquoted delimiter) or run as a script (a shell
//! on the same line) is kept, so it is judged as before.

use super::classify::dequote;

/// The command with inert heredoc bodies removed, along with their `<<`
/// operator and delimiter. The rest of the line — a real redirect target,
/// a file argument — stays.
pub(super) fn strip_heredoc_bodies(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut quote = None;
    let mut lines = cmd.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let (kept, docs) = split_heredocs(line, &mut quote);
        if docs.is_empty() || quote.is_some() {
            out.push_str(line);
            continue;
        }
        let bodies: Vec<String> = docs.iter().map(|d| take_body(&mut lines, d)).collect();
        if is_inert(&kept, &docs, &bodies) {
            out.push_str(&kept);
        } else {
            out.push_str(line);
            bodies.iter().for_each(|b| out.push_str(b));
        }
    }
    out
}

struct Heredoc {
    delimiter: String,
    /// `<<-`: the terminator may be indented by tabs.
    dash: bool,
    /// A quoted delimiter turns off expansion in the body.
    quoted: bool,
}

const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash", "ksh", "fish"];

/// A body is data when no shell on its line runs it as a script and the
/// shell would not expand anything in it.
fn is_inert(line: &str, docs: &[Heredoc], bodies: &[String]) -> bool {
    let runs_script = super::model::shell_words(line).iter().any(|w| {
        let word = dequote(w);
        SHELLS.contains(&word.rsplit('/').next().unwrap_or(""))
    });
    let expands = docs
        .iter()
        .zip(bodies)
        .any(|(d, b)| !d.quoted && b.contains(['$', '`']));
    !runs_script && !expands
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

/// The body lines and the terminator line, consumed from `lines`.
fn take_body<'a>(lines: &mut impl Iterator<Item = &'a str>, doc: &Heredoc) -> String {
    let mut body = String::new();
    for line in lines.by_ref() {
        body.push_str(line);
        let text = line.trim_end_matches(['\n', '\r']);
        let text = if doc.dash {
            text.trim_start_matches('\t')
        } else {
            text
        };
        if text == doc.delimiter {
            break;
        }
    }
    body
}

/// Which operand of a command is pattern or program text. `grep -E '/a/b'
/// src` searches for `/a/b`; it does not read a file there.
#[derive(Default)]
pub(super) struct PatternSlot {
    state: Slot,
    /// Flags of this program that take the next word as their value.
    value_flags: &'static [&'static str],
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum Slot {
    /// Not a pattern program: nothing is text.
    #[default]
    None,
    /// The next operand is the pattern.
    Operand,
    /// The pattern is known; the rest are files, except after `-e`.
    Files,
    /// The next word is a pattern (after `-e`).
    Explicit,
    /// The next word is a flag's value.
    FlagValue,
    /// The next word is a redirect target.
    Redirect,
}

impl PatternSlot {
    pub(super) fn for_program(program: &str) -> Self {
        let value_flags: &'static [&'static str] = match program {
            "grep" | "egrep" | "fgrep" => &["-A", "-B", "-C", "-m", "-d", "-D"],
            "rg" => &[
                "-A", "-B", "-C", "-m", "-g", "-t", "-T", "-j", "-M", "--glob", "--type",
            ],
            "awk" | "gawk" | "mawk" => &["-F", "-v"],
            "sed" | "jq" => &[],
            _ => return Self::default(),
        };
        Self {
            state: Slot::Operand,
            value_flags,
        }
    }

    /// Whether `word` (as written) is pattern text rather than a path.
    /// Called once per word after the program name, in order.
    pub(super) fn is_text(&mut self, word: &str) -> bool {
        let w = dequote(word);
        match self.state {
            Slot::None => false,
            Slot::Explicit => {
                self.state = Slot::Files;
                true
            }
            Slot::FlagValue => {
                self.state = Slot::Operand;
                true
            }
            Slot::Redirect => {
                self.state = Slot::Operand;
                false
            }
            Slot::Files => {
                if is_pattern_flag(&w) {
                    self.state = Slot::Explicit;
                }
                false
            }
            Slot::Operand => self.operand(word, &w),
        }
    }

    fn operand(&mut self, raw: &str, w: &str) -> bool {
        if is_pattern_flag(w) {
            self.state = Slot::Explicit;
            return false;
        }
        if matches!(w, "-f" | "--file") || w.starts_with("--regexp=") || w.starts_with("--file=") {
            self.state = Slot::Files;
            return false;
        }
        if self.value_flags.contains(&w) {
            self.state = Slot::FlagValue;
            return false;
        }
        if matches!(raw, ">" | ">>" | "<" | "1>" | "2>" | "&>") {
            self.state = Slot::Redirect;
            return false;
        }
        // A flag, a glued redirect (`>out`), or an empty word (`sed -i ''`)
        // is not the pattern.
        if w.starts_with('-') || raw.starts_with(['>', '<']) || w.is_empty() {
            return false;
        }
        self.state = Slot::Files;
        true
    }
}

fn is_pattern_flag(w: &str) -> bool {
    matches!(w, "-e" | "--regexp" | "--expression")
}
