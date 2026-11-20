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
        }
    }
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
        let mut loader = Loader {
            scope: self,
            project: &root,
            home,
            result,
            files: BTreeSet::new(),
            bytes_left: MAX_TOTAL,
            imports_left: MAX_IMPORTS,
        };
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
        loader.result
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
