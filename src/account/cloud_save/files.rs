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
    /// Keys of one JSON file that are not progress (`cloud.ignore`).
    pub ignore: Option<Ignore>,
}

/// Top-level keys of one JSON file of a save that stamps leave behind without
/// play. Left out when asking whether a save moved or two copies agree.
#[derive(Debug, Clone)]
pub struct Ignore {
    pub file: String,
    pub keys: Vec<String>,
}

/// How many `<file>.conflict-<secs>` copies are kept beside a file.
pub const KEEP_COPIES: usize = 5;

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

    /// The fingerprint a ledger keeps: over the files as progress, with the
    /// ignored keys left out of the file that has them and its JSON read in
    /// canonical form (key order and spacing do not count). Without an ignore
    /// list it is [`Layout::fingerprint`] exactly.
    pub fn progress_fingerprint(&self, files: &Files) -> u64 {
        self.fingerprint(&self.progress_of(files))
    }

    /// Whether this layout has keys to leave out.
    pub fn has_ignore(&self) -> bool {
        self.ignore.is_some()
    }

    fn progress_of(&self, files: &Files) -> Files {
        files
            .iter()
            .map(|(rel, bytes)| (rel.clone(), self.progress_bytes(rel, bytes)))
            .collect()
    }

    /// A file's bytes as progress: for the ignore file, canonical JSON without
    /// the ignored keys; anything else (or a file that is not a JSON object)
    /// as it is.
    fn progress_bytes(&self, rel: &str, bytes: &[u8]) -> Vec<u8> {
        let Some(ig) = self.ignore.as_ref().filter(|ig| ig.file == rel) else {
            return bytes.to_vec();
        };
        match serde_json::from_slice::<Value>(bytes) {
            Ok(Value::Object(mut o)) => {
                for k in &ig.keys {
                    o.remove(k);
                }
                canonical(&Value::Object(o)).to_string().into_bytes()
            }
            _ => bytes.to_vec(),
        }
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

    /// Before `current` loses to `incoming`, each file of it that `incoming`
    /// would change or lack is copied beside itself as
    /// `<file>.conflict-<unix secs>` — the files here before a pull, or the
    /// account's copy before this machine's replaces it. A file that differs
    /// only in ignored keys is no loss and gets no copy. Each file keeps its
    /// newest [`KEEP_COPIES`] copies; older ones go. Returns the copies.
    pub fn keep_conflicts(&self, current: &Files, incoming: &Files) -> Result<Vec<PathBuf>> {
        let stamp = crate::util::now_ts_secs();
        let mut kept = Vec::new();
        for (rel, bytes) in current {
            let same = |b: &Vec<u8>| self.progress_bytes(rel, b) == self.progress_bytes(rel, bytes);
            if incoming.get(rel).is_some_and(same) {
                continue;
            }
            let file = self.root.join(rel);
            let mut name = file.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".conflict-{stamp}"));
            let copy = file.with_file_name(name);
            if let Some(dir) = copy.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("create {}", dir.display()))?;
            }
            std::fs::write(&copy, bytes).with_context(|| format!("write {}", copy.display()))?;
            prune_copies(&file);
            kept.push(copy);
        }
        Ok(kept)
    }
}

/// JSON with every object's keys in sorted order, so two readings of the same
/// content print the same text.
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let sorted: BTreeMap<&String, Value> =
                o.iter().map(|(k, v)| (k, canonical(v))).collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Delete all but the newest [`KEEP_COPIES`] of `<file>.conflict-<digits>` in
