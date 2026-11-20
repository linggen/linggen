//! Discovery, expansion and change detection for native project instructions.

use super::permission::{self, DeclinedScopes, PermissionMode, ReadOrigin, SessionPermissions};
use super::AgentEngine;
use pulldown_cmark::{Event, Parser, Tag};
use std::collections::{hash_map::DefaultHasher, BTreeSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) const AGENTS: &str = "AGENTS.md";
pub(super) const CLAUDE: &str = "CLAUDE.md";
const LOCAL: &str = "CLAUDE.local.md";
pub(super) const CURSOR: &str = ".cursorrules";
const NAMES: &[&str] = &[AGENTS, LOCAL, CLAUDE, CURSOR];
const MAX_DEPTH: usize = 5;
const MAX_FILE: usize = 32 * 1024;
const MAX_TOTAL: usize = 128 * 1024;
const MAX_IMPORTS: usize = 128;

#[derive(Default)]
pub(super) struct Instructions {
    pub sections: Vec<(String, String)>,
    files: BTreeSet<PathBuf>,
}

impl Instructions {
    pub fn fingerprint(&self) -> u64 {
        let mut hash = DefaultHasher::new();
        self.sections.hash(&mut hash);
        self.files.hash(&mut hash);
        hash.finish()
    }
}

/// An owned snapshot so staleness checks can do their IO off the runtime.
pub(super) struct InstructionScope {
    root: PathBuf,
    enabled: bool,
    permissions: SessionPermissions,
    declined: DeclinedScopes,
    origin: ReadOrigin,
    coding: bool,
    ceiling: Option<PermissionMode>,
}

impl AgentEngine {
    pub(super) fn instruction_scope(&self) -> InstructionScope {
        InstructionScope {
            root: self.cfg.ws_root.clone(),
            enabled: self.prompt_profile.include_project_context,
            permissions: self.session_permissions.clone(),
            declined: self.declined_scopes.clone(),
            origin: self.read_origin(),
            coding: self.is_coding_session(),
            ceiling: self.session_spec.ceiling,
        }
    }
}

impl InstructionScope {
    pub(super) fn load(&self) -> Instructions {
        self.load_with_home(dirs::home_dir().as_deref())
    }

    fn load_with_home(&self, home: Option<&Path>) -> Instructions {
        if !self.enabled {
            return Instructions::default();
        }
        let Ok(root) = self.root.canonicalize() else {
            return Instructions::default();
        };
        let mut loader = Loader {
            scope: self,
            project: &root,
            result: Instructions::default(),
            bytes_left: MAX_TOTAL,
            imports_left: MAX_IMPORTS,
        };
        let dirs = super::tools::project_ancestors_within(&root, home);
        // Reserve discovered files in the legacy nearest-first order, then
        // render outermost first. An import cannot steal a discovered section.
        let mut entries = Vec::new();
        for dir in dirs {
            for &name in NAMES {
                let path = dir.join(name);
                let Ok(real) = path.canonicalize() else {
                    continue;
                };
                // Discovery is implicit too: a rule filename must not be
                // a symlink around the import gate.
                if real != path || loader.result.files.contains(&real) {
                    continue;
                }
                if let Some(text) = loader.read(&real, None) {
                    let label = if dir == root {
                        name.to_owned()
                    } else {
                        format!("{name} (from {})", dir.display())
                    };
                    entries.push((label, path, text));
                }
            }
        }
        for (label, path, text) in entries.into_iter().rev() {
            let text = loader.expand(&path, &text, 0);
            if !text.trim().is_empty() {
                loader.result.sections.push((label, text.trim().to_owned()));
            }
        }
        loader.result
    }
}

struct Loader<'a> {
    scope: &'a InstructionScope,
    project: &'a Path,
    result: Instructions,
    bytes_left: usize,
    imports_left: usize,
}

impl Loader<'_> {
    fn read(&mut self, path: &Path, expected: Option<&std::fs::Metadata>) -> Option<String> {
        let limit = self.bytes_left.min(MAX_FILE);
        if limit == 0 {
            return None;
        }
        let file = open_text(path, expected.is_some())?;
        let meta = file.metadata().ok()?;
        if !meta.is_file() || meta.len() > limit as u64 {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // A hard link can hide a protected path's name from the gate.
            // Check we opened the same file the import gate examined.
            if meta.nlink() != 1
                || expected
                    .is_some_and(|before| (before.dev(), before.ino()) != (meta.dev(), meta.ino()))
            {
                return None;
            }
        }
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() > limit {
            return None;
        }
        self.bytes_left -= bytes.len();
        let text = String::from_utf8(bytes).ok()?;
        if text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
        {
            return None;
        }
        self.result.files.insert(path.to_path_buf());
        Some(text)
    }

    fn expand(&mut self, source: &Path, text: &str, depth: usize) -> String {
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
            let end = start + 1 + token.len();
            let Some(expanded) = self.import(source, token, depth) else {
                continue;
            };
            out.push_str(&text[copied..start]);
            out.push_str(&expanded);
            copied = end;
        }
        out.push_str(&text[copied..]);
        out
    }

    /// None keeps the token verbatim (it doesn't name an existing file).
    fn import(&mut self, source: &Path, token: &str, depth: usize) -> Option<String> {
        if self.imports_left == 0 {
            return None;
        }
        self.imports_left -= 1;
        let base = source.parent()?;
        let target = match token.strip_prefix("~/") {
            Some(path) => dirs::home_dir()?.join(path),
            None => base.join(token),
        };
        let canonical = target.canonicalize().ok()?;
        let metadata = canonical.metadata().ok()?;
        if !metadata.is_file() {
            return None;
        }
        let note = |reason| format!("[Import @{token} not loaded: {reason}.]");
        if !canonical.starts_with(self.project) {
            return Some(note("outside the project"));
        }
        if !permission::allows_instruction_import(
            &target,
            &canonical,
            self.project,
            &self.scope.permissions,
            self.scope.coding,
            self.scope.ceiling,
            self.scope.origin,
            &self.scope.declined,
        ) {
            return Some(note("protected or requires approval"));
        }
        if self.result.files.contains(&canonical) {
            return Some(String::new());
        }
        if depth == MAX_DEPTH {
            return Some(note("depth limit"));
        }
        let Some(text) = self.read(&canonical, Some(&metadata)) else {
            return Some(note("unreadable, not text, or over size limit"));
        };
        let label = canonical.strip_prefix(self.project).ok()?;
        let text = self.expand(&target, &text, depth + 1);
        Some(format!(
            "\n\n[Imported from {label:?}]\n{}\n[End import]\n\n",
            text.trim()
        ))
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
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    let mut fd = open("/", flags | OFlags::DIRECTORY, Mode::empty()).ok()?;
    while let Some(Component::Normal(name)) = components.next() {
        let flags = if components.peek().is_some() {
            flags | OFlags::DIRECTORY
        } else {
            flags
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
