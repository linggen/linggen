//! The folder `$LINGGEN_HOME/memory/`: where a topic's file is, how it is
//! loaded, saved (atomically, the old version kept) and split, and the local
//! use counts. Nothing but [`super::write`] saves; nothing but
//! [`super::fence`] loads for a caller (`tests::only_the_fence_loads_and_only_write_saves`).

use super::format::{Scope, Topic};
use crate::outside_memory::{encode_project_path, Env};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const STATE: &str = ".state";
/// The folders topic files live in. The rest of `memory/` is not these
/// notes' (the v1 store keeps its backups and database there): never walked,
/// never read.
const TOPIC_DIRS: [&str; 5] = ["person", "areas", "projects", "moments", "chronicle"];
/// Prior versions kept for each file.
const HISTORY_KEPT: usize = 20;
const SLUG_CHARS: usize = 60;

/// Linggen's own memory on this machine.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    env: Env,
}

/// A topic file as it is on disk now.
#[derive(Clone, Debug)]
pub(crate) struct Loaded {
    /// Path inside the store, with `/`: `person/diet.md`.
    pub rel: String,
    /// What the file's text hashes to; a writer says which version it saw.
    pub version: String,
    pub topic: Topic,
}

impl Store {
    /// The store under `memory_root`; `env` is the person's home, where a
    /// project chain stops.
    pub fn new(memory_root: impl Into<PathBuf>, env: Env) -> Store {
        Store { root: memory_root.into(), env }
    }

