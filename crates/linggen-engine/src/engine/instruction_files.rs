//! Discovery, expansion and change detection for native instruction files.

use super::permission::{self, DeclinedScopes, PermissionMode, ReadOrigin, SessionPermissions};
use super::AgentEngine;
use pulldown_cmark::{Event, Parser, Tag};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) const AGENTS: &str = "AGENTS.md";
pub(super) const CLAUDE: &str = "CLAUDE.md";
const LOCAL: &str = "CLAUDE.local.md";
pub(super) const CURSOR: &str = ".cursorrules";
const NAMES: &[&str] = &[AGENTS, LOCAL, CLAUDE, CURSOR];
const MAX_DEPTH: usize = 5;
const MAX_FILE: usize = 256 * 1024;
const MAX_TOTAL: usize = 512 * 1024;
const MAX_IMPORTS: usize = 128;

fn personal_rule_path(home: &Path) -> PathBuf {
    home.join(".claude").join(CLAUDE)
}

/// Where a project keeps its path-scoped rules (Markdown, any depth).
pub(super) fn rules_dir(root: &Path) -> PathBuf {
    root.join(".claude").join("rules")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RuleSource {
    Project,
    Personal,
}

#[derive(Default)]
pub(super) struct Instructions {
    pub sections: Vec<(String, String)>,
    pub watch: InstructionWatch,
}

/// Names checked during the load, including missing rules and import candidates.
#[derive(Default)]
pub(crate) struct InstructionWatch {
    names: BTreeMap<PathBuf, NameStamp>,
    scope: Option<InstructionScope>,
}

impl InstructionWatch {
    pub(super) fn is_stale(&self, scope: &InstructionScope) -> bool {
        self.scope.as_ref() != Some(scope) || self.paths_changed()
    }

    fn paths_changed(&self) -> bool {
        self.names
            .iter()
            .any(|(path, before)| *before != NameStamp::of(path))
    }

    fn observe(&mut self, path: &Path) -> std::io::Result<Metadata> {
        let entry = path.symlink_metadata();
        self.names
            .entry(path.to_owned())
            .or_insert_with(|| NameStamp::from_entry(path, &entry));
        entry
    }
}

#[derive(PartialEq, Eq)]
struct NameStamp {
    entry: Result<FileStamp, std::io::ErrorKind>,
    target: Option<Result<FileStamp, std::io::ErrorKind>>,
}

impl NameStamp {
    fn of(path: &Path) -> Self {
        Self::from_entry(path, &path.symlink_metadata())
    }

    fn from_entry(path: &Path, entry: &std::io::Result<Metadata>) -> Self {
        Self {
            entry: FileStamp::from_result(entry),
            target: entry
                .as_ref()
                .ok()
                .filter(|meta| meta.is_symlink())
                .map(|_| FileStamp::from_result(&path.metadata())),
        }
    }
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    kind: std::fs::FileType,
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64),
    #[cfg(unix)]
    mode_and_links: (u32, u64),
}

impl FileStamp {
    fn from_result(meta: &std::io::Result<Metadata>) -> Result<Self, std::io::ErrorKind> {
        meta.as_ref().map(Self::of).map_err(std::io::Error::kind)
    }

    fn of(meta: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            kind: meta.file_type(),
            // A directory is not rule text. Adding an unrelated child does
            // not invalidate the prompt; the candidate names are watched.
            len: if meta.is_dir() { 0 } else { meta.len() },
            modified: if meta.is_dir() {
                None
            } else {
                meta.modified().ok()
            },
            #[cfg(unix)]
            identity: (meta.dev(), meta.ino()),
            #[cfg(unix)]
            mode_and_links: (meta.mode(), if meta.is_dir() { 0 } else { meta.nlink() }),
        }
    }
}

/// An owned snapshot of the policy used to load rules.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct InstructionScope {
    root: PathBuf,
    enabled: bool,
    permissions: SessionPermissions,
    declined: DeclinedScopes,
    origin: ReadOrigin,
    coding: bool,
    personal: bool,
    ceiling: Option<PermissionMode>,
    /// The place rules this session has met, in the order met.
    place_rules: Vec<PathBuf>,
}

