//! The registry: every agent memory Linggen reads, declared as data.
//!
//! A reader is a declaration ([`super::decl`]) — one file per agent. Two
//! sets are merged by the agent's id:
//!
//! - the **embedded** readers (`readers/*.toml`, compiled in), the floor a
//!   fresh install or an offline Mac forwards from;
//! - the readers **installed skills** carry: a skill whose `SKILL.md` says
//!   `memory-reader: reader.toml` in `~/.linggen/skills/<name>/`. An installed
//!   reader replaces the embedded one of the same agent id, or adds a new
//!   agent. Only that folder counts: a project's `.linggen/skills/` and the
//!   other agents' skill folders can be written by another agent, and a
//!   reader chooses what Linggen reads, so they are never looked at.
//!
//! An installed reader that does not parse, breaks a rule, or says a newer
//! `format` is refused whole; the embedded one (if any) stays, and the
//! refusal is kept in [`Registry::problems`] for Settings.
//!
//! This is the one place that says which files may be read: [`Registry::files`]
//! is the union of every place's declared paths, and [`Registry::locate`]
//! turns a path a caller gave into a [`Located`] only when it is one of them.

use super::decl::{Declaration, Pattern, PlaceDecl, Refused};
use super::disk::{self, Expanded, Projects};
use super::Env;
use std::path::{Path, PathBuf};

/// A reader file compiled in. The embedded readers are the offline floor.
const EMBEDDED: [&str; 2] = [include_str!("readers/claude-code.toml"), include_str!("readers/codex.toml")];

/// A registry holds at most this many readable files in all.
const MAX_FILES: usize = 5000;

/// An agent whose memory Linggen reads, and a calling host that has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    id: String,
    label: String,
}

impl Agent {
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The agent's name in an address and in the page's tabs.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Where a reader came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Embedded,
    Skill(String),
}

/// One reader, checked.
#[derive(Clone, Debug)]
pub(crate) struct Reader {
    pub decl: Declaration,
    pub origin: Origin,
}

impl Reader {
    pub(crate) fn agent(&self) -> Agent {
        Agent { id: self.decl.agent.clone(), label: self.decl.label.clone() }
    }

    /// `agent/place`, the id of one place in an address and in Settings.
    pub(crate) fn source_id(&self, place: &PlaceDecl) -> String {
        format!("{}/{}", self.decl.agent, place.id)
    }
}

/// A reader that was refused, and why. Shown in Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The skill (or `embedded`) whose reader it was.
    pub reader: String,
    pub why: String,
}

/// A note file inside a declared place. Only the registry makes one.
#[derive(Clone, Debug)]
pub(crate) struct Located {
    path: PathBuf,
    source: String,
    agent: Agent,
}

impl Located {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn source(&self) -> &str {
        &self.source
    }

    pub(super) fn agent(&self) -> &Agent {
        &self.agent
    }
}

/// The readers in force: the embedded ones, with the installed ones merged in.
pub(crate) struct Registry {
    home: PathBuf,
    pub readers: Vec<Reader>,
    pub problems: Vec<Problem>,
}

impl Registry {
    /// The readers of `env`'s machine: embedded, then installed skills'.
    pub(crate) fn load(env: &Env) -> Registry {
        let mut registry = Registry { home: env.home().to_path_buf(), readers: Vec::new(), problems: Vec::new() };
        for text in EMBEDDED {
            match Declaration::parse(text) {
                Ok(decl) => registry.readers.push(Reader { decl, origin: Origin::Embedded }),
                Err(why) => registry.problems.push(Problem { reader: "embedded".into(), why: why.to_string() }),
            }
        }
        registry.merge_installed();
        registry
    }

    fn merge_installed(&mut self) {
        let skills_dir = self.home.join(".linggen").join("skills");
        for skill in disk::skill_folders(&skills_dir) {
            let Some(file) = skill.head.as_deref().and_then(declared_reader_file) else {
                continue;
            };
            let accepted = disk::reader_text(&skill.dir, &file)
                .map_err(Refused::Invalid)
                .and_then(|text| Declaration::parse(&text));
            match accepted {
                Ok(decl) => self.put(Reader { decl, origin: Origin::Skill(skill.name) }),
                Err(why) => self.problems.push(Problem { reader: skill.name, why: why.to_string() }),
            }
        }
    }

    /// An installed reader replaces the one of its agent, or adds one. A
    /// second installed reader for the same agent is refused.
    fn put(&mut self, reader: Reader) {
        let same = self.readers.iter().position(|r| r.decl.agent == reader.decl.agent);
        match same {
            Some(at) if self.readers[at].origin == Origin::Embedded => self.readers[at] = reader,
            Some(_) => {
                let reader_name = match &reader.origin {
                    Origin::Skill(name) => name.clone(),
                    Origin::Embedded => "embedded".into(),
                };
                let why = format!("agent `{}` is already declared by another skill", reader.decl.agent);
                self.problems.push(Problem { reader: reader_name, why });
            }
            None => self.readers.push(reader),
        }
    }

    /// The agent a calling host's name stands for; `None` for a host with no
    /// memory Linggen reads (it is forwarded everyone's).
    pub(crate) fn agent_of_host(&self, host: &str) -> Option<Agent> {
        let host = host.trim().to_ascii_lowercase();
        self.readers
            .iter()
            .find(|r| r.decl.agent == host || r.decl.hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)))
            .map(Reader::agent)
    }

    /// Every file that may be read or searched: the union of the places'
    /// declared paths, a file once, under the first place that names it.
    pub(crate) fn files(&self) -> Vec<Located> {
        let mut out: Vec<Located> = Vec::new();
        for reader in &self.readers {
            for place in &reader.decl.places {
                for (file, _) in self.place_files(reader, place, &Projects::Any) {
                    let seen = out.iter().any(|l| l.path == file.path);
                    if !seen && out.len() < MAX_FILES {
                        out.push(file);
                    }
                }
            }
        }
        out
    }

    /// The files of one place for `projects`, each with the chain distance of
    /// the folder `{project}` was, when it was one.
    pub(crate) fn place_files(&self, reader: &Reader, place: &PlaceDecl, projects: &Projects) -> Vec<(Located, Option<usize>)> {
        let mut found = Vec::new();
        for text in place.path_texts() {
            // A declaration that was accepted has paths that parse.
            let Ok(pattern) = Pattern::parse(text, place.project.is_some()) else {
                continue;
            };
            for Expanded { path, distance } in disk::expand(&self.home, &pattern, projects) {
                found.push((Located { path, source: reader.source_id(place), agent: reader.agent() }, distance));
            }
        }
        found
    }

    /// `wanted` as a [`Located`], when — once links and `..` are resolved —
    /// it is a note file one of the declared places names.
    pub(crate) fn locate(&self, wanted: &Path) -> Option<Located> {
        let real = disk::canonical(wanted)?;
        if !real.is_file() {
            return None;
        }
        self.files().into_iter().find_map(|file| {
            let same_name = file.path.file_name() == real.file_name();
            let same_file = same_name && disk::canonical(&file.path).is_some_and(|p| p == real);
            same_file.then(|| Located { path: real.clone(), ..file })
        })
    }
}

/// The value of `memory-reader:` in a `SKILL.md`'s frontmatter.
fn declared_reader_file(head: &str) -> Option<String> {
    let (yaml, _) = crate::extensions::frontmatter::split(head);
    let value: serde_norway::Value = serde_norway::from_str(yaml?).ok()?;
    value.get("memory-reader")?.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
