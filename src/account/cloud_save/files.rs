//! A save's files as one value: read off the skill directory, fingerprinted,
//! turned into the body the site stores, and written back from one.
//!
//! The single-file form (`save: data/state.json`) keeps the body it always
//! had — the file's text as a JSON string. The list form stores a bundle:
//! `{"bundle": 1, "files": {"<rel path>": {"text": …} | {"b64": …}}}`.

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

/// Relative path (`/`-separated, inside the skill) → bytes. Sorted, so a
/// fingerprint never depends on the order the disk listed them in.
pub type Files = BTreeMap<String, Vec<u8>>;

/// What a save covers on disk.
#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
    /// The declared paths, each checked to stay inside `root`.
    pub entries: Vec<String>,
    /// The single-file form: the body is the file's text.
    pub one: bool,
    /// File or folder names left out at any depth (`cloud.skip`).
    pub skip: Vec<String>,
}

/// Files the engine and the scripts leave beside a save — never part of it.
fn is_scratch(name: &str) -> bool {
    name.ends_with(".lock") || name.ends_with(".tmp") || name.contains(".conflict-")
}

/// A relative path made only of plain names: nothing absolute, no `..`.
pub fn plain(rel: &str) -> bool {
    !rel.is_empty()
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

impl Layout {
    /// The first path named: where the lock and the conflict copies of the
    /// single-file form sit.
    pub fn first(&self) -> PathBuf {
        self.root.join(&self.entries[0])
    }

    /// Whether `rel` falls under what the save declares.
    fn covers(&self, rel: &str) -> bool {
        plain(rel)
            && !rel.split('/').any(|name| self.skips(name))
            && self.entries.iter().any(|e| {
                rel == e
                    || rel
                        .strip_prefix(e.as_str())
                        .is_some_and(|r| r.starts_with('/'))
            })
    }

    /// A name the save leaves on the device.
    fn skips(&self, name: &str) -> bool {
        self.skip.iter().any(|s| s == name)
    }

    /// Every file the save covers, as it is on disk now.
    pub fn read(&self) -> Files {
        let mut files = Files::new();
        for entry in &self.entries {
            self.collect(&self.root.join(entry), &mut files);
        }
        files
    }

    fn collect(&self, path: &Path, files: &mut Files) {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return;
        };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if self.skips(name) {
            return;
        }
        if meta.is_file() && !is_scratch(name) {
            let rel = path.strip_prefix(&self.root).ok().and_then(Path::to_str);
            if let (Some(rel), Ok(bytes)) = (rel, std::fs::read(path)) {
                files.insert(rel.replace('\\', "/"), bytes);
            }
        } else if meta.is_dir() {
            let Ok(entries) = std::fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                self.collect(&entry.path(), files);
            }
        }
    }

    pub fn fingerprint(&self, files: &Files) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if self.one {
            // The single form's fingerprint predates bundles: the file's text.
            let text = files.values().next().map(|b| String::from_utf8_lossy(b));
            text.as_deref().unwrap_or("").hash(&mut h);
        } else {
            files.hash(&mut h);
        }
        h.finish()
    }

    pub fn encode(&self, files: &Files) -> Value {
        if self.one {
            let text = files
                .values()
                .next()
                .map(|b| String::from_utf8_lossy(b).into_owned());
            return Value::String(text.unwrap_or_default());
        }
        let entries: Map<String, Value> = files
            .iter()
            .map(|(rel, bytes)| (rel.clone(), encode_file(bytes)))
            .collect();
        json!({ "bundle": 1, "files": entries })
    }

    /// The files a body carries. A bundle entry outside what the save
    /// declares is dropped, never written. A plain text body under the list
    /// form is a save from before bundles: it is the first path's text.
    pub fn decode(&self, body: &Value) -> Result<Files> {
        let mut files = Files::new();
        match body {
            Value::Null => bail!("the account's save is empty"),
            Value::Object(o) if !self.one && o.get("bundle").is_some() => {
                let listed = o
                    .get("files")
                    .and_then(Value::as_object)
                    .context("bundle without files")?;
                for (rel, file) in listed {
                    if !self.covers(rel) {
                        tracing::warn!("save bundle: '{rel}' is outside the save; dropped");
                        continue;
                    }
                    files.insert(rel.clone(), decode_file(file)?);
                }
            }
            Value::String(s) => {
                files.insert(self.entries[0].clone(), s.clone().into_bytes());
            }
            other => {
                files.insert(self.entries[0].clone(), other.to_string().into_bytes());
            }
        }
        Ok(files)
    }

    /// Make the disk hold exactly `files`. What the save covered here and the
    /// body lacks is removed — a world deleted on another device stays gone.
    pub fn write(&self, current: &Files, files: &Files) -> Result<()> {
        for (rel, bytes) in files {
            if current.get(rel) != Some(bytes) {
                write_atomic(&self.root.join(rel), bytes)?;
            }
        }
        for rel in current.keys().filter(|r| !files.contains_key(*r)) {
            let _ = std::fs::remove_file(self.root.join(rel));
        }
        Ok(())
    }

    /// Before a pull replaces changes made here, each file it would change or
    /// remove is copied beside itself as `<file>.conflict-<unix secs>`.
    /// Returns the copies made.
    pub fn keep_conflicts(&self, current: &Files, incoming: &Files) -> Result<Vec<PathBuf>> {
        let stamp = crate::util::now_ts_secs();
        let mut kept = Vec::new();
        for (rel, bytes) in current {
            if incoming.get(rel) == Some(bytes) {
                continue;
            }
            let file = self.root.join(rel);
            let mut name = file.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".conflict-{stamp}"));
            let copy = file.with_file_name(name);
            std::fs::write(&copy, bytes).with_context(|| format!("write {}", copy.display()))?;
            kept.push(copy);
        }
        Ok(kept)
    }
}