/// Rules written for one part of a project — a sub-folder's rule file, or a
/// rule under `.claude/rules/` whose `paths:` globs match — that a coding
/// session's file tools have met. Once met, a rule renders in the stable
/// prompt with the others, so a rebuild or a compaction keeps it and a change
/// to it is picked up like any rule file's. Held on the engine; an engine
/// dropped forgets them and the next file tool there meets them again.
#[derive(Default)]
pub(super) struct PlaceRules {
    met: Vec<PathBuf>,
    /// Met while the tools of the model's current reply ran: the reply was
    /// written without them.
    this_reply: BTreeSet<PathBuf>,
}

impl PlaceRules {
    /// A new model reply's tools begin: what was met before, it has seen.
    pub(super) fn new_reply(&mut self) {
        self.this_reply.clear();
    }
}

/// What one file tool call met (see [`AgentEngine::meet_place_rules`]).
#[derive(Default)]
pub(super) struct PlaceMeeting {
    /// New rules joined the prompt: the system message is built again.
    pub added: bool,
    /// A change the model wrote before its rules reached it: refused with
    /// this line, once.
    pub wait: Option<String>,
}

impl AgentEngine {
    pub(super) fn instruction_scope(&self) -> InstructionScope {
        let coding = self.is_coding_session();
        // A companion or app turn can speak in someone else's coding session.
        let personal = coding
            && self.agent_id.as_deref() != Some(super::agent::COMPANION_AGENT_ID)
            && self.tools.builtins.private_app.is_none()
            && self.turn_place().is_none_or(|place| place.skill.is_none());
        InstructionScope {
            root: self.cfg.ws_root.clone(),
            enabled: self.prompt_profile.include_project_context,
            permissions: self.session_permissions.clone(),
            declined: self.declined_scopes.clone(),
            origin: self.read_origin(),
            coding,
            personal,
            ceiling: self.session_spec.ceiling,
            place_rules: self.place_rules.met.clone(),
        }
    }

    /// A file tool (Read, Write, Edit) is about to touch a path: the rules
    /// written for that path's place join the session's prompt. A Write or
    /// Edit the model asked for before those rules reached it is refused
    /// once, so the change is made with them in view.
    pub(super) fn meet_place_rules(
        &mut self,
        tool: &str,
        args: &serde_json::Value,
    ) -> PlaceMeeting {
        if !matches!(tool, "Read" | "Write" | "Edit") {
            return PlaceMeeting::default();
        }
        let cwd = self.tools.builtins.cwd();
        let Some(path) = super::render::normalize_tool_path_arg(&cwd, args) else {
            return PlaceMeeting::default();
        };
        let rules = self.instruction_scope().place_rules_for(&cwd.join(path));
        let mut meeting = PlaceMeeting::default();
        for rule in &rules {
            if !self.place_rules.met.contains(rule) {
                self.place_rules.met.push(rule.clone());
                self.place_rules.this_reply.insert(rule.clone());
                meeting.added = true;
            }
        }
        let unseen: Vec<String> = rules
            .iter()
            .filter(|rule| self.place_rules.this_reply.contains(*rule))
            .map(|rule| relative_label(&self.cfg.ws_root, rule))
            .collect();
        if tool != "Read" && !unseen.is_empty() {
            meeting.wait = Some(format!(
                "Not changed yet: rules for this place were just added to your instructions ({}). Check the change against them, then make it again.",
                unseen.join(", ")
            ));
        }
        meeting
    }
}

fn relative_label(root: &Path, path: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());
    path.strip_prefix(&root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Where a path rule applies, from its front matter.
enum RuleReach {
    Everywhere,
    Paths(Vec<String>),
}

/// Split a rule's YAML front matter from its body. No front matter, or no
/// `paths:` in it, means the rule applies everywhere.
fn split_front_matter(text: &str) -> Result<(RuleReach, &str), &'static str> {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Ok((RuleReach::Everywhere, text));
    };
    let mut offset = 0;
    let mut end = None;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            end = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    let (yaml_end, body_start) = end.ok_or("front matter not closed")?;
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Paths {
        One(String),
        Many(Vec<String>),
    }
    #[derive(serde::Deserialize)]
    struct Front {
        paths: Option<Paths>,
    }
    let yaml = &rest[..yaml_end];
    let front: Option<Front> = if yaml.trim().is_empty() {
        None
    } else {
        Some(serde_norway::from_str(yaml).map_err(|_| "unreadable front matter")?)
    };
    let globs: Vec<String> = match front.and_then(|f| f.paths) {
        None => Vec::new(),
        Some(Paths::One(list)) => list.split(',').map(|g| g.trim().to_owned()).collect(),
        Some(Paths::Many(list)) => list.into_iter().map(|g| g.trim().to_owned()).collect(),
    };
    let globs: Vec<String> = globs.into_iter().filter(|g| !g.is_empty()).collect();
    let reach = if globs.is_empty() {
        RuleReach::Everywhere
    } else {
        RuleReach::Paths(globs)
    };
    Ok((reach, &rest[body_start..]))
}

