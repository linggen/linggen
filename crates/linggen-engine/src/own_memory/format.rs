//! The topic file: front matter, then one entry per `## summary` heading,
//! each with a meta comment and a body. Plain Markdown a person can read and
//! a phone can write. Build spec: memory-v2 design § 4a.
//!
//! The only place that knows the file's text form. `parse(render(x)) == x`
//! for anything [`Entry::new`] made (`tests::a_topic_survives_the_file_form`).

use std::path::{Path, PathBuf};

/// A summary line, in characters (not bytes: Chinese counts one each).
pub const SUMMARY_CHARS: usize = 80;
/// An entry's body, in characters.
pub const BODY_CHARS: usize = 4000;
/// A topic file holds fewer lines than this ...
pub const FILE_LINES: usize = 200;
/// ... and at most this many bytes (Claude Code's own limits).
pub const FILE_BYTES: usize = 16 * 1024;

const SEP: &str = " · ";

/// What an entry is. Closed: the writer refuses any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Person,
    Rule,
    Decision,
    Gotcha,
    /// A dated life moment, Yinyue's: what happened, how it felt, what was concluded.
    Moment,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Person, Kind::Rule, Kind::Decision, Kind::Gotcha, Kind::Moment];

    pub fn word(self) -> &'static str {
        match self {
            Kind::Person => "person",
            Kind::Rule => "rule",
            Kind::Decision => "decision",
            Kind::Gotcha => "gotcha",
            Kind::Moment => "moment",
        }
    }

    pub fn parse(word: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.word() == word.trim())
    }
}

/// Whose words an entry holds. A `User` entry changes only when the person
/// directs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Voice {
    User,
    Agent,
}

impl Voice {
    pub fn word(self) -> &'static str {
        match self {
            Voice::User => "user",
            Voice::Agent => "agent",
        }
    }

    pub fn parse(word: &str) -> Option<Voice> {
        match word.trim() {
            "user" => Some(Voice::User),
            "agent" => Some(Voice::Agent),
            _ => None,
        }
    }
}

/// A part of the person's life Yinyue keeps notes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Body,
    Mind,
    People,
    Music,
    Money,
    Play,
    Home,
}

impl Area {
    pub const ALL: [Area; 7] =
        [Area::Body, Area::Mind, Area::People, Area::Music, Area::Money, Area::Play, Area::Home];

    pub fn word(self) -> &'static str {
        match self {
            Area::Body => "body",
            Area::Mind => "mind",
            Area::People => "people",
            Area::Music => "music",
            Area::Money => "money",
            Area::Play => "play",
            Area::Home => "home",
        }
    }

    pub fn parse(word: &str) -> Option<Area> {
        Area::ALL.into_iter().find(|a| a.word() == word.trim())
    }
}

/// What a topic is about; also which folder it lives in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Person,
    Area(Area),
    Project(PathBuf),
    /// Yinyue's dated life moments, one file a month.
    Moments,
}

impl Scope {
    /// `person`, `area:body`, `project:/abs/path`, `moments`.
    pub fn parse(text: &str) -> Option<Scope> {
        let text = text.trim();
        match text {
            "person" => return Some(Scope::Person),
            "moments" => return Some(Scope::Moments),
            _ => {}
        }
        if let Some(area) = text.strip_prefix("area:") {
            return Area::parse(area).map(Scope::Area);
        }
        let path = text.strip_prefix("project:")?.trim();
        Path::new(path).is_absolute().then(|| Scope::Project(PathBuf::from(path)))
    }

    pub fn word(&self) -> String {
        match self {
            Scope::Person => "person".to_string(),
            Scope::Area(a) => format!("area:{}", a.word()),
            Scope::Project(p) => format!("project:{}", p.display()),
            Scope::Moments => "moments".to_string(),
        }
    }
}

/// One note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub summary: String,
    pub kind: Kind,
    pub voice: Voice,
    pub created: String,
    pub updated: String,
    /// Who wrote it: `claude-code`, `codex`, `linggen`, `yinyue`, `phone`, …
    pub source: String,
    /// Known wrong, and why. Shown only in Settings; never in an index or a search.
    pub void: Option<String>,
    /// Yinyue never brings it up herself.
    pub no_raise: bool,
    pub body: String,
    /// Meta keys this build does not know, kept so another writer's survive.
    pub extra: Vec<(String, String)>,
}

impl Entry {
    /// A new entry with its text made safe for the file form: one-line
    /// summary within [`SUMMARY_CHARS`]; body headings that would start an
    /// entry written one level deeper.
    pub fn new(
        id: String,
        summary: &str,
        kind: Kind,
        voice: Voice,
        today: &str,
        source: &str,
        body: &str,
    ) -> Entry {
        Entry {
            id,
            summary: one_line(summary, SUMMARY_CHARS),
            kind,
            voice,
            created: today.to_string(),
            updated: today.to_string(),
            source: one_line(source, 40),
            void: None,
            no_raise: false,
            body: safe_body(body),
            extra: Vec::new(),
        }
    }

    fn meta(&self) -> String {
        let mut parts = vec![
            format!("id: {}", self.id),
            format!("kind: {}", self.kind.word()),
            format!("voice: {}", self.voice.word()),
            format!("created: {}", self.created),
            format!("updated: {}", self.updated),
            format!("source: {}", self.source),
        ];
        if let Some(why) = &self.void {
            parts.push(format!("void: {}", meta_text(why)));
        }
        if self.no_raise {
            parts.push("no-raise".to_string());
        }
        parts.extend(self.extra.iter().map(|(k, v)| format!("{k}: {v}")));
        format!("<!-- {} -->", parts.join(SEP))
    }
}

