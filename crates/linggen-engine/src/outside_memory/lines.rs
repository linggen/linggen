//! From a reader's places to forwarded index lines: the closed set of
//! extractors, the scope rules, the version guards and the zero-match
//! canary. Text only: the files are read by [`super::disk`], through a
//! [`super::readers::Located`] the registry made.
//!
//! A line is about the person (forwarded in every folder) or about a project
//! (forwarded only when the project is in the session's chain). A note keeps
//! the scope its writer gave it: nothing here moves a line from one to the
//! other.

use super::decl::{Each, ExpectDecl, FileText, LinesDecl, MatchBy, PartDecl, PlaceDecl, Scope as LineScope};
use super::disk::{self, Projects};
use super::paths::{encode_project_path, project_chain};
use super::readers::{Agent, Located, Reader, Registry};
use super::redact::one_line;
use super::{shorten, Env, Scope};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One forwarded line, in characters.
const LINE_CHARS: usize = 160;
/// What is read of a file when its declaration names no window.
const DEFAULT_READ_BYTES: usize = 256 * 1024;

/// Why a place gave no lines. Shown, never fatal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Skip {
    NoData,
    /// The text says why ("format not known: first line is 'v2', expected 'v1'").
    UnknownFormat(String),
    Unreadable(String),
    /// A newer copy of the same place (same `group`) is the one forwarded.
    Older(String),
}

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Skip::NoData => f.write_str("no data"),
            Skip::UnknownFormat(why) => f.write_str(why),
            Skip::Unreadable(why) => write!(f, "unreadable: {why}"),
            Skip::Older(newer) => write!(f, "older than {newer}"),
        }
    }
}

/// One forwarded index line and where it came from.
#[derive(Clone, Debug)]
pub(crate) struct Line {
    pub source: String,
    pub agent: Agent,
    /// What the line is about, as its group header says it ("project ~/x").
    pub about: String,
    /// The folder or file its notes are in.
    pub place: PathBuf,
    /// How far the project is from the session's folder; `None` = the person.
    pub distance: Option<usize>,
    pub text: String,
    /// Seconds since the epoch the line's note (else its file) last changed.
    pub changed: u64,
}

/// What one place gave for a session.
pub(crate) struct PlaceResult {
    pub source: String,
    pub agent: Agent,
    pub label: String,
    pub outcome: Result<Vec<Line>, Skip>,
    /// Lines left out because a place's `first` cap, or a project heading that
    /// names no folder of the session's chain, kept them back (still found by
    /// search).
    pub unmatched: usize,
}

/// Every forwarded place of every reader, for the session's folder.
pub(crate) fn place_results(registry: &Registry, env: &Env, scope: &Scope) -> Vec<PlaceResult> {
    let chain = scope.cwd.as_deref().map(|cwd| project_chain(env, cwd)).unwrap_or_default();
    let readable: HashMap<PathBuf, Located> = registry.files().into_iter().map(|f| (f.path().to_path_buf(), f)).collect();
    let mut results = Vec::new();
    for reader in &registry.readers {
        let newest = newest_in_groups(registry, reader);
        for place in reader.decl.places.iter().filter(|p| p.forward) {
            let source = reader.source_id(place);
            let newer = place.group.as_ref().and_then(|g| newest.get(g)).filter(|(id, _)| *id != place.id);
            let has_files = || !registry.place_files(reader, place, &Projects::Any).is_empty();
            let (outcome, unmatched) = match newer {
                Some((newer, _)) if has_files() => (Err(Skip::Older(format!("{}/{newer}", reader.decl.agent))), 0),
                _ => forward_place(registry, env, reader, place, &chain, &readable),
            };
            results.push(PlaceResult { source, agent: reader.agent(), label: reader.decl.label.clone(), outcome, unmatched });
        }
    }
    results
}

/// For each `group` of a reader: the place whose files changed last, and when.
fn newest_in_groups(registry: &Registry, reader: &Reader) -> HashMap<String, (String, u64)> {
    let mut newest: HashMap<String, (String, u64)> = HashMap::new();
    for place in reader.decl.places.iter().filter(|p| p.forward) {
        let Some(group) = &place.group else {
            continue;
        };
        let changed = registry
            .place_files(reader, place, &Projects::Any)
            .iter()
            .map(|(file, _)| disk::changed(file))
            .max();
        let Some(changed) = changed else {
            continue;
        };
        if newest.get(group).is_none_or(|(_, at)| changed > *at) {
            newest.insert(group.clone(), (place.id.clone(), changed));
        }
    }
    newest
}

