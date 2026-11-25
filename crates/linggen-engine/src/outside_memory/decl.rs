//! A reader declaration: the TOML that says where one agent keeps its
//! memory and how its files become index lines. Parsed and checked here and
//! nowhere else; what a declaration may name is decided by [`Pattern::parse`]
//! and [`Declaration::check`] — a reader that breaks a rule is refused whole,
//! never partly applied.
//!
//! The declaration language is closed: the primitives (path variables, line
//! extractors, scope rules, version guards) are what this file lists, and a
//! format that needs another is an engine release. Agents and formats are
//! open data (design: memory-readers § 3).

use serde::Deserialize;
use std::fmt;

/// The declaration language version this engine understands.
pub(crate) const FORMAT: u32 = 1;

/// Caps that keep a declaration, and what it makes the engine read, small.
const MAX_PLACES: usize = 16;
const MAX_PATHS: usize = 8;
const MAX_SEGMENTS: usize = 10;
const MAX_LINES_CAP: usize = 1000;
const MAX_BYTES_CAP: usize = 256 * 1024;
const NOTE_EXTENSIONS: [&str; 3] = [".md", ".markdown", ".txt"];

/// Why a declaration was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// It says a newer declaration language than this engine knows.
    NeedsNewerLinggen,
    Invalid(String),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refused::NeedsNewerLinggen => f.write_str("needs a newer Linggen"),
            Refused::Invalid(why) => f.write_str(why),
        }
    }
}

fn invalid<T>(why: impl Into<String>) -> Result<T, Refused> {
    Err(Refused::Invalid(why.into()))
}

/// A whole reader file.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Declaration {
    pub format: u32,
    /// The id used in addresses and tabs (`claude-code`).
    pub agent: String,
    pub label: String,
    /// Words a calling host answers to; a host never gets its own agent's
    /// memory back.
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default, rename = "place")]
    pub places: Vec<PlaceDecl>,
}

/// One kind of file an agent keeps.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlaceDecl {
    pub id: String,
    pub path: OneOrMany,
    /// How `{project}` is written in a folder name (`dash`: Claude Code's).
    #[serde(default)]
    pub project: Option<ProjectEncoding>,
    #[serde(default)]
    pub scope: Scope,
    /// `false`: opened and searched, never listed.
    #[serde(default = "yes")]
    pub forward: bool,
    #[serde(default)]
    pub lines: Option<LinesDecl>,
    #[serde(default)]
    pub parts: Vec<PartDecl>,
    #[serde(default)]
    pub expect: Option<ExpectDecl>,
    /// Places with the same group: only the one whose files changed last is
    /// forwarded (an agent's old folder and its new one).
    #[serde(default)]
    pub group: Option<String>,
}

fn yes() -> bool {
    true
}

impl PlaceDecl {
    /// The place's path patterns, as written.
    pub(crate) fn path_texts(&self) -> Vec<&str> {
        self.path.all()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub(crate) fn all(&self) -> Vec<&str> {
        match self {
            OneOrMany::One(one) => vec![one.as_str()],
            OneOrMany::Many(many) => many.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProjectEncoding {
    Dash,
}

/// Whose memory a line is: the person's (forwarded in every folder) or a
/// project's (only in that project's chain).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Scope {
    #[default]
    Person,
    Project,
}

/// How a file becomes lines.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinesDecl {
    pub each: Each,
    /// For `item`: only list items indented this much (absent: any depth).
    #[serde(default)]
    pub depth: Option<usize>,
    /// At most this many lines from one file.
    #[serde(default)]
    pub first: Option<usize>,
    /// The vendor loads only the first N lines of a file: so does the reader.
    #[serde(default)]
    pub read_first_lines: Option<usize>,
    /// ... or the first N bytes.
    #[serde(default)]
    pub read_first_bytes: Option<usize>,
    /// For `file`: only the newest N files.
    #[serde(default)]
    pub newest: Option<usize>,
    /// For `file`: what the file's one line says.
    #[serde(default)]
    pub text: Option<FileText>,
    /// A list item that links `[t](file.md)` takes that note's change time.
    #[serde(default)]
    pub note_link: Option<NoteLink>,
}

/// The line extractors. Closed.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Each {
    /// A list item (`- ` or `* `), at `depth` when given.
    Item,
    /// Any line that is not blank, not a heading and not indented: a list
    /// item at the left edge or a plain paragraph line.
    Text,
    /// A heading's words.
    Heading,
    /// One line per file.
    File,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum FileText {
    #[serde(rename = "{first_line}")]
    FirstLine,
    #[serde(rename = "{description}")]
    Description,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NoteLink {
    Markdown,
}

/// A section of a file, with the scope its lines have.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartDecl {
    /// The heading that opens the section (`## User Profile`); it runs to the
    /// next heading of the same or a higher level.
    pub under: String,
    pub scope: Scope,
    /// For a project section: the heading levels (`###`, `####`) whose words
    /// name a project; matched to the session's chain by `match`.
    #[serde(default)]
    pub by: Vec<String>,
    #[serde(default, rename = "match")]
    pub matching: Vec<MatchBy>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MatchBy {
    Path,
    Name,
}

/// The vendor's own version guard.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectDecl {
    #[serde(default)]
    pub first_line: Option<String>,
    #[serde(default)]
    pub has_heading: Option<String>,
}

/// One path segment of a place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    Literal(String),
    /// A name with `*` in it (never matching a name that starts with a dot).
    Wild(String),
    /// One folder of the session's chain (`{project}`).
    Project,
}

/// A place's path, under the home folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pattern {
    pub segments: Vec<Segment>,
}