fn globs_match(globs: &[String], relative: &Path) -> bool {
    globs.iter().any(|glob| {
        globset::GlobBuilder::new(glob)
            .literal_separator(true)
            .build()
            .is_ok_and(|glob| glob.compile_matcher().is_match(relative))
    })
}

/// The canonical form of a path that may not exist yet (a Write of a new
/// file): its nearest existing ancestor, canonical, with the rest joined on.
fn canonical_place(path: &Path) -> Option<PathBuf> {
    let mut missing = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(canonical) = existing.canonicalize() {
            let mut out = canonical;
            out.extend(missing.iter().rev());
            return Some(out);
        }
        missing.push(existing.file_name()?);
        existing = existing.parent()?;
    }
}

/// The Markdown files under `.claude/rules/`, any depth, sorted. Symlinks
/// are listed, not followed: the loader's gate decides on them.
fn rule_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && depth < MAX_DEPTH {
                walk(&path, depth + 1, out);
            } else if !kind.is_dir() && path.extension().is_some_and(|ext| ext == "md") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out.sort();
    out
}

impl InstructionScope {
    pub(super) fn load(&self) -> Instructions {
        self.load_with_home(dirs::home_dir().as_deref())
    }

    fn load_with_home(&self, home: Option<&Path>) -> Instructions {
        let mut result = Instructions::default();
        result.watch.scope = Some(self.clone());
        if !self.enabled {
            return result;
        }
        let _ = result.watch.observe(&self.root);
        if let Some(home) = home {
            let _ = result.watch.observe(home);
        }
        let Ok(root) = self.root.canonicalize() else {
            return result;
        };
        let mut loader = self.loader(&root, home, result);
        let personal = home.filter(|_| self.personal).and_then(|home| {
            let path = personal_rule_path(home);
            let label = format!("Personal instructions (from {})", path.display());
            // Treat a mounted/aliased home like the project root. Links below
            // that boundary still use the gate, as do all explicit imports.
            let discovery_home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
            loader.discover_entry(
                label,
                personal_rule_path(&discovery_home),
                RuleSource::Personal,
            )
        });
        // Personal rules have their own allowance. Reserve both sets of
        // discovered files before expanding either set of imports.
        let personal_bytes_left = std::mem::replace(&mut loader.bytes_left, MAX_TOTAL);
        let dirs = super::tools::project_ancestors_within(&root, home);
        // Reserve discovered files in the legacy nearest-first order, then
        // render outermost first. An import cannot steal a discovered section.
        let mut entries = Vec::new();
        for dir in dirs {
            for &name in NAMES {
                let path = dir.join(name);
                let label = if dir == root {
                    name.to_owned()
                } else {
                    format!("{name} (from {})", dir.display())
                };
                if let Some(entry) = loader.discover_entry(label, path, RuleSource::Project) {
                    entries.push(entry);
                }
            }
        }
        if let Some(entry) = personal {
            let project_bytes_left = std::mem::replace(&mut loader.bytes_left, personal_bytes_left);
            loader.render(entry);
            loader.bytes_left = project_bytes_left;
            loader.imports_left = MAX_IMPORTS;
        }
        for entry in entries.into_iter().rev() {
            loader.render(entry);
        }
        // Coding sessions only, the personal rule's boundary: the project's
        // rules for everywhere, then the place rules met, in that order.
        if self.personal {
            for path in self.everywhere_rules(&root, home) {
                if let Some(entry) = loader.discover_place_rule(path) {
                    loader.render(entry);
                }
            }
            for path in self
                .place_rules
                .iter()
                .filter(|path| path.starts_with(&root))
            {
                if let Some(entry) = loader.discover_place_rule(path.clone()) {
                    loader.render(entry);
                }
            }
        }
        loader.result
    }

