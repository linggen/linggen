//! The registry: every agent memory Linggen reads, declared once.
//!
//! An entry says whose memory it is, which files it may be read from (its
//! roots — the only places [`super::read`] and [`super::search`] reach), and
//! how to turn its index into forwarded lines for a session's scope. This is
//! the one file that names another agent's folders.

use super::{shorten, Agent, Env, Scope};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// One agent memory Linggen can read.
pub(crate) struct Source {
    pub id: &'static str,
    pub agent: Agent,
    pub label: &'static str,
    /// Files and folders this source's notes live in, as they exist now.
    pub roots: fn(&Env) -> Vec<PathBuf>,
    /// This source's index lines for a session's scope, or why there are none.
    pub lines: fn(&Env, &Scope) -> Result<Vec<Line>, Skip>,
}

/// The declared list. Add an agent here and nowhere else.
pub(crate) const SOURCES: &[Source] = &[CLAUDE_PROJECTS, CLAUDE_GLOBAL, CODEX_MEMORIES];

/// Why a source gave no lines. Shown, never fatal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Skip {
    NoData,
    UnknownFormat,
    Unreadable(String),
}

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Skip::NoData => f.write_str("no data"),
            Skip::UnknownFormat => f.write_str("format not known yet"),
            Skip::Unreadable(why) => write!(f, "unreadable: {why}"),
        }
    }
}

/// One forwarded index line and where it came from.
#[derive(Clone, Debug)]
pub(crate) struct Line {
    pub source: &'static str,
    pub agent: Agent,
    /// What the line is about, as its group header says it ("project ~/x").
    pub about: String,
    /// The folder or file its notes are in.
    pub place: PathBuf,
    /// How far the project is from the session's folder; `None` = the person.
    pub distance: Option<usize>,
    pub text: String,
    /// Seconds since the epoch the line's note (else its index) last changed.
    pub changed: u64,
}

/// A source and whether it can be read now, for Settings and the tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceReport {
    pub id: &'static str,
    pub label: &'static str,
    pub agent: Agent,
    pub state: String,
}

/// Every source with what it reads as, for a scope.
pub fn reports(env: &Env, scope: &Scope) -> Vec<SourceReport> {
    SOURCES
        .iter()
        .map(|s| SourceReport {
            id: s.id,
            label: s.label,
            agent: s.agent,
            state: match (s.lines)(env, scope) {
                Ok(lines) => format!("{} index lines", lines.len()),
                Err(skip) => skip.to_string(),
            },
        })
        .collect()
}

/// Claude Code's index entries are read as it loads them: the first 200
/// lines or 25 KB of `MEMORY.md`, whichever comes first.
const INDEX_LINES: usize = 200;
const INDEX_BYTES: usize = 25 * 1024;
/// One forwarded line, in characters.
const LINE_CHARS: usize = 160;
/// How many headings of a CLAUDE.md are forwarded.
const HEADINGS: usize = 12;

const CLAUDE_PROJECTS: Source = Source {
    id: "claude-code-project",
    agent: Agent::ClaudeCode,
    label: "Claude Code project memory",
    roots: claude_project_roots,
    lines: claude_project_lines,
};

const CLAUDE_GLOBAL: Source = Source {
    id: "claude-code-instructions",
    agent: Agent::ClaudeCode,
    label: "Claude Code ~/.claude/CLAUDE.md",
    roots: |env| existing(vec![env.home().join(".claude").join("CLAUDE.md")]),
    lines: claude_global_lines,
};

/// Codex memories were switched on only just: the folder is read-only
/// searchable, but its format is not known, so it forwards nothing until a
/// later pass reads what Codex really stores.
const CODEX_MEMORIES: Source = Source {
    id: "codex-memories",
    agent: Agent::Codex,
    label: "Codex memories",
    roots: |env| existing(vec![env.home().join(".codex").join("memories")]),
    lines: |env, _| match has_files(&env.home().join(".codex").join("memories")) {
        true => Err(Skip::UnknownFormat),
        false => Err(Skip::NoData),
    },
};

fn existing(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.into_iter().filter(|p| p.exists()).collect()
}

fn has_files(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// Claude Code's folder name for a project: every character that is not an
/// ASCII letter or digit becomes `-` (a character outside the BMP, two).
/// The mapping is lossy, so it is only ever applied forward: a folder is
/// found by encoding a path, never by decoding a name.
pub(crate) fn encode_project_path(path: &Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        match c.is_ascii_alphanumeric() {
            true => out.push(c),
            false => (0..c.len_utf16()).for_each(|_| out.push('-')),
        }
    }
    out
}