    /// This machine's: `$LINGGEN_HOME/memory`.
    pub fn this_machine() -> Option<Store> {
        Env::this_machine().map(|env| Store::new(crate::paths::linggen_home().join("memory"), env))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Every topic file, as loaded; a file that cannot be read or parsed is
    /// named in the second list, never fatal.
    pub(super) fn load_all_unfiltered(&self) -> (Vec<Loaded>, Vec<String>) {
        let (mut loaded, mut problems) = (Vec::new(), Vec::new());
        for rel in self.topic_files() {
            match self.load(&rel) {
                Ok(Some(file)) => loaded.push(file),
                Ok(None) => {}
                Err(why) => problems.push(format!("{rel}: {why}")),
            }
        }
        (loaded, problems)
    }

    /// One file by its path in the store; `None` when it is not there.
    pub(super) fn load(&self, rel: &str) -> Result<Option<Loaded>, String> {
        let path = self.root.join(rel);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.kind().to_string()),
        };
        let topic = Topic::parse(&text).map_err(|e| e.to_string())?;
        Ok(Some(Loaded { rel: rel.to_string(), version: version_of(&text), topic }))
    }

    /// Paths (with `/`) of every `.md` under the topic folders, sorted.
    fn topic_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        for dir in TOPIC_DIRS {
            collect(&self.root.join(dir), dir, &mut out);
        }
        out.sort();
        out
    }

    /// Whether `rel` is inside one of the topic folders.
    pub(super) fn is_topic_path(rel: &str) -> bool {
        TOPIC_DIRS.iter().any(|dir| rel.starts_with(&format!("{dir}/")))
    }

    /// Where a topic of `scope` named `slug` lives.
    pub(super) fn rel_of(scope: &Scope, slug: &str) -> String {
        match scope {
            Scope::Person => format!("person/{slug}.md"),
            Scope::Area(a) => format!("areas/{}/{slug}.md", a.word()),
            Scope::Project(p) => format!("projects/{}/{slug}.md", encode_project_path(p)),
            Scope::Moments => by_year("moments", slug),
            Scope::Chronicle => by_year("chronicle", slug),
        }
    }

    /// Save `topic` as `rel`, the old file kept under `.state/history`, then
    /// split past the limits: the oldest entries move to `<slug>-2.md`, and
    /// on. Atomic per file (temp file, then rename).
    pub(super) fn save(&self, rel: &str, topic: Topic) -> std::io::Result<()> {
        let mut topic = topic;
        let mut spilled = Vec::new();
        while topic.is_over_limits() && topic.entries.len() > 1 {
            spilled.push(topic.entries.remove(0));
        }
        self.put(rel, &topic)?;
        if spilled.is_empty() {
            return Ok(());
        }
        let next_rel = Store::rel_of(&topic.scope, &next_slug(&slug_of_rel(rel)));
        let mut next = match self.load(&next_rel) {
            Ok(Some(file)) => file.topic,
            _ => Topic::new(&format!("{} (continued)", topic.name), topic.scope.clone()),
        };
        spilled.append(&mut next.entries);
        next.entries = spilled;
        self.save(&next_rel, next)
    }

    fn put(&self, rel: &str, topic: &Topic) -> std::io::Result<()> {
        let path = self.root.join(rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        if let Ok(old) = fs::read(&path) {
            self.keep_history(rel, &old)?;
        }
        let temp = path.with_extension("md.tmp");
        let mut file = fs::File::create(&temp)?;
        file.write_all(topic.render().as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &path)
    }

    /// Take the file `rel` away, its last version kept in the history.
    pub(super) fn remove(&self, rel: &str) -> std::io::Result<()> {
        let path = self.root.join(rel);
        if let Ok(old) = fs::read(&path) {
            self.keep_history(rel, &old)?;
        }
        fs::remove_file(&path)
    }

    fn history_dir(&self, rel: &str) -> PathBuf {
        self.root.join(STATE).join("history").join(rel.replace('/', "__"))
    }

    fn keep_history(&self, rel: &str, old: &[u8]) -> std::io::Result<()> {
        let dir = self.history_dir(rel);
        fs::create_dir_all(&dir)?;
        let name = format!("{}.md", version_of(&String::from_utf8_lossy(old)));
        fs::write(dir.join(name), old)?;
        let mut kept: Vec<_> = fs::read_dir(&dir)?
            .flatten()
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        kept.sort();
        let extra = kept.len().saturating_sub(HISTORY_KEPT);
        kept.iter().take(extra).for_each(|(_, p)| {
            let _ = fs::remove_file(p);
        });
        Ok(())
    }

    /// The file `rel` as it was at `version`, if it was kept.
    pub(super) fn at_version(&self, rel: &str, version: &str) -> Option<Topic> {
        let text = fs::read_to_string(self.history_dir(rel).join(format!("{version}.md"))).ok()?;
        Topic::parse(&text).ok()
    }

    fn uses_file(&self) -> PathBuf {
        self.root.join(STATE).join("uses.jsonl")
    }

    /// Note that `ids` were read, now. Local only; never synced.
    pub(super) fn record_uses(&self, ids: &[&str], now: i64) {
        if ids.is_empty() {
            return;
        }
        let path = self.uses_file();
        let lines: String = ids
            .iter()
            .map(|id| format!("{}\n", serde_json::json!({"id": id, "at": now})))
            .collect();
        let write = || -> std::io::Result<()> {
            fs::create_dir_all(path.parent().expect("a state folder"))?;
            fs::OpenOptions::new().create(true).append(true).open(&path)?.write_all(lines.as_bytes())
        };
        if let Err(e) = write() {
            tracing::debug!("memory uses not recorded: {e}");
        }
    }

    /// How many times each id was read at or after `since` (epoch seconds).
    pub(super) fn uses_since(&self, since: i64) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        let Ok(text) = fs::read_to_string(self.uses_file()) else {
            return counts;
        };
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let (Some(id), Some(at)) = (v["id"].as_str(), v["at"].as_i64()) else {
                continue;
            };
            if at >= since {
                *counts.entry(id.to_string()).or_insert(0) += 1;
            }
        }
        counts
    }
}

fn collect(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && name != STATE {
            collect(&entry.path(), &child, out);
        } else if kind.is_file() && name.ends_with(".md") {
            out.push(child);
        }
    }
}

/// A short hash of a file's text: its version.
pub(crate) fn version_of(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// A topic name as a file name: letters and digits of any language, lower
/// case, other runs one `-`.
pub(crate) fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(SLUG_CHARS).collect();
    match out.is_empty() {
        true => "notes".to_string(),
        false => out,
    }
}

/// A file named for its month (or year), in that year's folder.
fn by_year(dir: &str, slug: &str) -> String {
    let year: String = slug.chars().take(4).collect();
    format!("{dir}/{year}/{slug}.md")
}

fn slug_of_rel(rel: &str) -> String {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    file.strip_suffix(".md").unwrap_or(file).to_string()
}

/// `diet` -> `diet-part2` -> `diet-part3` (never a bare number: a month's
/// file is `2026-11`).
fn next_slug(slug: &str) -> String {
    match slug.rsplit_once("-part") {
        Some((base, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
            format!("{base}-part{}", n.parse::<u32>().unwrap_or(1) + 1)
        }
        _ => format!("{slug}-part2"),
    }
}