    fn loader<'a>(
        &'a self,
        project: &'a Path,
        home: Option<&'a Path>,
        result: Instructions,
    ) -> Loader<'a> {
        Loader {
            scope: self,
            project,
            home,
            result,
            files: BTreeSet::new(),
            bytes_left: MAX_TOTAL,
            imports_left: MAX_IMPORTS,
        }
    }

    /// Each rule under `.claude/rules/` with its reach, read through the
    /// loader's own gate and limits (a rule it cannot read is `Err`).
    fn path_rules(
        &self,
        root: &Path,
        home: Option<&Path>,
    ) -> Vec<(PathBuf, Result<RuleReach, &'static str>)> {
        let mut probe = self.loader(root, home, Instructions::default());
        rule_files(&rules_dir(root))
            .into_iter()
            .filter_map(|path| {
                let reach = match probe.discover(&path, RuleSource::Project) {
                    Ok(None) => return None,
                    Ok(Some((_, text))) => split_front_matter(&text).map(|(reach, _)| reach),
                    Err(reason) => Err(reason),
                };
                Some((path, reach))
            })
            .collect()
    }

    /// The path rules with no `paths:` — and any that cannot be read, so
    /// their refusal line shows where every rule shows.
    fn everywhere_rules(&self, root: &Path, home: Option<&Path>) -> Vec<PathBuf> {
        self.path_rules(root, home)
            .into_iter()
            .filter(|(_, reach)| !matches!(reach, Ok(RuleReach::Paths(_))))
            .map(|(path, _)| path)
            .collect()
    }

    /// The rules written for `target`'s place: rule files in each folder
    /// below the project root down to the target's own, outermost first,
    /// then the path rules whose globs match it. Coding sessions only.
    pub(super) fn place_rules_for(&self, target: &Path) -> Vec<PathBuf> {
        if !self.enabled || !self.personal {
            return Vec::new();
        }
        let (Ok(root), Some(target)) = (self.root.canonicalize(), canonical_place(target)) else {
            return Vec::new();
        };
        let Ok(relative) = target.strip_prefix(&root) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let folders: Vec<&Path> = relative
            .parent()
            .map(|folder| {
                folder
                    .ancestors()
                    .filter(|dir| !dir.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default();
        for folder in folders.into_iter().rev() {
            for &name in NAMES {
                let path = root.join(folder).join(name);
                if path.symlink_metadata().is_ok() {
                    found.push(path);
                }
            }
        }
        let home = dirs::home_dir();
        for (path, reach) in self.path_rules(&root, home.as_deref()) {
            if let Ok(RuleReach::Paths(globs)) = reach {
                if globs_match(&globs, relative) {
                    found.push(path);
                }
            }
        }
        found
    }
}

struct RuleEntry {
    label: String,
    path: PathBuf,
    source: RuleSource,
    text: Result<String, &'static str>,
}

struct Loader<'a> {
    scope: &'a InstructionScope,
    project: &'a Path,
    home: Option<&'a Path>,
    result: Instructions,
    files: BTreeSet<PathBuf>,
    bytes_left: usize,
    imports_left: usize,
}

impl Loader<'_> {
    fn discover_entry(
        &mut self,
        label: String,
        path: PathBuf,
        source: RuleSource,
    ) -> Option<RuleEntry> {
        let (source, text) = match self.discover(&path, source) {
            Ok(Some((source, text))) => (source, Ok(text)),
            Ok(None) => return None,
            Err(reason) => (source, Err(reason)),
        };
        Some(RuleEntry {
            label,
            path,
            source,
            text,
        })
    }

    /// A place rule — a sub-folder's rule file or a path rule — labelled
    /// with where it applies; a path rule's front matter is not rule text.
    fn discover_place_rule(&mut self, path: PathBuf) -> Option<RuleEntry> {
        let relative = path.strip_prefix(self.project).ok()?.to_owned();
        let mut entry = self.discover_entry(String::new(), path, RuleSource::Project)?;
        if !relative.starts_with(rules_dir(Path::new(""))) {
            let folder = relative.parent().unwrap_or(Path::new(""));
            entry.label = format!(
                "{} (for files under {}/)",
                relative.display(),
                folder.display()
            );
            return Some(entry);
        }
        entry.label = relative.display().to_string();
        let split = match &entry.text {
            Ok(text) => split_front_matter(text).map(|(reach, body)| (reach, body.to_owned())),
            Err(reason) => Err(*reason),
        };
        match split {
            Ok((reach, body)) => {
                if let RuleReach::Paths(globs) = reach {
                    entry.label = format!("{} (for {})", entry.label, globs.join(", "));
                }
                entry.text = Ok(body);
            }
            Err(reason) => entry.text = Err(reason),
        }
        Some(entry)
    }

    fn render(&mut self, entry: RuleEntry) {
        let text = match entry.text {
            Ok(text) => self.expand(&entry.path, &text, 0, entry.source),
            Err(reason) => format!("[Rule not loaded: {reason}.]"),
        };
        if !text.trim().is_empty() {
            self.result
                .sections
                .push((entry.label, text.trim().to_owned()));
        }
    }

    fn discover(
        &mut self,
        path: &Path,
        source: RuleSource,
    ) -> Result<Option<(RuleSource, String)>, &'static str> {
        let entry = match self.result.watch.observe(path) {
            Ok(entry) => entry,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("unreadable"),
        };
        let canonical = path.canonicalize().map_err(|_| "unreadable")?;
        if canonical != path {
            let _ = self.result.watch.observe(&canonical);
        }
        // Discovery starts in canonical directories. Compare the parent,
        // not the filename: case-insensitive lookup can change its spelling.
        let parent_link = path.parent().is_some_and(|parent| {
            parent
                .symlink_metadata()
                .is_ok_and(|meta| meta.is_symlink())
                || parent.canonicalize().ok().as_deref() != canonical.parent()
        });
        let expected = if entry.is_symlink() || parent_link {
            let metadata = canonical.metadata().map_err(|_| "unreadable")?;
            self.check_import_target(path, &canonical, source)?;
            Some(metadata)
        } else {
            None
        };
        if self.files.contains(&canonical) {
            return Ok(None);
        }
        let source = if expected.is_some() {
            self.followed_source(&canonical, source)
        } else {
            source
        };
        // For a plain discovery, open the name we checked, not a resolved
        // replacement. The descriptor walk rejects links swapped in later.
        let opened = if expected.is_some() { &canonical } else { path };
        self.read(opened, expected.as_ref()).map(|text| {
            self.files.insert(canonical);
            Some((source, text))
        })
    }

    /// Opting into a file inside the workspace does not let its author
    /// opt into further reads outside it.
    fn followed_source(&self, canonical: &Path, source: RuleSource) -> RuleSource {
        if canonical.starts_with(self.project) {
            RuleSource::Project
        } else {
            source
        }
    }

    fn check_import_target<'p>(
        &self,
        target: &Path,
        canonical: &'p Path,
        source: RuleSource,
    ) -> Result<&'p Path, &'static str> {
        let label = match canonical.strip_prefix(self.project) {
            Ok(relative) => relative,
            Err(_) if source == RuleSource::Personal => canonical,
            Err(_) => return Err("outside the project"),
        };
        if !permission::allows_instruction_import(
            target,
            canonical,
            self.project,
            &self.scope.permissions,
            self.scope.coding,
            self.scope.ceiling,
            self.scope.origin,
            &self.scope.declined,
        ) {
            return Err("protected or requires approval");
        }
        Ok(label)
    }

    fn check_size(&self, len: u64) -> Result<(), &'static str> {
        if len > MAX_FILE as u64 {
            return Err("over file size limit");
        }
        if len > self.bytes_left as u64 {
            return Err("over total size limit");
        }
        Ok(())
    }

    fn read(&mut self, path: &Path, expected: Option<&Metadata>) -> Result<String, &'static str> {
        let limit = self.bytes_left.min(MAX_FILE);
        let file = open_text(path, expected.is_some()).ok_or("unreadable")?;
        let meta = file.metadata().map_err(|_| "unreadable")?;
        if !meta.is_file() {
            return Err("not a regular file");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // A hard link can hide a protected path's name from the gate.
            if expected.is_some() && meta.nlink() != 1 {
                return Err("hard-linked file");
            }
            if expected
                .is_some_and(|before| (before.dev(), before.ino()) != (meta.dev(), meta.ino()))
            {
                return Err("file changed before reading");
            }
        }
        self.check_size(meta.len())?;
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "unreadable")?;
        self.check_size(bytes.len() as u64)?;
        let text = String::from_utf8(bytes).map_err(|_| "not UTF-8 text")?;
        if text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
        {
            return Err("not UTF-8 text");
        }
        self.bytes_left -= text.len();
        Ok(text)
    }

    fn expand(&mut self, path: &Path, text: &str, depth: usize, source: RuleSource) -> String {
        let mut code = Parser::new(text)
            .into_offset_iter()
            .filter_map(|(event, range)| {
                matches!(
                    event,
                    Event::Start(Tag::CodeBlock(_))
                        | Event::Code(_)
                        | Event::Html(_)
                        | Event::InlineHtml(_)
                )
                .then_some(range)
            })
            .peekable();
        let mut out = String::new();
        let mut copied = 0;
        for (start, _) in text.match_indices('@') {
            if !text[..start]
                .chars()
                .next_back()
                .is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '['))
            {
                continue;
            }
            while code.peek().is_some_and(|range| range.end <= start) {
                code.next();
            }
            if code.peek().is_some_and(|range| range.contains(&start)) {
                continue;
            }
            let tail = &text[start + 1..];
            let token = tail.split(char::is_whitespace).next().unwrap_or("");
            if token.is_empty() || token.contains('@') {
                continue;
            }
            let Some((expanded, consumed)) = self.import(path, token, depth, source) else {
                continue;
            };
            out.push_str(&text[copied..start]);
            out.push_str(&expanded);
            copied = start + 1 + consumed;
        }
        out.push_str(&text[copied..]);
        out
    }

    /// None keeps the token verbatim (it doesn't name an existing file).
    fn import(
        &mut self,
        path: &Path,
        token: &str,
        depth: usize,
        source: RuleSource,
    ) -> Option<(String, usize)> {
        let base = path.parent()?;
        let shortened =
            token.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '"']);
        let shortened = (shortened != token && !shortened.is_empty()).then_some(shortened);
        let (token, (target, canonical, metadata)) = [Some(token), shortened]
            .into_iter()
            .flatten()
            .find_map(|candidate| {
                self.existing_import(base, candidate)
                    .map(|file| (candidate, file))
            })?;
        let text = self.expand_import(token, &target, &canonical, &metadata, depth, source);
        Some((text, token.len()))
    }

    fn expand_import(
        &mut self,
        token: &str,
        target: &Path,
        canonical: &Path,
        metadata: &Metadata,
        depth: usize,
        source: RuleSource,
    ) -> String {
        let note = |reason| format!("[Import @{token} not loaded: {reason}.]");
        if self.imports_left == 0 {
            return note("import count limit");
        }
        self.imports_left -= 1;
        let label = match self.check_import_target(target, canonical, source) {
            Ok(relative) => relative,
            Err(reason) => return note(reason),
        };
        if self.files.contains(canonical) {
            return String::new();
        }
        if depth == MAX_DEPTH {
            return note("depth limit");
        }
        let text = match self.read(canonical, Some(metadata)) {
            Ok(text) => text,
            Err(reason) => return note(reason),
        };
        self.files.insert(canonical.to_path_buf());
        let text = self.expand(
            target,
            &text,
            depth + 1,
            self.followed_source(canonical, source),
        );
        format!(
            "\n\n[Imported from {label:?}]\n{}\n[End import]\n\n",
            text.trim()
        )
    }
    fn existing_import(
        &mut self,
        base: &Path,
        token: &str,
    ) -> Option<(PathBuf, PathBuf, Metadata)> {
        let target = match token.strip_prefix("~/") {
            Some(path) => self.home?.join(path),
            None => base.join(token),
        };
        self.result.watch.observe(&target).ok()?;
        let canonical = target.canonicalize().ok()?;
        let metadata = self.result.watch.observe(&canonical).ok()?;
        metadata.is_file().then_some((target, canonical, metadata))
    }
}

