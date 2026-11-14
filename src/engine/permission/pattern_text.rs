//! The pattern or program operand of grep, rg, sed, awk and jq: text, not
//! a path (`grep -E '/a/b' src` searches for `/a/b`).
//!
//! Failing closed: a flag form not listed here that may carry the pattern
//! makes the whole command opaque, and every word is judged as a path as
//! before.

use super::classify::dequote;
use super::model::Token;

/// How a pattern program takes its pattern (or program) text. Flags not
/// listed are taken to be switches; a long one that is not listed and
/// stands right before the would-be pattern makes the segment opaque, as
/// it may take that word as its value.
struct PatternFlags {
    /// Flags whose next word is pattern text (`grep -e`).
    pattern: &'static [&'static str],
    /// Flags whose next word is a file holding the pattern (`grep -f`).
    file: &'static [&'static str],
    /// Short letters that start `pattern` or `file` flags: any other form
    /// of them (`-e/a`, `-rfp`) makes the segment opaque.
    letters: &'static str,
    /// Long names (`--regexp`) any abbreviation or `=` form of which makes
    /// the segment opaque.
    long: &'static [&'static str],
    /// Short letters that take a value: the rest of the cluster, else the
    /// next word (`-A 3`, `-A3`).
    value_letters: &'static str,
    /// Long flags whose next word is a value (`--glob '*.rs'`).
    value_long: &'static [&'static str],
    /// Long flags whose next two words are values (`jq --arg name v`).
    value_long2: &'static [&'static str],
    /// Flags after which no operand is a pattern (`rg --files`).
    no_pattern: &'static [&'static str],
    /// Flags an empty word may follow as their value (BSD `sed -i ''`).
    empty_value_after: &'static [&'static str],
}

const GREP: PatternFlags = PatternFlags {
    pattern: &["-e", "--regexp"],
    file: &["-f", "--file"],
    letters: "ef",
    long: &["regexp", "file"],
    value_letters: "ABCmdD",
    value_long: &[
        "--after-context",
        "--before-context",
        "--context",
        "--max-count",
        "--directories",
        "--devices",
        "--label",
        "--include",
        "--exclude",
        "--exclude-dir",
        "--exclude-from",
        "--binary-files",
    ],
    value_long2: &[],
    no_pattern: &[],
    empty_value_after: &[],
};

const RG: PatternFlags = PatternFlags {
    pattern: &["-e", "--regexp"],
    file: &["-f", "--file"],
    letters: "ef",
    long: &["regexp", "file"],
    value_letters: "ABCmgtTjMErd",
    value_long: &[
        "--glob",
        "--iglob",
        "--type",
        "--type-not",
        "--type-add",
        "--max-count",
        "--max-depth",
        "--pre",
        "--pre-glob",
        "--ignore-file",
        "--encoding",
        "--replace",
        "--context",
        "--after-context",
        "--before-context",
        "--threads",
        "--max-columns",
        "--max-filesize",
        "--sort",
        "--sortr",
        "--color",
        "--colors",
        "--engine",
        "--path-separator",
    ],
    value_long2: &[],
    no_pattern: &["--files", "--type-list"],
    empty_value_after: &[],
};

const SED: PatternFlags = PatternFlags {
    pattern: &["-e", "--expression"],
    file: &["-f", "--file"],
    letters: "ef",
    long: &["expression", "file"],
    value_letters: "l",
    value_long: &["--line-length"],
    value_long2: &[],
    no_pattern: &[],
    empty_value_after: &["-i"],
};

const AWK: PatternFlags = PatternFlags {
    pattern: &["-e", "--source"],
    file: &["-f", "--file", "-E", "--exec"],
    letters: "efE",
    long: &["source", "file", "exec"],
    value_letters: "Fvil",
    value_long: &["--field-separator", "--assign", "--include", "--load"],
    value_long2: &[],
    no_pattern: &[],
    empty_value_after: &[],
};

const JQ: PatternFlags = PatternFlags {
    pattern: &[],
    file: &["-f", "--from-file"],
    letters: "f",
    long: &["from-file"],
    value_letters: "L",
    value_long: &["--indent"],
    value_long2: &["--arg", "--argjson", "--slurpfile", "--rawfile"],
    no_pattern: &[],
    empty_value_after: &[],
};

fn pattern_flags(program: &str) -> Option<&'static PatternFlags> {
    match program {
        "grep" | "egrep" | "fgrep" => Some(&GREP),
        "rg" => Some(&RG),
        "sed" => Some(&SED),
        "awk" | "gawk" | "mawk" => Some(&AWK),
        "jq" => Some(&JQ),
        _ => None,
    }
}

/// For each token of `tokens`, whether it is pattern or program text of a
/// grep / rg / sed / awk / jq command rather than a possible path.
pub(super) fn text_words(tokens: &[Token]) -> Vec<bool> {
    let mut mask = vec![false; tokens.len()];
    let mut segment = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Word(w) => segment.push((i, w.as_str())),
            Token::Op(_) => mark_segment(&std::mem::take(&mut segment), &mut mask),
        }
    }
    mark_segment(&segment, &mut mask);
    mask
}

