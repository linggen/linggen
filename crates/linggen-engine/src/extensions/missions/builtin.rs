//! The missions the engine ships (this repo's `missions/`), and who owns
//! which part of one. The engine owns a built-in's instructions — its body,
//! kickoff, tools, permissions and every other field; the person owns
//! [`PERSONAL_MISSION_FIELDS`]. This module decides both: what start-up
//! seeds and refreshes ([`sync`]) and what a change through the mission API
//! or the raw editor may touch ([`check_change`]).
//!
//! A built-in is a mission the install marker ([`SEEDED`]) lists under a
//! name this binary ships. The marker cannot tell a person's own file under
//! such a name from a built-in the person edited by hand, so both are
//! refreshed, with the old file kept as `mission.md.prev`.

use anyhow::{Context, Result};
use rust_embed::Embed;
use serde_norway::{Mapping, Value};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

/// Built-in mission definitions, embedded at compile time.
#[derive(Embed)]
#[folder = "../../missions/"]
struct MissionAssets;

/// The install marker in the missions folder: the built-ins the engine has
/// seeded. A built-in the person deleted stays listed, and stays deleted.
pub const SEEDED: &str = ".builtin-missions-installed";

/// The person owns scheduling and model selection. Everything else,
/// including kickoff messages, follows the binary.
pub const PERSONAL_MISSION_FIELDS: &[&str] = &["enabled", "schedule", "catchup_hours", "model"];

/// A change refused because the mission is a built-in: only the person's
/// fields are theirs to change.
#[derive(Debug)]
pub struct BuiltInOwned {
    pub id: String,
}

impl std::fmt::Display for BuiltInOwned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "'{}' is a built-in: its schedule, switch, catch-up and model are yours; \
             to change its instructions, copy it under a new name",
            self.id
        )
    }
}

impl std::error::Error for BuiltInOwned {}

/// Whether `id` in `missions_dir` is a built-in.
pub fn is_builtin(missions_dir: &Path, id: &str) -> bool {
    names().iter().any(|n| n == id)
        && seeded(&missions_dir.join(SEEDED))
            .unwrap_or_default()
            .iter()
            .any(|n| n == id)
}

/// Refuse writing `after` over `before` (`None`: no file) for a built-in,
/// unless only the person's fields differ. `Ok` for any other mission.
pub fn check_change(missions_dir: &Path, id: &str, before: Option<&str>, after: &str) -> Result<()> {
    if !is_builtin(missions_dir, id) {
        return Ok(());
    }
    let owned = || anyhow::Error::from(BuiltInOwned { id: id.to_string() });
    let Some(before) = before else {
        return Err(owned());
    };
    let (was, _, was_body) = parts(before)?;
    let (now, _, now_body) = parts(after).map_err(|_| owned())?;
    if was != now || was_body != now_body {
        return Err(owned());
    }
    Ok(())
}

/// `text` with the person's fields in `changes` set (`Some`) or removed
/// (`None`); nothing else moves.
pub fn with_personal(text: &str, changes: &[(&str, Option<Value>)]) -> Result<String> {
    let (engine, mut personal, body) = parts(text)?;
    for (key, value) in changes {
        debug_assert!(PERSONAL_MISSION_FIELDS.contains(key), "{key}");
        match value {
            Some(v) => personal.insert((*key).into(), v.clone()),
            None => personal.remove(*key),
        };
    }
    render(engine, personal, &body)
}

