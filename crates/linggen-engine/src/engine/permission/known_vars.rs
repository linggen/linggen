//! `$HOME` and `$?` read as the shell reads them.
//!
//! A `$` outside single quotes makes a command unclear to the gate — its
//! words are decided at run time — so the whole command asks for admin.
//! One variable is no mystery: the home folder. Putting its value in before
//! the command is judged turns `ls "$HOME/notes"` from "cannot be read"
//! into a read of that folder, judged like any other path.
//!
//! Failing closed, like the rest of the gate: nothing is put in when the
//! command sets `HOME` anywhere (a later word could mean another folder),
//! or when the home path holds a character the shell would treat specially.
//!
//! `$?`, the exit status of the last command, is only ever a number: it is
//! read as one where nothing is glued to it ([`with_status`]).

use super::classify::dequote;
use super::model::{command_run, command_tokens, Joint, Token};

/// `cmd` as the shell would read it, when any of it is read here: each
/// `for` loop over written words written out (`super::for_loop`), then
/// `$HOME` put in, then `$?`. `None` when nothing is.
pub(super) fn read_as_the_shell(cmd: &str) -> Option<String> {
    let mut read = super::for_loop::unrolled(cmd);
    for step in [with_home, with_status] {
        let now = read.as_deref().unwrap_or(cmd);
        read = step(now).or(read);
    }
    read
}

/// `cmd` with every `$HOME` and `${HOME}` outside single quotes replaced by
/// this machine's home folder; `None` when there is none to replace or the
/// command is left as it is.
fn with_home(cmd: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    with_home_at(cmd, home.to_str()?)
}

fn with_home_at(cmd: &str, home: &str) -> Option<String> {
    let plain = !home.is_empty()
        && home.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c));
    if !plain || !cmd.contains("HOME") || sets_home(cmd) {
        return None;
    }
    let mut out = String::with_capacity(cmd.len() + home.len());
    let mut replaced = false;
    let mut in_single = false;
    let mut in_double = false;
    let mut rest = cmd;
    while let Some(c) = rest.chars().next() {
        let after = &rest[c.len_utf8()..];
        match c {
            '\\' if !in_single => {
                // The next character is taken as itself.
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
                if let Some(tail) = home_reference(after) {
                    out.push_str(home);
                    replaced = true;
                    rest = tail;
                    continue;
                }
            }
            _ => {}
        }
        out.push(c);
        rest = after;
    }
    replaced.then_some(out)
}

/// What follows `HOME` or `{HOME}` when `after` (the text past a `$`)
/// starts with one — and the name ends there (`$HOMEBREW_PREFIX` is another
/// variable).
fn home_reference(after: &str) -> Option<&str> {
    if let Some(tail) = after.strip_prefix("{HOME}") {
        return Some(tail);
    }
    let tail = after.strip_prefix("HOME")?;
    let continues = tail
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    (!continues).then_some(tail)
}

