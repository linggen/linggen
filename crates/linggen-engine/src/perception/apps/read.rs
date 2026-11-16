//! Reading a declared file into one line: `name: label value · label value ·
//! age`. Values are copied as text — never compared, summed or judged.

use super::declare::{warn_once, WorldLine};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Characters in one app's line, all of it.
pub const LINE_CHARS: usize = 160;
/// Characters in one value; a longer one is cut and marked `…`.
pub const VALUE_CHARS: usize = 48;
/// Characters of the app's name.
pub const NAME_CHARS: usize = 32;
/// App lines in the block. When more apps have a reading, the ones whose
/// files changed most recently stay.
pub const LINES_MAX: usize = 8;
/// A reading older than this is left out rather than shown stale.
pub const MAX_AGE_SECS: u64 = 7 * 86_400;
/// A declared file larger than this is not a small reading; it is skipped.
pub const FILE_BYTES_MAX: u64 = 16 * 1024;

/// One installed skill that declares a line.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub name: String,
    pub dir: PathBuf,
    pub line: WorldLine,
}

/// What one app's file said, and when it last changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub app: String,
    pub pairs: Vec<(String, String)>,
    pub modified: SystemTime,
}

/// The parsed fields of each file, kept while its modification time and size
/// stand still — so a turn costs one `stat` per app, not a read and a parse.
#[derive(Debug, Default)]
pub struct Cache {
    files: HashMap<PathBuf, Entry>,
}

#[derive(Debug)]
struct Entry {
    modified: SystemTime,
    len: u64,
    pairs: Vec<(String, String)>,
}

/// Every app's line as it stands at `now`: fresh readings only, the most
/// recently changed first, at most [`LINES_MAX`].
pub fn lines(apps: &[App], now: SystemTime, cache: &mut Cache) -> Vec<String> {
    let mut readings: Vec<Reading> = apps.iter().filter_map(|a| read(a, now, cache)).collect();
    readings.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.app.cmp(&b.app)));
    readings
        .iter()
        .take(LINES_MAX)
        .filter_map(|r| render(r, now))
        .collect()
}

/// One app's reading, or `None`: no file, outside the skill, too large, too
/// old, not a JSON object, or none of the declared fields present as text.
pub fn read(app: &App, now: SystemTime, cache: &mut Cache) -> Option<Reading> {
    let path = confined(&app.dir, &app.line.file, &app.name)?;
    let meta = std::fs::metadata(&path).ok()?;
    let modified = meta.modified().ok()?;
    if age_secs(modified, now) > MAX_AGE_SECS {
        return None;
    }
    if meta.len() > FILE_BYTES_MAX {
        warn_once(&app.name, "world-line file is over 16 KB; skipped");
        return None;
    }
    let pairs = cached_pairs(&path, modified, meta.len(), app, cache)?;
    (!pairs.is_empty()).then(|| Reading {
        app: app.name.clone(),
        pairs,
        modified,
    })
}

/// The declared file, if it really is inside the skill's folder — a link
/// that leads out of it is refused, so no app can show another's data.
fn confined(dir: &Path, file: &str, skill: &str) -> Option<PathBuf> {
    let root = dir.canonicalize().ok()?;
    let path = root.join(file).canonicalize().ok()?;
    if !path.starts_with(&root) {
        warn_once(skill, "world-line file leads outside the skill; ignored");
        return None;
    }
    Some(path)
}

fn cached_pairs(
    path: &Path,
    modified: SystemTime,
    len: u64,
    app: &App,
    cache: &mut Cache,
) -> Option<Vec<(String, String)>> {
    if let Some(e) = cache.files.get(path) {
        if e.modified == modified && e.len == len {
            return Some(e.pairs.clone());
        }
    }
    let text = std::fs::read_to_string(path).ok()?;
    let pairs = pairs_of(&text, &app.line.fields, &app.name)?;
    cache.files.insert(
        path.to_path_buf(),
        Entry {
            modified,
            len,
            pairs: pairs.clone(),
        },
    );
    Some(pairs)
}

/// The declared fields of a JSON object, in declared order, as
/// `(label, value)`. A field that is missing or not a string, number or
/// boolean is left out.
pub fn pairs_of(text: &str, fields: &[String], skill: &str) -> Option<Vec<(String, String)>> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = v.as_object()?;
    let pairs = fields
        .iter()
        .filter_map(|f| {
            let text = as_text(obj.get(f)?);
            if text.is_none() {
                warn_once(
                    skill,
                    &format!("world-line field '{f}' is not text; left out"),
                );
            }
            Some((label(f), text?))
        })
        .filter(|(_, v)| !v.is_empty())
        .collect();
    Some(pairs)
}

/// A value as text. A number is printed as JSON prints it; the engine never
/// formats, rounds, compares or adds to it.
fn as_text(v: &serde_json::Value) -> Option<String> {
    let raw = match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    Some(crate::perception::activity::clean(&raw, VALUE_CHARS))
}

fn label(field: &str) -> String {
    field.replace('_', " ")
}

fn age_secs(modified: SystemTime, now: SystemTime) -> u64 {
    now.duration_since(modified)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `name: label value · label value · 4m ago`, within [`LINE_CHARS`]. Fields
/// that do not fit are dropped from the end, whole; a line is never cut
/// inside a value.
pub fn render(r: &Reading, now: SystemTime) -> Option<String> {
    let name = crate::perception::activity::clean(&r.app, NAME_CHARS);
    let age = crate::perception::activity::ago(age_secs(r.modified, now) as i64);
    let mut shown: Vec<String> = r.pairs.iter().map(|(l, v)| format!("{l} {v}")).collect();
    while !shown.is_empty() {
        let line = format!("{name}: {} · {age}", shown.join(" · "));
        if line.chars().count() <= LINE_CHARS {
            return Some(line);
        }
        shown.pop();
    }
    None
}