fn encode_file(bytes: &[u8]) -> Value {
    match std::str::from_utf8(bytes) {
        Ok(text) => json!({ "text": text }),
        Err(_) => json!({ "b64": base64::engine::general_purpose::STANDARD.encode(bytes) }),
    }
}

fn decode_file(file: &Value) -> Result<Vec<u8>> {
    if let Some(text) = file.get("text").and_then(Value::as_str) {
        return Ok(text.as_bytes().to_vec());
    }
    let b64 = file
        .get("b64")
        .and_then(Value::as_str)
        .context("bundle file without text or b64")?;
    Ok(base64::engine::general_purpose::STANDARD.decode(b64)?)
}

pub fn write_atomic(file: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".cloud-{}.tmp", std::process::id()));
    let tmp = file.with_file_name(name);
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, file).with_context(|| format!("replace {}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("save-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bundle(root: &Path) -> Layout {
        Layout {
            root: root.to_path_buf(),
            entries: vec!["data/state.json".into(), "data/worlds".into()],
            one: false,
            skip: Vec::new(),
        }
    }

    #[test]
    fn a_bundle_round_trips_text_and_bytes_and_skips_scratch() {
        let root = scratch("round");
        let l = bundle(&root);
        write_atomic(&root.join("data/state.json"), b"{\"a\":1}").unwrap();
        write_atomic(&root.join("data/worlds/w/world.json"), b"{}").unwrap();
        write_atomic(&root.join("data/worlds/w/art.png"), &[0xff, 0x00, 0x9c]).unwrap();
        write_atomic(&root.join("data/state.json.lock"), b"1").unwrap();
        write_atomic(&root.join("data/worlds/w/world.json.conflict-9"), b"old").unwrap();
        write_atomic(&root.join("data/other.json"), b"not saved").unwrap();
        let files = l.read();
        assert_eq!(
            files.keys().cloned().collect::<Vec<_>>(),
            vec![
                "data/state.json",
                "data/worlds/w/art.png",
                "data/worlds/w/world.json"
            ]
        );
        let body = l.encode(&files);
        assert_eq!(l.decode(&body).unwrap(), files);
    }

    #[test]
    fn a_skipped_name_stays_on_the_device_through_push_and_pull() {
        let root = scratch("skip");
        let l = Layout {
            skip: vec!["art".into()],
            ..bundle(&root)
        };
        write_atomic(&root.join("data/state.json"), b"{}").unwrap();
        write_atomic(&root.join("data/worlds/w/world.json"), b"{}").unwrap();
        write_atomic(&root.join("data/worlds/w/art/kui.webp"), &[0xff, 0x00]).unwrap();
        let here = l.read();
        assert_eq!(
            here.keys().cloned().collect::<Vec<_>>(),
            vec!["data/state.json", "data/worlds/w/world.json"],
            "the picture is never pushed"
        );
        // A body that names a skipped file is not written; a pull of a body
        // without it does not remove the device's copy.
        let body = json!({ "bundle": 1, "files": {
            "data/state.json": { "text": "{}" },
            "data/worlds/w/art/other.webp": { "b64": "AAA=" },
        }});
        let incoming = l.decode(&body).unwrap();
        assert!(!incoming.contains_key("data/worlds/w/art/other.webp"));
        l.write(&here, &incoming).unwrap();
        assert!(root.join("data/worlds/w/art/kui.webp").exists());
        assert!(!root.join("data/worlds/w/art/other.webp").exists());
    }

    #[test]
    fn a_bundle_never_writes_outside_the_save() {
        let l = bundle(Path::new("/s/g"));
        let body = json!({ "bundle": 1, "files": {
            "data/state.json": { "text": "ok" },
            "../../etc/x": { "text": "no" },
            "data/worldsX/y": { "text": "no" },
            "/abs": { "text": "no" },
        }});
        let files = l.decode(&body).unwrap();
        assert_eq!(
            files.keys().cloned().collect::<Vec<_>>(),
            vec!["data/state.json"]
        );
    }

    #[test]
    fn a_save_from_before_bundles_is_the_first_paths_text() {
        let l = bundle(Path::new("/s/g"));
        let files = l.decode(&Value::String("{\"old\":true}".into())).unwrap();
        assert_eq!(files.get("data/state.json").unwrap(), b"{\"old\":true}");
    }

    #[test]
    fn the_single_form_keeps_its_text_body_and_fingerprint() {
        let l = Layout {
            root: "/s/g".into(),
            entries: vec!["data/state.json".into()],
            one: true,
            skip: Vec::new(),
        };
        let mut files = Files::new();
        files.insert("data/state.json".into(), b"hello".to_vec());
        assert_eq!(l.encode(&files), Value::String("hello".into()));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        "hello".hash(&mut h);
        assert_eq!(
            l.fingerprint(&files),
            h.finish(),
            "ledgers written before bundles still agree"
        );
    }

    #[test]
    fn a_pull_mirrors_the_body_and_keeps_what_it_replaces() {
        let root = scratch("pull");
        let l = bundle(&root);
        write_atomic(&root.join("data/state.json"), b"mine").unwrap();
        write_atomic(&root.join("data/worlds/gone/world.json"), b"{}").unwrap();
        let current = l.read();
        let mut incoming = Files::new();
        incoming.insert("data/state.json".into(), b"theirs".to_vec());
        let kept = l.keep_conflicts(&current, &incoming).unwrap();
        assert_eq!(kept.len(), 2);
        l.write(&current, &incoming).unwrap();
        assert_eq!(l.read(), incoming);
        assert!(kept.iter().all(|k| k.exists()));
        assert_eq!(std::fs::read(&kept[0]).unwrap(), b"mine");
    }
}