fn claude_projects_dir(env: &Env) -> PathBuf {
    env.home().join(".claude").join("projects")
}

fn claude_project_roots(env: &Env) -> Vec<PathBuf> {
    let Ok(projects) = fs::read_dir(claude_projects_dir(env)) else {
        return Vec::new();
    };
    let mut roots: Vec<PathBuf> = projects
        .flatten()
        .map(|p| p.path().join("memory"))
        .filter(|m| m.is_dir())
        .collect();
    roots.sort();
    roots
}

/// The folders a session at `cwd` is inside, nearest first: `cwd`, its
/// parents, up to but never including the home folder or `/` (a memory
/// there belongs to no project). The real path is tried too, as a host may
/// have resolved a link.
pub(crate) fn project_chain(env: &Env, cwd: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = Vec::new();
    let real = cwd.canonicalize().ok();
    for start in std::iter::once(cwd.to_path_buf()).chain(real) {
        for dir in start.ancestors() {
            let stop = dir == env.home() || dir.parent().is_none();
            if stop {
                break;
            }
            if !chain.iter().any(|seen| seen == dir) {
                chain.push(dir.to_path_buf());
            }
        }
    }
    chain
}

fn claude_project_lines(env: &Env, scope: &Scope) -> Result<Vec<Line>, Skip> {
    let Some(cwd) = &scope.cwd else {
        return Err(Skip::NoData);
    };
    let mut lines = Vec::new();
    let mut trouble = None;
    for (distance, dir) in project_chain(env, cwd).iter().enumerate() {
        let folder = claude_projects_dir(env)
            .join(encode_project_path(dir))
            .join("memory");
        let index = folder.join("MEMORY.md");
        if !index.is_file() {
            continue;
        }
        match fs::read_to_string(&index) {
            Ok(text) => lines.extend(index_lines(env, &text, &folder, &index, dir, distance)),
            Err(e) => trouble = Some(Skip::Unreadable(e.kind().to_string())),
        }
    }
    match (lines.is_empty(), trouble) {
        (true, Some(skip)) => Err(skip),
        (true, None) => Err(Skip::NoData),
        (false, _) => Ok(lines),
    }
}

/// The bullet lines of one `MEMORY.md`, the way Claude Code loads it.
fn index_lines(env: &Env, text: &str, folder: &Path, index: &Path, project: &Path, distance: usize) -> Vec<Line> {
    let index_changed = changed_secs(index);
    let mut used = 0;
    let mut out = Vec::new();
    for raw in text.lines().take(INDEX_LINES) {
        used += raw.len() + 1;
        if used > INDEX_BYTES {
            break;
        }
        let line = raw.trim();
        if !(line.starts_with("- ") || line.starts_with("* ")) {
            continue;
        }
        let note = linked_note(line, folder);
        out.push(Line {
            source: CLAUDE_PROJECTS.id,
            agent: Agent::ClaudeCode,
            about: format!("project {}", env.tilde(project)),
            place: folder.to_path_buf(),
            distance: Some(distance),
            text: shorten(line, LINE_CHARS),
            changed: note.as_deref().map(changed_secs).unwrap_or(index_changed),
        });
    }
    out
}

/// The note a bullet links to (`[title](file.md)`), when it is a plain file
/// name inside the folder.
fn linked_note(line: &str, folder: &Path) -> Option<PathBuf> {
    let open = line.find("](")? + 2;
    let target = line[open..].split(')').next()?;
    let plain = !target.is_empty()
        && !target.contains(['/', '\\', ':'])
        && !target.starts_with('.')
        && target.ends_with(".md");
    plain.then(|| folder.join(target))
}

fn claude_global_lines(env: &Env, _: &Scope) -> Result<Vec<Line>, Skip> {
    let file = env.home().join(".claude").join("CLAUDE.md");
    if !file.is_file() {
        return Err(Skip::NoData);
    }
    let text = fs::read_to_string(&file).map_err(|e| Skip::Unreadable(e.kind().to_string()))?;
    let changed = changed_secs(&file);
    let lines: Vec<Line> = text
        .lines()
        .filter(|l| l.starts_with('#'))
        .map(|l| l.trim_start_matches('#').trim())
        .filter(|h| !h.is_empty())
        .take(HEADINGS)
        .map(|heading| Line {
            source: CLAUDE_GLOBAL.id,
            agent: Agent::ClaudeCode,
            about: format!("the person, from {}", env.tilde(&file)),
            place: file.clone(),
            distance: None,
            text: shorten(&format!("- {heading}"), LINE_CHARS),
            changed,
        })
        .collect();
    match lines.is_empty() {
        true => Err(Skip::NoData),
        false => Ok(lines),
    }
}

fn changed_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}