/// A place and whether it can be read now, for Settings and the tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceReport {
    pub id: String,
    pub label: String,
    pub agent: Agent,
    pub state: String,
}

/// Every forwarded place with what it reads as, for a scope.
pub fn reports(env: &Env, scope: &Scope) -> Vec<SourceReport> {
    place_results(&Registry::load(env), env, scope)
        .into_iter()
        .map(|r| SourceReport {
            id: r.source,
            label: r.label,
            agent: r.agent,
            state: match r.outcome {
                Ok(lines) => format!("{} index lines", lines.len()),
                Err(skip) => skip.to_string(),
            },
        })
        .collect()
}

/// One place's lines for a session whose chain is `chain`, and how many
/// project lines no heading of the chain claimed.
fn forward_place(registry: &Registry, env: &Env, reader: &Reader, place: &PlaceDecl, chain: &[PathBuf], readable: &HashMap<PathBuf, Located>) -> (Result<Vec<Line>, Skip>, usize) {
    let names: Vec<(usize, String)> = chain.iter().enumerate().map(|(d, dir)| (d, encode_project_path(dir))).collect();
    let files = registry.place_files(reader, place, &Projects::Chain(&names));
    if files.is_empty() {
        return (Err(Skip::NoData), 0);
    }
    let Some(lines) = &place.lines else {
        return (Err(Skip::NoData), 0);
    };
    let ctx = Ctx { env, place, lines, chain, readable };
    let mut out = Vec::new();
    let mut trouble = None;
    let mut unmatched = 0;
    let mut by_file: Vec<(Located, Option<usize>)> = files;
    if lines.each == Each::File {
        by_file.sort_by_key(|(file, _)| std::cmp::Reverse(disk::changed(file)));
        by_file.truncate(lines.newest.unwrap_or(usize::MAX));
    }
    for (file, distance) in &by_file {
        match ctx.lines_of(file, *distance) {
            Ok((mut got, left)) => {
                out.append(&mut got);
                unmatched += left;
            }
            Err(skip) => trouble = trouble.or(Some(skip)),
        }
    }
    match (out.is_empty(), trouble) {
        (false, _) => (Ok(out), unmatched),
        (true, Some(skip)) => (Err(skip), unmatched),
        (true, None) => (Err(Skip::NoData), unmatched),
    }
}

/// What reading one place needs to know.
struct Ctx<'a> {
    env: &'a Env,
    place: &'a PlaceDecl,
    lines: &'a LinesDecl,
    chain: &'a [PathBuf],
    /// Every file the registry reads, to date the notes an index links to.
    readable: &'a HashMap<PathBuf, Located>,
}

/// A line before it knows its source.
struct Raw {
    text: String,
    /// The note file the line links to, by name.
    link: Option<String>,
}