/// Seed new built-ins and refresh previously seeded definitions. Returns
/// the number of files installed or refreshed (not counting backups).
pub fn sync(missions_dir: &Path) -> Result<usize> {
    let sentinel = missions_dir.join(SEEDED);
    let mut seeded = seeded(&sentinel)?;
    let mut count = 0;
    for name in names() {
        let dir = missions_dir.join(&name);
        match dir.symlink_metadata() {
            Ok(metadata) if !metadata.is_dir() => continue,
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        let path = dir.join("mission.md");
        if seeded.contains(&name) {
            match refresh(&path, &name) {
                Ok(changed) => count += usize::from(changed),
                Err(e) => tracing::warn!("Failed to refresh built-in mission {name}: {e:#}"),
            }
            continue;
        }
        // A pre-existing definition was not seeded by the engine. Do not
        // claim it, even if its directory has a built-in name.
        if seed(&path, &name)? {
            seeded.push(name);
            fs::write(&sentinel, seeded.join("\n") + "\n")?;
            count += 1;
        }
    }
    Ok(count)
}

fn refresh(path: &Path, name: &str) -> Result<bool> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let old = fs::read_to_string(path)?;
    let builtin = builtin(name)?;
    let Some(merged) = merge(&old, std::str::from_utf8(&builtin.data)?)? else {
        return Ok(false);
    };

    let dir = path.parent().context("Missing mission directory")?;
    let backup = path.with_extension("md.prev");
    let kept = !backup.try_exists()?;
    if kept {
        let mut copy = tempfile::NamedTempFile::new_in(dir)?;
        copy.write_all(old.as_bytes())?;
        if let Err(e) = copy.persist_noclobber(&backup) {
            if e.error.kind() != ErrorKind::AlreadyExists {
                return Err(e.into());
            }
        }
    }
    let mut replacement = tempfile::NamedTempFile::new_in(dir)?;
    replacement.write_all(merged.text.as_bytes())?;
    replacement.as_file().set_permissions(metadata.permissions())?;
    replacement.persist(path)?;
    if merged.instructions_replaced {
        let was = if kept { "kept at" } else { "not kept (the first one replaced is at" };
        let close = if kept { "" } else { ")" };
        tracing::info!(
            "built-in mission {} replaced by this engine's; the file it replaced is {was} {}{close}",
            path.display(),
            backup.display()
        );
    }
    Ok(true)
}

/// A refreshed file, and whether more than the person's fields changed.
struct Merged {
    text: String,
    instructions_replaced: bool,
}

/// The built-in with the installed file's personal fields — a field the
/// installed file lacks keeps the built-in's default. `None` when that is
/// what is installed.
fn merge(installed: &str, builtin: &str) -> Result<Option<Merged>> {
    let (installed_engine, installed_personal, installed_body) = parts(installed)?;
    let (builtin_engine, mut personal, builtin_body) = parts(builtin)?;
    for (key, value) in &installed_personal {
        personal.insert(key.clone(), value.clone());
    }
    let instructions_replaced =
        installed_engine != builtin_engine || installed_body != builtin_body;
    if !instructions_replaced && installed_personal == personal {
        return Ok(None);
    }
    Ok(Some(Merged {
        text: render(builtin_engine, personal, &builtin_body)?,
        instructions_replaced,
    }))
}

/// A mission file as the engine's fields, the person's, and the body.
fn parts(text: &str) -> Result<(Mapping, Mapping, String)> {
    let (yaml, body) = crate::extensions::frontmatter::split(text);
    let mut engine: Mapping = serde_norway::from_str(yaml.unwrap_or("{}"))?;
    let mut personal = Mapping::new();
    for key in PERSONAL_MISSION_FIELDS {
        if let Some(value) = engine.remove(*key) {
            personal.insert((*key).into(), value);
        }
    }
    Ok((engine, personal, body.to_string()))
}

fn render(mut engine: Mapping, personal: Mapping, body: &str) -> Result<String> {
    engine.extend(personal);
    let yaml = serde_norway::to_string(&engine)?;
    Ok(format!("---\n{yaml}---{body}"))
}

/// The missions the sentinel lists. An empty sentinel is the old
/// install-once marker: `dream` was the only built-in then.
fn seeded(sentinel: &Path) -> Result<Vec<String>> {
    let text = match fs::read_to_string(sentinel) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return Ok(vec!["dream".to_string()]);
    }
    Ok(names)
}

/// The built-in missions, by folder name.
fn names() -> Vec<String> {
    let mut names: Vec<String> = MissionAssets::iter()
        .filter_map(|f| f.split('/').next().map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Seed a definition only if no mission already occupies this name.
fn seed(path: &Path, name: &str) -> Result<bool> {
    let builtin = builtin(name)?;
    fs::create_dir_all(path.parent().context("Missing mission directory")?)?;
    let mut file = match fs::File::create_new(path) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    file.write_all(builtin.data.as_ref())?;
    Ok(true)
}

fn builtin(name: &str) -> Result<rust_embed::EmbeddedFile> {
    MissionAssets::get(&format!("{name}/mission.md")).context("Missing built-in definition")
}

#[cfg(test)]
mod tests;
