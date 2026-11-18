//! A `for` loop over words written in the command, read as the shell runs
//! it: `for f in a b; do wc -l "$f"; done` is `wc -l "a"`, then
//! `wc -l "b"`. The loop is replaced by its body once for each word, with
//! `$NAME` and `${NAME}` put in where the shell would put them (not inside
//! single quotes, not after a backslash), and every check then judges those
//! commands — never more gently than the same commands written by hand.
//!
//! Failing closed, like the rest of the gate: only the exact shape below is
//! read; anything else leaves the command as it is (a `$` in it keeps it
//! unreadable).
//!
//! - `for NAME in w1 w2 …; do BODY; done`, with `;` or newlines between
//!   the parts, standing alone or between other commands joined by `;` or
//!   a newline (not after `&&`, `||` or `|`, not before a pipe or a
//!   redirect, not inside another compound command);
//! - NAME a lower-case name (an upper-case one may be read by the shell or
//!   a program, like `PATH` or `IFS`) and not a proxy setting;
//! - every word of the list a plain literal: letters, digits and `._/+,:@%-`
//!   (no quote, `$`, glob, brace or `=`), or the home folder as `~` or
//!   `~/…` — then put in only where it starts a word, unquoted, so the
//!   shell reads the written-out word as the same folder; at most
//!   [`MAX_VALUES`];
//! - a body that names NAME only as `$NAME` or `${NAME}`, with no `=` or
//!   `+` glued after it (it could become an assignment), and holds no
//!   compound command (`for`, `while`, `if`, `case`, `{ }`, …);
//! - nowhere in the command: a subshell or a substitution (`(`, `)`,
//!   backticks, `$(`), a heredoc, a comment, a brace other than `${NAME}`,
//!   a line continuation, or `IFS`.

/// The most words a loop's list may hold.
pub(super) const MAX_VALUES: usize = 64;

/// The command with each `for` loop replaced by its body once per word;
/// `None` when it has no loop, or one that is not of the shape above.
pub(super) fn unrolled(cmd: &str) -> Option<String> {
    rewrite(cmd).ok().flatten()
}

/// `Ok(None)`: no loop. `Err`: a loop, or something around it, not read.
fn rewrite(cmd: &str) -> Result<Option<String>, ()> {
    let items = scan(cmd).ok_or(())?;
    let Some(at) = (0..items.len()).find(|&i| starts_a_loop(&items, i)) else {
        return Ok(None);
    };
    if !plain_around(cmd) || !prefix_is_plain(&items[..at]) {
        return Err(());
    }
    let parsed = parse_loop(cmd, &items[at..]).ok_or(())?;
    let rest = &cmd[parsed.end..];
    let rest = rewrite(rest)?.unwrap_or_else(|| rest.to_string());
    let bodies: Vec<String> = parsed
        .values
        .iter()
        .map(|value| with_value(parsed.body, parsed.name, value))
        .collect::<Option<_>>()
        .ok_or(())?;
    Ok(Some(format!(
        "{}{}{}",
        &cmd[..items[at].start],
        bodies.join("\n"),
        rest
    )))
}

/// A word or a control operator of the command, with where it stands.
#[derive(Clone, Copy)]
struct Item<'a> {
    text: &'a str,
    start: usize,
    end: usize,
    word: bool,
}