/// Whether the command assigns `HOME` anywhere (`HOME=…`, `env HOME=…`,
/// `export HOME=…`): after that, `$HOME` may be another folder.
fn sets_home(cmd: &str) -> bool {
    cmd.match_indices("HOME=").any(|(at, _)| {
        !cmd[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// `cmd` with every `$?` and `${?}` outside single quotes read as a number
/// (`0`): the exit status of the last command, which names no path and no
/// command. Only where nothing is glued to it: it ends its word (or its
/// double-quoted string), and the word before it holds only letters,
/// digits and `_=:,+@%` (`EXIT=$?`, `"rc $?"`) — no flag, path or tilde
/// it could complete. `None` when there is none to read, or when the
/// number would stand where a program is named.
pub(super) fn with_status(cmd: &str) -> Option<String> {
    let out = status_replaced(cmd)?;
    (!runs_the_number(&out)).then_some(out)
}

/// What stands in for `$?`.
const STATUS: &str = "0";

fn status_replaced(cmd: &str) -> Option<String> {
    if !cmd.contains("$?") && !cmd.contains("${?}") {
        return None;
    }
    let mut out = String::with_capacity(cmd.len());
    let mut replaced = false;
    let mut in_single = false;
    let mut in_double = false;
    // The word so far holds only what a number may follow.
    let mut plain_so_far = true;
    let mut rest = cmd;
    while let Some(c) = rest.chars().next() {
        let after = &rest[c.len_utf8()..];
        let ends_word = !in_single && !in_double && ends_a_word(c);
        if c == '$' && !in_single && plain_so_far {
            if let Some(tail) = status_reference(after).filter(|t| ends_here(t, in_double)) {
                out.push_str(STATUS);
                replaced = true;
                rest = tail;
                continue;
            }
        }
        match c {
            '\\' if !in_single => {
                // The next character is taken as itself.
                out.push(c);
                plain_so_far = false;
                let Some(next) = after.chars().next() else {
                    break;
                };
                out.push(next);
                rest = &after[next.len_utf8()..];
                continue;
            }
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ if ends_word => plain_so_far = true,
            ' ' | '\t' if in_double => {}
            c => plain_so_far &= c.is_ascii_alphanumeric() || "_=:,+@%".contains(c),
        }
        out.push(c);
        rest = after;
    }
    replaced.then_some(out)
}

/// What follows `?` or `{?}` when `after` (the text past a `$`) starts with one.
fn status_reference(after: &str) -> Option<&str> {
    after
        .strip_prefix("{?}")
        .or_else(|| after.strip_prefix('?'))
}

/// Whether `tail`, the text after a `$?`, ends its word there: at the end,
/// a space, or `;`, `&`, `|`, `)`, a newline — inside double quotes, at a
/// space or at the closing quote followed by one of those.
fn ends_here(tail: &str, in_double: bool) -> bool {
    let next = tail.chars().next();
    if !in_double {
        return next.is_none_or(ends_a_word);
    }
    match next {
        Some(' ' | '\t') => true,
        Some('"') => tail[1..].chars().next().is_none_or(ends_a_word),
        _ => false,
    }
}

fn ends_a_word(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | ';' | '&' | '|' | ')')
}

/// Whether any command of `cmd` runs the program `0` — the number would
/// then name a command.
fn runs_the_number(cmd: &str) -> bool {
    let mut words: Vec<String> = Vec::new();
    for token in command_tokens(cmd)
        .into_iter()
        .chain([Token::Op(Joint::Seq)])
    {
        match token {
            Token::Word(w) => words.push(dequote(&w)),
            Token::Op(_) => {
                let all: Vec<&str> = words.iter().map(String::as_str).collect();
                if command_run(&all).first() == Some(&STATUS) {
                    return true;
                }
                words.clear();
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::with_home_at;

    const HOME: &str = "/Users/alex";

    #[test]
    fn home_is_put_in_where_the_shell_would() {
        let cases = [
            ("ls $HOME/notes", "ls /Users/alex/notes"),
            ("ls \"$HOME/my notes\"", "ls \"/Users/alex/my notes\""),
            ("ls ${HOME}/a ${HOME}", "ls /Users/alex/a /Users/alex"),
            ("mkdir -p \"$HOME/.cache/x\" && ls $HOME", "mkdir -p \"/Users/alex/.cache/x\" && ls /Users/alex"),
        ];
        for (cmd, read_as) in cases {
            assert_eq!(with_home_at(cmd, HOME).as_deref(), Some(read_as), "{cmd}");
        }
    }

    #[test]
    fn it_is_left_alone_where_the_shell_would_not_or_might_not() {
        for cmd in [
            // Single quotes and a backslash keep the dollar a dollar.
            "echo '$HOME/x'",
            "echo \\$HOME/x",
            // Another variable whose name starts the same.
            "ls $HOMEBREW_PREFIX/bin",
            // The command sets HOME: a later word could mean another folder.
            "HOME=/tmp/h ls $HOME",
            "export HOME=/tmp/h; ls $HOME",
            "env HOME=/tmp/h CARGO_HOME=$HOME/.cargo cargo test",
            // Nothing to read.
            "ls /tmp",
        ] {
            assert_eq!(with_home_at(cmd, HOME), None, "{cmd}");
        }
        // A home folder the shell would split or expand is not put in.
        assert_eq!(with_home_at("ls $HOME", "/Users/a b"), None);
        assert_eq!(with_home_at("ls $HOME", "/Users/a$b"), None);
        // A second variable keeps the command unclear, HOME read or not.
        assert_eq!(with_home_at("ls $HOME/$X", HOME).as_deref(), Some("ls /Users/alex/$X"));
    }
}