/// A void reason or any meta value: one line, no separator, no comment end.
pub fn meta_text(text: &str) -> String {
    one_line(text, 200).replace(SEP, " - ").replace("-->", "->")
}

/// `text` on one line, cut to `max` characters.
pub fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(max).collect()
}

/// A body whose lines can never start an entry (`## `): they are written one
/// level deeper. Trailing space and blank edges go.
pub fn safe_body(body: &str) -> String {
    let lines: Vec<String> = body
        .trim()
        .lines()
        .map(|l| {
            let l = l.trim_end();
            match l == "##" || l.starts_with("## ") {
                true => format!("#{l}"),
                false => l.to_string(),
            }
        })
        .collect();
    lines.join("\n")
}

/// One topic file: a name, what it is about, and its entries in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topic {
    pub name: String,
    pub scope: Scope,
    pub entries: Vec<Entry>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FormatError(pub String);

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Topic {
    pub fn new(name: &str, scope: Scope) -> Topic {
        Topic { name: one_line(name, 80), scope, entries: Vec::new() }
    }

    /// The file's text.
    pub fn render(&self) -> String {
        let mut out = format!("---\ntopic: {}\nscope: {}\n---\n", self.name, self.scope.word());
        for e in &self.entries {
            out.push_str(&format!("\n## {}\n{}\n", e.summary, e.meta()));
            if !e.body.is_empty() {
                out.push_str(&e.body);
                out.push('\n');
            }
        }
        out
    }

    /// Whether the rendered file is past the limits (fewer than
    /// [`FILE_LINES`] lines, at most [`FILE_BYTES`] bytes).
    pub fn is_over_limits(&self) -> bool {
        let text = self.render();
        text.lines().count() >= FILE_LINES || text.len() > FILE_BYTES
    }

    pub fn parse(text: &str) -> Result<Topic, FormatError> {
        let mut lines = text.lines().enumerate().peekable();
        let bad = |n: usize, why: &str| FormatError(format!("line {}: {why}", n + 1));
        match lines.next() {
            Some((_, "---")) => {}
            _ => return Err(bad(0, "a topic file starts with ---")),
        }
        let (mut name, mut scope) = (None, None);
        loop {
            let Some((n, line)) = lines.next() else {
                return Err(bad(0, "the front matter never ends"));
            };
            if line == "---" {
                break;
            }
            if let Some(v) = line.strip_prefix("topic:") {
                name = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("scope:") {
                scope = Some(Scope::parse(v).ok_or_else(|| bad(n, "unknown scope"))?);
            }
        }
        let name = name.ok_or_else(|| bad(0, "no topic name"))?;
        let scope = scope.ok_or_else(|| bad(0, "no scope"))?;
        let mut entries: Vec<Entry> = Vec::new();
        let mut body: Vec<&str> = Vec::new();
        for (n, line) in lines {
            if let Some(summary) = line.strip_prefix("## ") {
                finish_body(entries.last_mut(), &mut body);
                entries.push(Entry {
                    id: String::new(),
                    summary: summary.trim().to_string(),
                    kind: Kind::Rule,
                    voice: Voice::Agent,
                    created: String::new(),
                    updated: String::new(),
                    source: String::new(),
                    void: None,
                    no_raise: false,
                    body: String::new(),
                    extra: Vec::new(),
                });
                continue;
            }
            let Some(entry) = entries.last_mut() else {
                if line.trim().is_empty() {
                    continue;
                }
                return Err(bad(n, "text before the first entry"));
            };
            if entry.id.is_empty() {
                if line.trim().is_empty() {
                    continue;
                }
                read_meta(entry, line).map_err(|why| bad(n, &why))?;
                continue;
            }
            body.push(line);
        }
        finish_body(entries.last_mut(), &mut body);
        if let Some(e) = entries.iter().find(|e| e.id.is_empty()) {
            return Err(FormatError(format!("entry \"{}\" has no meta comment", e.summary)));
        }
        Ok(Topic { name, scope, entries })
    }
}

fn finish_body(entry: Option<&mut Entry>, body: &mut Vec<&str>) {
    if let Some(e) = entry {
        e.body = body.join("\n").trim().to_string();
    }
    body.clear();
}

fn read_meta(entry: &mut Entry, line: &str) -> Result<(), String> {
    let inner = line
        .trim()
        .strip_prefix("<!--")
        .and_then(|l| l.strip_suffix("-->"))
        .ok_or("the line after a heading is its meta comment")?;
    for part in inner.trim().split(SEP) {
        let part = part.trim();
        if part == "no-raise" {
            entry.no_raise = true;
            continue;
        }
        let (key, value) = part.split_once(':').ok_or("a meta part is `key: value`")?;
        let value = value.trim().to_string();
        match key.trim() {
            "id" => entry.id = value,
            "kind" => entry.kind = Kind::parse(&value).ok_or("unknown kind")?,
            "voice" => entry.voice = Voice::parse(&value).ok_or("unknown voice")?,
            "created" => entry.created = value,
            "updated" => entry.updated = value,
            "source" => entry.source = value,
            "void" => entry.void = Some(value),
            other => entry.extra.push((other.to_string(), value)),
        }
    }
    match entry.id.is_empty() {
        true => Err("the meta comment has no id".to_string()),
        false => Ok(()),
    }
}