/// the file's folder — only names of exactly that pattern, only this file's.
fn prune_copies(file: &Path) {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let prefix = format!("{name}.conflict-");
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    let mut copies: Vec<(u64, PathBuf)> = listing
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().into_string().ok()?;
            let digits = n.strip_prefix(&prefix)?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Some((digits.parse().ok()?, e.path()))
        })
        .collect();
    copies.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in copies.into_iter().skip(KEEP_COPIES) {
        let _ = std::fs::remove_file(path);
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
            ignore: None,
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
            ignore: None,
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

    fn ignoring(root: &Path) -> Layout {
        Layout {
            ignore: Some(Ignore {
                file: "data/state.json".into(),
                keys: vec!["updated".into(), "last_seen".into(), "stamina".into()],
            }),
            ..bundle(root)
        }
    }

    fn one_file(rel: &str, text: &str) -> Files {
        let mut f = Files::new();
        f.insert(rel.into(), text.as_bytes().to_vec());
        f
    }

    #[test]
    fn the_progress_fingerprint_leaves_out_the_ignored_keys_order_and_spacing() {
        let l = ignoring(Path::new("/s/g"));
        let at = |t: &str| l.progress_fingerprint(&one_file("data/state.json", t));
        let base = at(r#"{"done":["a"],"tier":"qi","updated":"t1","last_seen":"t1","stamina":50}"#);
        // Stamps only, other order, other spacing, nested order: the same progress.
        assert_eq!(
            base,
            at(
                r#"{ "tier": "qi", "stamina": 7, "done": ["a"], "last_seen": "t9", "updated": "t9" }"#
            )
        );
        assert_eq!(
            at(r#"{"a":{"x":1,"y":2},"updated":1}"#),
            at(r#"{"updated":2,"a":{"y":2,"x":1}}"#)
        );
        // Real progress moves it, and so does a change in an array's order.
        assert_ne!(base, at(r#"{"done":["a","b"],"tier":"qi","updated":"t1"}"#));
        assert_ne!(
            base,
            at(r#"{"done":["a"],"tier":"foundation","updated":"t1"}"#)
        );
        assert_ne!(at(r#"{"d":["a","b"]}"#), at(r#"{"d":["b","a"]}"#));
        // Only the declared file's top-level keys are left out.
        let other = |t: &str| l.progress_fingerprint(&one_file("data/worlds/w/world.json", t));
        assert_ne!(other(r#"{"updated":1}"#), other(r#"{"updated":2}"#));
        assert_ne!(at(r#"{"n":{"updated":1}}"#), at(r#"{"n":{"updated":2}}"#));
        // A file that is not a JSON object is compared as it is.
        assert_ne!(at("not json"), at("not json "));
    }

    #[test]
    fn without_an_ignore_list_the_fingerprint_is_the_raw_one() {
        let l = bundle(Path::new("/s/g"));
        let f = one_file("data/state.json", r#"{"updated":1}"#);
        assert_eq!(l.progress_fingerprint(&f), l.fingerprint(&f));
        let g = one_file("data/state.json", r#"{ "updated": 1 }"#);
        assert_ne!(l.progress_fingerprint(&f), l.progress_fingerprint(&g));
        assert!(!l.has_ignore());
    }

    fn copies_of(dir: &Path, name: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with(name))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_loser_that_differs_only_in_ignored_keys_leaves_no_copy() {
        let root = scratch("same");
        let l = ignoring(&root);
        let here = one_file(
            "data/state.json",
            r#"{"tier":"qi","updated":"a","stamina":3}"#,
        );
        let there = one_file(
            "data/state.json",
            r#"{"stamina":90,"tier":"qi","updated":"b"}"#,
        );
        assert!(l.keep_conflicts(&here, &there).unwrap().is_empty());
        // A real difference is kept, once.
        let ahead = one_file("data/state.json", r#"{"tier":"core","updated":"b"}"#);
        write_atomic(&root.join("data/state.json"), b"x").unwrap();
        let kept = l.keep_conflicts(&here, &ahead).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read(&kept[0]).unwrap(), here["data/state.json"]);
        // …and a file the other side lacks is kept too.
        assert_eq!(l.keep_conflicts(&here, &Files::new()).unwrap().len(), 1);
    }

    #[test]
    fn only_the_newest_five_copies_of_a_file_stay() {
        let root = scratch("prune");
        let l = bundle(&root);
        let dir = root.join("data");
        for n in 1..=7u64 {
            write_atomic(&dir.join(format!("state.json.conflict-{n}")), b"old").unwrap();
        }
        // Not this engine's pattern, or another file's: never touched.
        for other in [
            "state.json.conflict-abc",
            "state.json.conflict-",
            "state.json.conflict-12x",
            "state.json.conflicts-1",
            "state.jsonx.conflict-1",
            "other.json.conflict-1",
        ] {
            write_atomic(&dir.join(other), b"keep").unwrap();
        }
        let kept = l
            .keep_conflicts(
                &one_file("data/state.json", "mine"),
                &one_file("data/state.json", "theirs"),
            )
            .unwrap();
        assert_eq!(kept.len(), 1);
        let left: Vec<String> = copies_of(&dir, "state.json.conflict-")
            .into_iter()
            .filter(|n| {
                let d = &n["state.json.conflict-".len()..];
                !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())
            })
            .collect();
        let now = kept[0].file_name().unwrap().to_str().unwrap().to_string();
        assert_eq!(left.len(), 5, "{left:?}");
        assert!(left.contains(&now));
        for n in 4..=7 {
            assert!(
                left.contains(&format!("state.json.conflict-{n}")),
                "{n} stays"
            );
        }
        for n in 1..=3 {
            assert!(
                !dir.join(format!("state.json.conflict-{n}")).exists(),
                "{n} went"
            );
        }
        for other in [
            "state.json.conflict-abc",
            "state.json.conflict-",
            "state.json.conflict-12x",
            "state.json.conflicts-1",
            "state.jsonx.conflict-1",
            "other.json.conflict-1",
        ] {
            assert!(dir.join(other).exists(), "{other} untouched");
        }
    }
}
