//! Bounded shell read scopes. Classification and scope proof are separate:
//! a read-class program does not necessarily name every input it can read.

use super::classify::{classify_bash, dequote};
use super::model::{command_run, command_tokens, file_name, is_odd_space, resolve_bash_arg, BashClass, Token};
use super::sensitive;
use std::path::{Path, PathBuf};

pub(super) fn recursive_search(cmd: &str) -> bool {
    let tokens = command_tokens(cmd);
    tokens.split(|token| matches!(token, Token::Op(_))).any(|segment| {
        let words: Vec<_> = segment.iter().filter_map(|token| match token {
            Token::Word(word) => Some(word.as_str()),
            Token::Op(_) => None,
        }).collect();
        let Some((program, args)) = command_run(&words).split_first() else { return false };
        match file_name(program).as_str() {
            "rg" | "find" => true,
            "grep" | "egrep" | "fgrep" => args.iter().map(|word| dequote(word)).take_while(|word| word != "--").any(|word| {
                matches!(word.as_str(), "--recursive" | "--dereference-recursive")
                    || word.strip_prefix('-').is_some_and(|flags| {
                        !flags.starts_with('-') && flags.contains(['r', 'R'])
                    })
            }),
            _ => false,
        }
    })
}

pub(super) fn tree_is_unprotected(root: &Path) -> bool {
    let mut pending = vec![root.to_path_buf()];
    let mut budget = 16_384;
    while let Some(path) = pending.pop() {
        budget -= 1;
        if budget == 0 || sensitive::protected(&path).is_some() {
            return false;
        }
        let Ok(metadata) = path.symlink_metadata() else {
            if path == root && !path.exists() { continue; }
            return false;
        };
        // Traversal through a directory link is not part of this proof.
        if metadata.is_symlink() && path.is_dir() { return false; }
        if metadata.is_dir() {
            let Ok(entries) = path.read_dir() else { return false; };
            for entry in entries {
                let Ok(entry) = entry else { return false; };
                pending.push(entry.path());
                if pending.len() >= budget { return false; }
            }
        }
    }
    true
}

pub(super) struct ReadTargets {
    pub paths: Vec<PathBuf>,
    pub has_pattern: bool,
}

pub(super) fn targets(cmd: &str, cwd: &Path, coding: bool) -> Option<ReadTargets> {
    let tokens = command_tokens(cmd);
    let mut out = ReadTargets { paths: Vec::new(), has_pattern: false };
    if cfg!(windows) { return None; }
    for segment in tokens.split(|t| matches!(t, Token::Op(_))).filter(|s| !s.is_empty()) {
        let words: Vec<_> = segment.iter().map(|t| match t {
            Token::Word(w) => w.as_str(),
            Token::Op(_) => unreachable!(),
        }).collect();
        let args = operands(&words)?;
        let mut literals = Vec::new();
        for &word in args {
            let plain = dequote(word);
            if plain == "-" {
                literals.push(word);
                continue;
            }
            if word.contains(['*', '?', '[', ']']) && word == plain {
                out.paths.extend(expand(word, cwd)?);
                out.has_pattern = true;
                literals.push("input");
            } else {
                if plain.is_empty() || plain.starts_with('-') || (plain.starts_with('~') && plain != "~" && !plain.starts_with("~/")) {
                    return None;
                }
                let path = if word != plain && plain.starts_with('~') {
                    cwd.join(&plain)
                } else {
                    resolve_bash_arg(&plain, cwd)
                };
                out.paths.push(path);
                literals.push(word);
            }
        }
        let fixed = &words[..words.len() - args.len()];
        let text = fixed.iter().copied().chain(literals).collect::<Vec<_>>().join(" ");
        if classify_bash(&text, coding).0 != BashClass::Read {
            return None;
        }
    }
    Some(out)
}

fn operands<'a>(words: &'a [&'a str]) -> Option<&'a [&'a str]> {
    let (program, args) = words.split_first()?;
    // Check raw words, not the dequoted result: shell syntax is not data.
    if words.iter().any(|w| {
        let p = dequote(w);
        w.chars().any(is_odd_space) || w.contains(['$', '`', '\\', '\n']) || (p == *w && w.contains(['<', '>', '{', '}', '(', ')', '#']))
            || (p != *w && *w != format!("'{p}'") && *w != format!("\"{p}\""))
    }) { return None; }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match *program {
        "sed" if args.first() == Some(&"-n") && args.len() >= 2 => Some(&args[2..]),
        "cat" | "wc" => Some(args.strip_prefix(&["--"]).unwrap_or(args)),
        "tail" | "head" => match args {
            ["-n", n, rest @ ..] if digits(n) => Some(rest),
            [n, rest @ ..] if n.strip_prefix('-').is_some_and(digits) => Some(rest),
            _ => Some(args),
        },
        "grep" => {
            let at = args.iter().take_while(|s| {
                s.strip_prefix('-').is_some_and(|flags| !flags.is_empty() && flags.chars().all(|c| "nEFivclqwxHhso".contains(c)))
            }).count();
            let rest = args.get(at..)?;
            let rest = rest.strip_prefix(&["--"]).unwrap_or(rest);
            let (pattern, files) = rest.split_first()?;
            (!pattern.starts_with('-') && (!pattern.contains(['*', '?', '[']) || dequote(pattern) != *pattern)).then_some(files)
        }
        "awk" => {
            let (program, files) = args.split_first()?;
            (!program.starts_with('-') && dequote(program) != *program && !files.iter().any(|s| s.contains('='))).then_some(files)
        }
        "echo" | "pwd" | "sleep" | "true" | "false" if !args.iter().any(|s| s.contains(['*', '?', '[', '~'])) => Some(&[]),
        _ => None,
    }
}

fn expand(word: &str, cwd: &Path) -> Option<Vec<PathBuf>> {
    if word.starts_with('-') || (word.starts_with('~') && !word.starts_with("~/")) {
        return None;
    }
    let path = resolve_bash_arg(word, cwd);
    let mut prefix = PathBuf::new();
    let mut pattern = Vec::new();
    for part in path.components() {
        let s = part.as_os_str().to_str()?;
        if s == ".." { return None; }
        if pattern.is_empty() && !s.contains(['*', '?', '[', ']']) {
            prefix.push(part);
        } else {
            pattern.push(s);
        }
    }
    if prefix == cwd && !word.contains('/') { return None; }
    if sensitive::protected(&prefix).is_some() || !prefix.canonicalize().ok()?.is_dir() {
        return None;
    }
    if !tree_is_unprotected(&prefix) {
        return None;
    }
    let mut matches = vec![prefix];
    for component in pattern {
        if !component.contains(['*', '?', '[', ']']) {
            matches = matches.into_iter().map(|p| p.join(component)).filter(|p| p.exists()).collect();
            continue;
        }
        if component.starts_with('.') || component.contains([']', '[']) && (component.starts_with("[.") || component.starts_with("[!"))
            || component.contains([']', '[']) && component.contains("[:")
            || component.contains("**") {
            return None;
        }
        let glob = globset::GlobBuilder::new(component).literal_separator(true).build().ok()?.compile_matcher();
        let mut next = Vec::new();
        for base in matches {
            if !base.is_dir() { continue; }
            for entry in base.read_dir().ok()? {
                let entry = entry.ok()?;
                let name = entry.file_name();
                if !name.to_str()?.starts_with('.') && glob.is_match(Path::new(&name)) {
                    next.push(entry.path());
                }
            }
        }
        matches = next;
    }
    (!matches.is_empty() && matches.iter().all(|p| p.canonicalize().is_ok())).then_some(matches)
}