/// The command's words and operators, as `command_tokens` splits them;
/// `None` for an unclosed quote.
fn scan<'a>(cmd: &'a str) -> Option<Vec<Item<'a>>> {
    let mut items = Vec::new();
    let mut start = None;
    let mut quote = None;
    let mut chars = cmd.char_indices().peekable();
    let push = |items: &mut Vec<Item<'a>>, start: usize, end: usize, word: bool| {
        items.push(Item {
            text: &cmd[start..end],
            start,
            end,
            word,
        });
    };
    while let Some((i, c)) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' {
                chars.next();
            }
            continue;
        }
        let in_word = start.is_some();
        match c {
            '\\' => {
                start.get_or_insert(i);
                chars.next();
            }
            '\'' | '"' => {
                start.get_or_insert(i);
                quote = Some(c);
            }
            ' ' | '\t' => {
                if let Some(s) = start.take() {
                    push(&mut items, s, i, true);
                }
            }
            '&' | '|' if in_word && cmd[..i].ends_with(['>', '<']) => {}
            '&' if chars.peek().is_some_and(|(_, n)| *n == '>') => {
                start.get_or_insert(i);
            }
            ';' | '\n' | '&' | '|' | '(' | ')' => {
                if let Some(s) = start.take() {
                    push(&mut items, s, i, true);
                }
                let doubled = matches!(c, ';' | '&' | '|')
                    && chars
                        .next_if(|(_, n)| *n == c || (c == '|' && *n == '&'))
                        .is_some();
                let end = i + c.len_utf8() + usize::from(doubled);
                push(&mut items, i, end, false);
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if let Some(s) = start {
        push(&mut items, s, cmd.len(), true);
    }
    Some(items)
}

fn is_word(item: Option<&Item<'_>>, text: &str) -> bool {
    item.is_some_and(|it| it.word && it.text == text)
}

fn is_sep(item: Option<&Item<'_>>, seps: &[&str]) -> bool {
    item.is_some_and(|it| !it.word && seps.contains(&it.text))
}

/// Whether `items[at]` is a `for` where a command starts.
fn starts_a_loop(items: &[Item<'_>], at: usize) -> bool {
    is_word(items.get(at), "for") && (at == 0 || is_sep(items.get(at - 1), &[";", "\n"]))
}

/// Words that open or close a compound command, or change how one reads.
const KEYWORDS: &[&str] = &[
    "for", "while", "until", "do", "done", "select", "case", "esac", "if", "then", "else", "elif",
    "fi", "function", "coproc", "{", "}", "[[", "]]",
];

/// What stands before the loop: plain commands, the last one ended by `;`
/// or a newline (a newline right after `&&`, `||` or `|` continues it).
fn prefix_is_plain(prefix: &[Item<'_>]) -> bool {
    let no_keyword = prefix
        .iter()
        .all(|it| !(it.word && KEYWORDS.contains(&it.text)));
    let before_newlines = prefix.iter().rev().find(|it| it.word || it.text != "\n");
    no_keyword && before_newlines.is_none_or(|it| it.word || it.text == ";")
}

/// What no loop is read in: subshells, substitutions, heredocs, comments,
/// braces but `${NAME}`, line continuations, a changed `IFS`.
fn plain_around(cmd: &str) -> bool {
    if cmd.contains("<<") || cmd.contains("IFS") || cmd.contains("\\\n") {
        return false;
    }
    let mut in_single = false;
    let mut in_double = false;
    let mut rest = cmd;
    while let Some(c) = rest.chars().next() {
        let after = &rest[c.len_utf8()..];
        if in_single {
            in_single = c != '\'';
            rest = after;
            continue;
        }
        match c {
            '\\' => {
                let skip = after.chars().next().map_or(0, char::len_utf8);
                rest = &after[skip..];
                continue;
            }
            '\'' if !in_double => in_single = true,
            '"' => in_double = !in_double,
            '`' | '{' | '}' => return false,
            '#' | '(' | ')' if !in_double => return false,
            '$' => match braced_name(after) {
                Some(tail) => {
                    rest = tail;
                    continue;
                }
                None if after.starts_with(['(', '{']) => return false,
                None => {}
            },
            _ => {}
        }
        rest = after;
    }
    true
}

/// The text after `{NAME}` (or `{?}`) when `after` (past a `$`) starts with one.
fn braced_name(after: &str) -> Option<&str> {
    let inner = after.strip_prefix('{')?;
    let close = inner.find('}')?;
    let name = &inner[..close];
    let plain = name == "?"
        || (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    plain.then(|| &inner[close + 1..])
}

/// A loop read from its items.
struct Loop<'a> {
    name: &'a str,
    values: Vec<&'a str>,
    /// From the body's first word to the end of its last.
    body: &'a str,
    /// Where the text after `done` starts.
    end: usize,
}

/// `for NAME in w1 …; do BODY; done`, from `items[0]` (the `for`).
fn parse_loop<'a>(cmd: &'a str, items: &[Item<'a>]) -> Option<Loop<'a>> {
    let name = items
        .get(1)
        .filter(|it| it.word && is_loop_name(it.text))?
        .text;
    if !is_word(items.get(2), "in") {
        return None;
    }
    let mut i = 3;
    let mut values = Vec::new();
    while let Some(it) = items.get(i).filter(|it| it.word) {
        values.push(it.text);
        i += 1;
    }
    let plain = values.iter().all(|v| is_plain_value(v));
    if values.is_empty() || values.len() > MAX_VALUES || !plain {
        return None;
    }
    i = skip_ends(items, i, true)?;
    if !is_word(items.get(i), "do") {
        return None;
    }
    let body_start = skip_ends(items, i + 1, false)?;
    let done = (body_start..items.len()).find(|&j| is_word(items.get(j), "done"))?;
    let body_end = last_word_before(items, body_start, done)?;
    let after = items.get(done + 1);
    if after.is_some_and(|it| it.word || !matches!(it.text, ";" | "\n")) {
        return None;
    }
    let body = &cmd[items[body_start].start..items[body_end].end];
    let body_items = &items[body_start..=body_end];
    let compound = body_items
        .iter()
        .any(|it| it.word && KEYWORDS.contains(&it.text));
    if compound || !names_only_as_values(body, name) {
        return None;
    }
    Some(Loop {
        name,
        values,
        body,
        end: items[done].end,
    })
}

/// Past the `;` and newlines from `items[i]`: there must be at least one
/// when `required`, and at most one `;`. The index after them.
fn skip_ends(items: &[Item<'_>], mut i: usize, required: bool) -> Option<usize> {
    let from = i;
    let mut semis = 0;
    while is_sep(items.get(i), &[";", "\n"]) {
        semis += usize::from(items[i].text == ";");
        i += 1;
    }
    let enough = !required || i > from;
    (enough && semis <= if required { 1 } else { 0 }).then_some(i)
}

/// The body's last word before `done` (at `done`): the body ends with a
/// command and one `;` or newlines, and holds at least one word.
fn last_word_before(items: &[Item<'_>], start: usize, done: usize) -> Option<usize> {
    let mut j = done;
    let mut semis = 0;
    while j > start && is_sep(items.get(j - 1), &[";", "\n"]) {
        semis += usize::from(items[j - 1].text == ";");
        j -= 1;
    }
    let ended = j < done && semis <= 1;
    (ended && j > start && items[j - 1].word).then(|| j - 1)
}

fn is_loop_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    first
        && name != "_"
        && name.len() <= 32
        && !name.contains("proxy")
        && !KEYWORDS.contains(&name)
        && name != "in"
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A plain literal, or `~` / `~/…` (the home folder, as the shell reads
/// it in the list).
fn is_plain_value(word: &str) -> bool {
    let rest = match word.strip_prefix('~') {
        Some(rest) => rest,
        None => word,
    };
    let plain = rest
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._/+,:@%-".contains(c));
    !word.is_empty()
        && plain
        && (rest.len() == word.len() || rest.is_empty() || rest.starts_with('/'))
}

/// Whether every mention of `name` in `body` is `$name` or `${name}`, with
/// no `=` or `+` glued after it.
fn names_only_as_values(body: &str, name: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    body.match_indices(name).all(|(at, _)| {
        let before = &body[..at];
        let after = &body[at + name.len()..];
        if before.chars().next_back().is_some_and(ident) || after.chars().next().is_some_and(ident)
        {
            return true; // Part of another word.
        }
        let next = if before.ends_with("${") {
            match after.strip_prefix('}') {
                Some(tail) => tail.chars().next(),
                None => return false,
            }
        } else if before.ends_with('$') {
            after.chars().next()
        } else {
            return false;
        };
        !matches!(next, Some('=' | '+'))
    })
}

/// `body` with `$name` and `${name}` replaced by `value` where the shell
/// would replace them: outside single quotes and not after a backslash.
/// `None` when `value` is the home folder (`~…`) and a reference to it does
/// not start an unquoted word followed by its end or a `/`.
fn with_value(body: &str, name: &str, value: &str) -> Option<String> {
    let tilde = value.starts_with('~');
    let mut out = String::with_capacity(body.len() + value.len());
    let mut in_single = false;
    let mut in_double = false;
    let mut word_start = true;
    let mut rest = body;
    while let Some(c) = rest.chars().next() {
        let after = &rest[c.len_utf8()..];
        let starts = word_start;
        word_start = !in_single && !in_double && " \t\n;&|".contains(c);
        match c {
            '\\' if !in_single => {
                out.push(c);
                let Some(next) = after.chars().next() else {
                    break;
                };
                out.push(next);
                rest = &after[next.len_utf8()..];
                continue;
            }
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '$' if !in_single => {
                if let Some(tail) = reference(after, name) {
                    let safe = starts
                        && !in_double
                        && tail.chars().next().is_none_or(|n| " \t\n;&|/".contains(n));
                    if tilde && !safe {
                        return None;
                    }
                    out.push_str(value);
                    rest = tail;
                    continue;
                }
            }
            _ => {}
        }
        out.push(c);
        rest = after;
    }
    Some(out)
}

/// What follows `name` or `{name}` when `after` (past a `$`) starts with
/// one and the name ends there.
fn reference<'a>(after: &'a str, name: &str) -> Option<&'a str> {
    if let Some(tail) = after.strip_prefix('{').and_then(|t| t.strip_prefix(name)) {
        return tail.strip_prefix('}');
    }
    let tail = after.strip_prefix(name)?;
    let continues = tail
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    (!continues).then_some(tail)
}