fn mark_segment(segment: &[(usize, &str)], mask: &mut [bool]) {
    let Some(((_, program), args)) = segment.split_first() else {
        return;
    };
    let Some(flags) = pattern_flags(&dequote(program)) else {
        return;
    };
    let words: Vec<&str> = args.iter().map(|(_, w)| *w).collect();
    for at in text_operands(&flags, &words).unwrap_or_default() {
        mask[args[at].0] = true;
    }
}

/// Which of `args` are pattern text; `None` when a flag form is not
/// understood, so nothing is.
fn text_operands(flags: &PatternFlags, args: &[&str]) -> Option<Vec<usize>> {
    let mut scan = Scan::default();
    let mut i = 0;
    while i < args.len() {
        let raw = args[i];
        i += 1;
        if let Some(takes_next) = redirect(raw) {
            i += usize::from(takes_next);
            continue;
        }
        let w = dequote(raw);
        let after = i.checked_sub(2).map(|p| dequote(args[p]));
        if w.is_empty() && after.is_some_and(|a| flags.empty_value_after.contains(&a.as_str())) {
            continue;
        }
        if scan.options_done || w == "-" || !w.starts_with('-') {
            scan.operand(i - 1)?;
            continue;
        }
        i += scan.flag(flags, &w, (i < args.len()).then_some(i))?;
    }
    Some(scan.text())
}

/// What a pattern program's words said so far.
#[derive(Default)]
struct Scan {
    /// Words a pattern flag gave as text.
    explicit: Vec<usize>,
    /// A flag gave the pattern (or said there is none): no operand is text.
    given_by_flag: bool,
    /// The first operand: the pattern, unless a flag gave it.
    first_operand: Option<usize>,
    options_done: bool,
    /// The index right after an unlisted long flag, which may be its value.
    after_unknown_long: Option<usize>,
}

impl Scan {
    fn operand(&mut self, at: usize) -> Option<()> {
        if self.first_operand.is_none() {
            if self.after_unknown_long == Some(at) {
                return None;
            }
            self.first_operand = Some(at);
        }
        Some(())
    }

    /// Takes in flag `w`, followed by the word at `next`; how many
    /// following words it consumes, or `None` when it is not understood.
    fn flag(&mut self, flags: &PatternFlags, w: &str, next: Option<usize>) -> Option<usize> {
        if w == "--" {
            self.options_done = true;
            return Some(0);
        }
        if flags.pattern.contains(&w) {
            self.given_by_flag = true;
            self.explicit.extend(next);
            return Some(1);
        }
        if flags.file.contains(&w) || flags.no_pattern.contains(&w) {
            self.given_by_flag = true;
            return Some(usize::from(flags.file.contains(&w)));
        }
        if is_unclear_flag(flags, w) {
            return None;
        }
        if flags.value_long2.contains(&w) {
            return Some(2);
        }
        if flags.value_long.contains(&w) {
            return Some(1);
        }
        if w.starts_with("--") {
            self.after_unknown_long = next.filter(|_| !w.contains('='));
            return Some(0);
        }
        Some(usize::from(short_takes_next(flags, w)))
    }

    fn text(self) -> Vec<usize> {
        if self.given_by_flag {
            return self.explicit;
        }
        self.first_operand.into_iter().collect()
    }
}

/// A short cluster whose value letter ends it, so its value is the next word.
fn short_takes_next(flags: &PatternFlags, w: &str) -> bool {
    let cluster = &w[1..];
    match cluster.find(|c| flags.value_letters.contains(c)) {
        Some(at) => at + 1 == cluster.len(),
        None => false,
    }
}

/// A flag that may carry or name a pattern in a form not listed: a short
/// cluster holding a pattern letter, or a long name's abbreviation or
/// `=` form.
fn is_unclear_flag(flags: &PatternFlags, w: &str) -> bool {
    if let Some(long) = w.strip_prefix("--") {
        let name = long.split('=').next().unwrap_or("");
        return !name.is_empty() && flags.long.iter().any(|l| l.starts_with(name));
    }
    let cluster = &w[1..];
    let value_at = cluster
        .find(|c| flags.value_letters.contains(c))
        .unwrap_or(cluster.len());
    cluster[..value_at]
        .chars()
        .any(|c| flags.letters.contains(c))
}

/// A redirect word: `Some(true)` when its target is the next word
/// (`>`, `2>>`), `Some(false)` when glued (`2>/x`), `None` when no redirect.
fn redirect(raw: &str) -> Option<bool> {
    let rest = raw.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&');
    if !rest.starts_with(['>', '<']) {
        return None;
    }
    Some(rest.trim_start_matches(['>', '<', '|', '&']).is_empty())
}