impl Pattern {
    /// `~/a/*/b.md`. Refused: anything not under the home folder, `..`,
    /// `~/.linggen` (Linggen's own store), a dot folder behind a `*`, a last
    /// segment that is not a note file, `{project}` where none is declared.
    pub(crate) fn parse(text: &str, has_project: bool) -> Result<Pattern, Refused> {
        let Some(rest) = text.strip_prefix("~/") else {
            return invalid(format!("path `{text}` must start with ~/"));
        };
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() > MAX_SEGMENTS {
            return invalid(format!("path `{text}` is too deep"));
        }
        if parts.first() == Some(&".linggen") {
            return invalid(format!("path `{text}` is inside Linggen's own store"));
        }
        let mut segments = Vec::new();
        for part in &parts {
            segments.push(Self::segment(part, text, has_project)?);
        }
        let last_is_note = match segments.last() {
            Some(Segment::Literal(name) | Segment::Wild(name)) => NOTE_EXTENSIONS.iter().any(|e| name.ends_with(e)),
            _ => false,
        };
        if !last_is_note {
            return invalid(format!("path `{text}` must end in a .md, .markdown or .txt file"));
        }
        Ok(Pattern { segments })
    }

    fn segment(part: &str, whole: &str, has_project: bool) -> Result<Segment, Refused> {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', '\0']) {
            return invalid(format!("path `{whole}` has a segment `{part}`"));
        }
        if part == "{project}" {
            return match has_project {
                true => Ok(Segment::Project),
                false => invalid(format!("path `{whole}` uses {{project}} but the place declares no `project`")),
            };
        }
        if part.contains('{') || part.contains('}') {
            return invalid(format!("path `{whole}`: only a whole `{{project}}` segment is a variable"));
        }
        if part.contains("**") {
            return invalid(format!("path `{whole}`: `**` is not supported"));
        }
        if !part.contains('*') {
            return Ok(Segment::Literal(part.to_string()));
        }
        if part.starts_with('.') {
            return invalid(format!("path `{whole}`: a dot folder is never matched by a `*`"));
        }
        Ok(Segment::Wild(part.to_string()))
    }
}

/// Whether `name` matches a segment's `*` pattern.
pub(crate) fn wild_matches(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    let pieces: Vec<&str> = pattern.split('*').collect();
    let (first, last) = (pieces[0], pieces[pieces.len() - 1]);
    if !name.starts_with(first) || name.len() < first.len() + last.len() || !name.ends_with(last) {
        return false;
    }
    let mut at = first.len();
    let end = name.len() - last.len();
    for middle in &pieces[1..pieces.len() - 1] {
        match name[at..end].find(middle) {
            Some(found) => at += found + middle.len(),
            None => return false,
        }
    }
    true
}