/// Open the already checked canonical path without following a replacement
/// symlink at any component. NONBLOCK prevents a swapped FIFO from hanging.
#[cfg(unix)]
fn open_text(path: &Path, _import: bool) -> Option<File> {
    use rustix::fs::{open, openat, Mode, OFlags};
    use std::path::Component;
    let mut components = path.components().peekable();
    if components.next()? != Component::RootDir {
        return None;
    }
    let flags = OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let search = OFlags::PATH;
    #[cfg(target_os = "macos")]
    let search = OFlags::from_bits_retain(libc::O_SEARCH as _);
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    let search = OFlags::RDONLY;
    let directory = flags | OFlags::DIRECTORY | search;
    let mut fd = open("/", directory, Mode::empty()).ok()?;
    while let Some(Component::Normal(name)) = components.next() {
        let flags = if components.peek().is_some() {
            directory
        } else {
            flags | OFlags::RDONLY
        };
        fd = openat(&fd, name, flags, Mode::empty()).ok()?;
    }
    Some(File::from(fd))
}

#[cfg(not(unix))]
fn open_text(path: &Path, import: bool) -> Option<File> {
    // Keep native discovery but fail closed for imports on platforms without
    // the descriptor-relative, no-symlink walk above.
    if import || !path.is_file() {
        return None;
    }
    File::open(path).ok()
}

#[cfg(test)]
mod tests;
