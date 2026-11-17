//! `$HOME` read as the shell reads it.
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

/// `cmd` with every `$HOME` and `${HOME}` outside single quotes replaced by
/// this machine's home folder; `None` when there is none to replace or the
/// command is left as it is.
pub(super) fn with_home(cmd: &str) -> Option<String> {
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