impl Ctx<'_> {
    fn lines_of(&self, file: &Located, distance: Option<usize>) -> Result<(Vec<Line>, usize), Skip> {
        let window = self.lines.read_first_bytes.unwrap_or(DEFAULT_READ_BYTES) as u64;
        let bytes = disk::read_note(file, window).map_err(|e| Skip::Unreadable(e.kind().to_string()))?;
        let whole = String::from_utf8_lossy(&bytes).into_owned();
        if let Some(expect) = &self.place.expect {
            check_expect(expect, &whole)?;
        }
        let text = windowed(&whole, self.lines);
        let (groups, unmatched) = match self.place.parts.is_empty() {
            true => (vec![(self.place.scope, distance, extract(self.lines, &text))], 0),
            false => self.parts_of(&text),
        };
        let found: usize = groups.iter().map(|(_, _, raws)| raws.len()).sum();
        if found == 0 && unmatched == 0 && self.place.expect.is_none() && has_content(&text) {
            return Err(Skip::UnknownFormat("format not known: nothing matched".into()));
        }
        let changed = disk::changed(file);
        let mut out = Vec::new();
        let mut capped = 0;
        for (scope, distance, raws) in groups {
            let cap = self.lines.first.unwrap_or(usize::MAX);
            capped += raws.len().saturating_sub(cap);
            for raw in raws.into_iter().take(cap) {
                out.push(self.line(file, scope, distance, raw, changed));
            }
        }
        Ok((out, unmatched + capped))
    }

    fn line(&self, file: &Located, scope: LineScope, distance: Option<usize>, raw: Raw, file_changed: u64) -> Line {
        let folder = file.path().parent().map(Path::to_path_buf).unwrap_or_default();
        let (about, place, distance) = match scope {
            LineScope::Person if self.lines.each == Each::File => (format!("the person, from {}", self.env.tilde(&folder)), folder.clone(), None),
            LineScope::Person => (format!("the person, from {}", self.env.tilde(file.path())), file.path().to_path_buf(), None),
            LineScope::Project => {
                let dir = distance.and_then(|d| self.chain.get(d)).map(|d| self.env.tilde(d)).unwrap_or_default();
                (format!("project {dir}"), folder.clone(), distance)
            }
        };
        let changed = raw
            .link
            .and_then(|name| self.readable.get(&folder.join(name)))
            .map_or(file_changed, disk::changed);
        Line {
            source: file.source().to_string(),
            agent: file.agent().clone(),
            about,
            place,
            distance,
            text: shorten(&one_line(&raw.text), LINE_CHARS),
            changed,
        }
    }

    /// The sections a place's `parts` name, with the scope and chain distance
    /// of each group of lines, and the count of project lines no heading of
    /// the chain claimed.
    fn parts_of(&self, text: &str) -> (Vec<(LineScope, Option<usize>, Vec<Raw>)>, usize) {
        let all: Vec<&str> = text.lines().collect();
        let mut groups = Vec::new();
        let mut unmatched = 0;
        for part in &self.place.parts {
            let Some((start, end)) = section(&all, &part.under) else {
                continue;
            };
            let body = &all[start..end];
            match part.scope {
                LineScope::Person => groups.push((LineScope::Person, None, extract(self.lines, &body.join("\n")))),
                LineScope::Project => {
                    let (found, left) = self.project_groups(part, body);
                    groups.extend(found);
                    unmatched += left;
                }
            }
        }
        (groups, unmatched)
    }

    /// A project section split by its `by` headings, each group matched to the
    /// chain; a heading that names no folder inside a heading that does
    /// (a date under a project) belongs to that project.
    fn project_groups(&self, part: &PartDecl, body: &[&str]) -> (Vec<(LineScope, Option<usize>, Vec<Raw>)>, usize) {
        let levels: Vec<usize> = part.by.iter().map(String::len).collect();
        // The open headings, outermost first: (level, the distance it claims).
        let mut open: Vec<(usize, Option<usize>)> = Vec::new();
        let mut chunks: Vec<(Option<usize>, Vec<&str>)> = Vec::new();
        for line in body {
            let level = heading_level(line);
            if level > 0 && levels.contains(&level) {
                open.retain(|(l, _)| *l < level);
                let own = project_match(self.env, self.chain, &part.matching, line.trim_start_matches('#').trim());
                let inherited = open.last().and_then(|(_, d)| *d);
                open.push((level, own.or(inherited)));
                chunks.push((open.last().and_then(|(_, d)| *d), Vec::new()));
                continue;
            }
            match chunks.last_mut() {
                Some((_, lines)) => lines.push(line),
                None => chunks.push((None, vec![line])),
            }
        }
        let mut found = Vec::new();
        let mut unmatched = 0;
        for (distance, lines) in chunks {
            let raws = extract(self.lines, &lines.join("\n"));
            match distance {
                Some(_) => found.push((LineScope::Project, distance, raws)),
                None => unmatched += raws.len(),
            }
        }
        (found, unmatched)
    }
}

/// The text a loader would read: its first lines and bytes.
fn windowed(text: &str, lines: &LinesDecl) -> String {
    let max_lines = lines.read_first_lines.unwrap_or(usize::MAX);
    let max_bytes = lines.read_first_bytes.unwrap_or(usize::MAX);
    let mut used = 0;
    let mut out = Vec::new();
    for line in text.lines().take(max_lines) {
        used += line.len() + 1;
        if used > max_bytes {
            break;
        }
        out.push(line);
    }
    out.join("\n")
}

/// Whether a file has any line that is not blank and not a heading — so
/// that a file with lines, none of which matched, is told from an empty one.
fn has_content(text: &str) -> bool {
    text.lines().any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
}