impl Declaration {
    /// Parse a reader file's text. A newer `format` is refused before the
    /// rest is looked at, so a future field is never mistaken for a typo.
    pub(crate) fn parse(text: &str) -> Result<Declaration, Refused> {
        let table: toml::Table = text.parse().map_err(|e: toml::de::Error| Refused::Invalid(format!("not valid TOML: {}", e.message())))?;
        match table.get("format").and_then(toml::Value::as_integer) {
            Some(n) if n > i64::from(FORMAT) => return Err(Refused::NeedsNewerLinggen),
            Some(n) if n == i64::from(FORMAT) => {}
            _ => return invalid(format!("`format` must be {FORMAT}")),
        }
        let decl: Declaration = table.try_into().map_err(|e: toml::de::Error| Refused::Invalid(e.message().to_string()))?;
        decl.check()?;
        Ok(decl)
    }

    fn check(&self) -> Result<(), Refused> {
        if self.format != FORMAT {
            return invalid(format!("`format` must be {FORMAT}"));
        }
        let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !word(&self.agent) {
            return invalid(format!("agent id `{}` must be letters, digits, - or _", self.agent));
        }
        if self.label.trim().is_empty() {
            return invalid("`label` is empty");
        }
        if self.places.len() > MAX_PLACES {
            return invalid("too many places");
        }
        let mut seen: Vec<&str> = Vec::new();
        for place in &self.places {
            if !word(&place.id) || seen.contains(&place.id.as_str()) {
                return invalid(format!("place id `{}` is empty, odd or repeated", place.id));
            }
            seen.push(&place.id);
            place.check()?;
        }
        Ok(())
    }
}

impl PlaceDecl {
    fn check(&self) -> Result<(), Refused> {
        let paths = self.path.all();
        if paths.is_empty() || paths.len() > MAX_PATHS {
            return invalid(format!("place `{}` has no path or too many", self.id));
        }
        for path in paths {
            Pattern::parse(path, self.project.is_some())?;
        }
        if let Some(lines) = &self.lines {
            lines.check(&self.id)?;
        }
        for part in &self.parts {
            if !part.under.starts_with('#') {
                return invalid(format!("place `{}`: `under` must be a heading like `## Name`", self.id));
            }
            if part.scope == Scope::Project && (part.by.is_empty() || part.matching.is_empty()) {
                return invalid(format!("place `{}`: a project part names `by` heading levels and a `match`", self.id));
            }
            if part.by.iter().any(|h| h.is_empty() || !h.chars().all(|c| c == '#')) {
                return invalid(format!("place `{}`: `by` holds heading levels such as `###`", self.id));
            }
        }
        if let Some(expect) = &self.expect {
            if expect.first_line.is_some() == expect.has_heading.is_some() {
                return invalid(format!("place `{}`: `expect` holds exactly one of first_line, has_heading", self.id));
            }
        }
        Ok(())
    }
}

impl LinesDecl {
    fn check(&self, place: &str) -> Result<(), Refused> {
        for (name, value, cap) in [
            ("first", self.first, MAX_LINES_CAP),
            ("read_first_lines", self.read_first_lines, MAX_LINES_CAP),
            ("newest", self.newest, MAX_LINES_CAP),
            ("read_first_bytes", self.read_first_bytes, MAX_BYTES_CAP),
        ] {
            if value.is_some_and(|v| v == 0 || v > cap) {
                return invalid(format!("place `{place}`: `{name}` must be 1..={cap}"));
            }
        }
        if self.each == Each::File && self.text.is_none() {
            return invalid(format!("place `{place}`: `each = \"file\"` needs `text`"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wild_segment_never_matches_a_dot_name() {
        assert!(wild_matches("*.md", "a.md"));
        assert!(wild_matches("a*c*e.md", "abcde.md"));
        assert!(!wild_matches("*.md", ".hidden.md"));
        assert!(!wild_matches("*.md", "a.txt"));
        assert!(!wild_matches("ab*ba", "aba"));
        assert!(wild_matches("*", "anything"));
    }

    #[test]
    fn paths_that_reach_outside_the_home_or_into_lingens_store_are_refused() {
        for bad in [
            "/etc/notes.md",
            "~/../x.md",
            "~/a/../x.md",
            "~/.linggen/memory/person/x.md",
            "~/a/.*/x.md",
            "~/a/x.json",
            "~/a/**/x.md",
            "~/a/{project}/x.md",
            "~/a/pre{project}/x.md",
            "~//x.md",
            "~/a/b",
        ] {
            assert!(Pattern::parse(bad, false).is_err(), "{bad} was accepted");
        }
        assert!(Pattern::parse("~/.agent/projects/{project}/memory/*.md", true).is_ok());
        assert!(Pattern::parse("~/.agent/notes.txt", false).is_ok());
    }
}