fn check_expect(expect: &ExpectDecl, text: &str) -> Result<(), Skip> {
    if let Some(wanted) = &expect.first_line {
        let first = text.lines().next().unwrap_or("").trim();
        if first != wanted {
            return Err(Skip::UnknownFormat(format!("format not known: first line is '{}', expected '{wanted}'", shorten(first, 40))));
        }
    }
    if let Some(heading) = &expect.has_heading {
        if !text.lines().any(|l| l.trim() == heading) {
            return Err(Skip::UnknownFormat(format!("format not known: no '{heading}' heading")));
        }
    }
    Ok(())
}

/// The level of a Markdown heading line (`### x` is 3), 0 when it is none.
fn heading_level(line: &str) -> usize {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    match line[hashes..].starts_with(' ') {
        true => hashes,
        false => 0,
    }
}

/// The lines of the section that `under` opens: after it, to the next
/// heading of the same or a higher level.
fn section(all: &[&str], under: &str) -> Option<(usize, usize)> {
    let at = all.iter().position(|l| l.trim() == under.trim())?;
    let level = heading_level(under.trim());
    let end = all[at + 1..]
        .iter()
        .position(|l| (1..=level).contains(&heading_level(l)))
        .map_or(all.len(), |n| at + 1 + n);
    Some((at + 1, end))
}

/// Which folder of the chain a heading names, nearest first.
fn project_match(env: &Env, chain: &[PathBuf], by: &[MatchBy], heading: &str) -> Option<usize> {
    let heading = heading.trim().trim_matches('`');
    let tokens: Vec<&str> = heading
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '`'))
        .map(|t| t.trim_start_matches("cwd=").trim_end_matches('/'))
        .filter(|t| !t.is_empty())
        .collect();
    chain.iter().position(|dir| {
        let by_path = by.contains(&MatchBy::Path) && tokens.iter().any(|t| Path::new(t) == dir || *t == env.tilde(dir));
        let by_name = by.contains(&MatchBy::Name)
            && dir.file_name().and_then(|n| n.to_str()).is_some_and(|name| name.eq_ignore_ascii_case(heading));
        by_path || by_name
    })
}

/// The lines of `text` by the declared extractor.
fn extract(lines: &LinesDecl, text: &str) -> Vec<Raw> {
    match lines.each {
        Each::Item => text
            .lines()
            .filter(|l| lines.depth.is_none_or(|d| indent_of(l) == d))
            .map(str::trim)
            .filter(|l| l.starts_with("- ") || l.starts_with("* "))
            .map(|l| Raw { text: shorten(l, LINE_CHARS), link: lines.note_link.and_then(|_| linked_name(l)) })
            .collect(),
        Each::Text => text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with(char::is_whitespace) && heading_level(l) == 0)
            .map(|l| {
                let l = l.trim_end();
                let item = l.starts_with("- ") || l.starts_with("* ");
                Raw { text: shorten(&if item { l.to_string() } else { format!("- {l}") }, LINE_CHARS), link: None }
            })
            .collect(),
        Each::Heading => text
            .lines()
            .filter(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim())
            .filter(|h| !h.is_empty())
            .map(|h| Raw { text: shorten(&format!("- {h}"), LINE_CHARS), link: None })
            .collect(),
        Each::File => file_line(lines.text, text).into_iter().map(|t| Raw { text: shorten(&format!("- {t}"), LINE_CHARS), link: None }).collect(),
    }
}

/// A list item's indentation as a depth: two spaces or one tab per level.
fn indent_of(line: &str) -> usize {
    let lead: usize = line.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 2 } else { 1 }).sum();
    lead / 2
}

/// The note a bullet links to (`[title](file.md)`), when it is a plain file
/// name.
fn linked_name(line: &str) -> Option<String> {
    let open = line.find("](")? + 2;
    let target = line[open..].split(')').next()?;
    let plain = !target.is_empty()
        && !target.contains(['/', '\\', ':'])
        && !target.starts_with('.')
        && target.ends_with(".md");
    plain.then(|| target.to_string())
}

/// A whole file's one line: its frontmatter `description`, or its first line.
fn file_line(text: Option<FileText>, body: &str) -> Option<String> {
    let first = || {
        body.lines()
            .map(|l| l.trim().trim_start_matches('#').trim())
            .find(|l| !l.is_empty())
            .map(String::from)
    };
    match text? {
        FileText::FirstLine => first(),
        FileText::Description => frontmatter_description(body).or_else(first),
    }
}

fn frontmatter_description(body: &str) -> Option<String> {
    let (yaml, _) = crate::extensions::frontmatter::split(body);
    let value: serde_norway::Value = serde_norway::from_str(yaml?).ok()?;
    value.get("description")?.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
