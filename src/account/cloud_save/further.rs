//! Which of two copies of a save is further on, by the rule its skill
//! declares (`cloud.further`, skill-spec § Cloud): one JSON file of the save,
//! compared entry by entry, the first difference deciding. The same rule the
//! phone ships and decides by, so the Mac↔cloud, phone↔cloud and phone↔Mac
//! routes all pick the same winner. Nothing here knows a skill: the keys and
//! the order are the declaration's.
//!
//!   further:
//!     {"file": "data/state.json", "by": [
//!       {"count": "done_scenes"},
//!       {"number": "version", "newest": 19},
//!       {"rank": "tier", "order": {"file": "ladder.json", "list": "tiers", "key": "id"}, "rise": true},
//!       {"number": "step", "within": ["tier", "version"]},
//!       {"time": "updated"}]}
//!
//! - `count`: the longer list wins (not a list counts 0).
//! - `rank`: the later place in `order` wins; only when both are in it.
//! - `number`: the larger wins (not a number counts -1); with `within`, only
//!   when those keys are equal on both sides.
//! - `time`: the later instant wins; only when both read as one.
//! - `newest` (on one `number` entry, never with `within`): the largest value
//!   the installed skill plays — a side past it was written by a newer build
//!   and wins before any entry is read, so an older build never writes over
//!   it. Otherwise the entry stands in its place in the order.
//!
//! A side without the file never wins; all even, the other side wins. The
//! engine cannot bring a save to the skill's version; a skill orders first
//! what reads the same at every version (2026-11-12: scenes done before the
//! save's version — a save only migrated first lost a further one).
//! `rise` marks the entries whose move up is a milestone ([`Rule::rose`]).

use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::path::Path;

use super::files::{plain, Files};

#[derive(Debug, Clone)]
enum Kind {
    Count,
    Rank(Vec<String>),
    Number,
    Time,
}

#[derive(Debug, Clone)]
struct By {
    kind: Kind,
    key: String,
    within: Vec<String>,
    rise: bool,
    /// The newest value the installed skill plays (`number` only).
    newest: Option<f64>,
}

/// A declared `cloud.further`, its orders read.
#[derive(Debug, Clone)]
pub struct Rule {
    file: String,
    by: Vec<By>,
}

/// The rule a skill declares, read against its directory. A shape that
/// cannot be read is warned about and leaves the save on the old rule (the
/// other side wins every disagreement).
pub fn rule(skill: &str, root: &Path, declared: &Value) -> Option<Rule> {
    match parse(root, declared) {
        Ok(r) => Some(r),
        Err(why) => {
            tracing::warn!(
                "skill '{skill}': cloud.further not read ({why}); the other copy wins a conflict"
            );
            None
        }
    }
}

fn parse(root: &Path, v: &Value) -> Result<Rule, String> {
    let file = v.get("file").and_then(Value::as_str).filter(|f| plain(f));
    let file = file.ok_or("`file` must be a path inside the skill")?;
    let entries = v
        .get("by")
        .and_then(Value::as_array)
        .ok_or("`by` must be a list")?;
    if entries.is_empty() {
        return Err("`by` is empty".into());
    }
    let by = entries
        .iter()
        .enumerate()
        .map(|(i, e)| entry(root, e).map_err(|w| format!("by[{i}]: {w}")))
        .collect::<Result<Vec<By>, _>>()?;
    if by.iter().filter(|b| b.newest.is_some()).count() > 1 {
        return Err("one entry with `newest` at most".into());
    }
    Ok(Rule {
        file: file.to_string(),
        by,
    })
}

fn entry(root: &Path, e: &Value) -> Result<By, String> {
    let named: Vec<&str> = ["count", "rank", "number", "time"]
        .into_iter()
        .filter(|k| e.get(k).is_some())
        .collect();
    let [kind] = named[..] else {
        return Err("exactly one of count, rank, number, time".into());
    };
    let key = e[kind].as_str().filter(|k| !k.is_empty()).ok_or("a key")?;
    let kind = match kind {
        "count" => Kind::Count,
        "rank" => Kind::Rank(order(root, e.get("order").ok_or("rank without order")?)?),
        "number" => Kind::Number,
        _ => Kind::Time,
    };
    let within = match e.get("within") {
        None => Vec::new(),
        Some(w) => w
            .as_array()
            .and_then(|a| a.iter().map(|k| k.as_str().map(String::from)).collect())
            .ok_or("`within` must be a list of keys")?,
    };
    let newest = match e.get("newest") {
        None => None,
        Some(n) if matches!(kind, Kind::Number) && within.is_empty() => {
            Some(n.as_f64().ok_or("`newest` must be a number")?)
        }
        Some(_) => return Err("`newest` only on a `number` without `within`".into()),
    };
    Ok(By {
        kind,
        key: key.to_string(),
        within,
        rise: e.get("rise").and_then(Value::as_bool).unwrap_or(false),
        newest,
    })
}

/// `order: {file, list, key?}` — the list at `list` (dotted) in a JSON file
/// of the skill, each item's `key`, or the item itself.
fn order(root: &Path, o: &Value) -> Result<Vec<String>, String> {
    let file = o.get("file").and_then(Value::as_str).filter(|f| plain(f));
    let file = file.ok_or("order.file must be a path inside the skill")?;
    let list = o.get("list").and_then(Value::as_str).ok_or("order.list")?;
    let text = std::fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| format!("{file}: {e}"))?;
    let items = at(&doc, list)
        .and_then(Value::as_array)
        .ok_or(format!("{file}: `{list}` is not a list"))?;
    let key = o.get("key").and_then(Value::as_str);
    Ok(items
        .iter()
        .map(|x| text_of(key.map_or(x, |k| x.get(k).unwrap_or(&Value::Null))))
        .collect())
}

/// A value at a dotted path.
fn at<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |v, k| v.get(k))
}

/// A value as a rank names it: a string as itself, anything else as JSON.
fn text_of(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), String::from)
}

fn time(v: Option<&Value>) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(v?.as_str()?).ok()
}

impl By {
    /// How `a` stands against `b` on this entry; `None` when it does not decide.
    fn compare(&self, a: &Map<String, Value>, b: &Map<String, Value>) -> Option<Ordering> {
        let get = |m: &Map<String, Value>, k: &str| at_map(m, k).cloned().unwrap_or(Value::Null);
        if !self.within.iter().all(|k| get(a, k) == get(b, k)) {
            return None;
        }
        let (va, vb) = (at_map(a, &self.key), at_map(b, &self.key));
        let ord = match &self.kind {
            Kind::Count => {
                let n = |v: Option<&Value>| v.and_then(Value::as_array).map_or(0, Vec::len);
                n(va).cmp(&n(vb))
            }
            Kind::Rank(order) => {
                let i = |v: Option<&Value>| {
                    let t = text_of(v.unwrap_or(&Value::Null));
                    order.iter().position(|o| *o == t)
                };
                i(va)?.cmp(&i(vb)?)
            }
            Kind::Number => {
                let n = |v: Option<&Value>| v.and_then(Value::as_f64).unwrap_or(-1.0);
                n(va).partial_cmp(&n(vb))?
            }
            Kind::Time => time(va)?.cmp(&time(vb)?),
        };
        (ord != Ordering::Equal).then_some(ord)
    }

    /// With `newest`: how `a` stands against `b` when one side is past it.
    fn past_newest(&self, a: &Map<String, Value>, b: &Map<String, Value>) -> Option<Ordering> {
        let newest = self.newest?;
        let past = |m: &Map<String, Value>| {
            at_map(m, &self.key)
                .and_then(Value::as_f64)
                .is_some_and(|v| v > newest)
        };
        let (pa, pb) = (past(a), past(b));
        (pa != pb).then_some(if pa {
            Ordering::Greater
        } else {
            Ordering::Less
        })
    }
}

fn at_map<'a>(m: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let (first, rest) = path
        .split_once('.')
        .map_or((path, None), |(f, r)| (f, Some(r)));
    let v = m.get(first)?;
    match rest {
        None => Some(v),
        Some(r) => at(v, r),
    }
}

impl Rule {
    /// The save's file this rule reads.
    pub fn file(&self) -> &str {
        &self.file
    }

    fn state(&self, files: &Files) -> Option<Map<String, Value>> {
        let bytes = files.get(&self.file)?;
        match serde_json::from_slice(bytes) {
            Ok(Value::Object(m)) => Some(m),
            _ => None,
        }
    }

    /// Whether `here` is further on than `there`. A tie is not.
    pub fn further(&self, here: &Files, there: &Files) -> bool {
        let Some(a) = self.state(here) else {
            return false;
        };
        let Some(b) = self.state(there) else {
            return true;
        };
        self.by
            .iter()
            .find_map(|e| e.past_newest(&a, &b))
            .or_else(|| self.by.iter().find_map(|e| e.compare(&a, &b)))
            .is_some_and(|o| o == Ordering::Greater)
    }

    /// What the `rise` entries read in `files` — the keys they compare and
    /// their `within` — kept to tell a later copy's rise by ([`Rule::rose`]).
    pub fn standing(&self, files: &Files) -> Option<Value> {
        let s = self.state(files)?;
        let mut out = Map::new();
        for e in self.by.iter().filter(|e| e.rise) {
            for k in std::iter::once(&e.key).chain(&e.within) {
                if let Some(v) = at_map(&s, k) {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        Some(Value::Object(out))
    }

    /// Whether a standing moved up a `rise` entry since `before`.
    pub fn rose(&self, before: &Value, now: &Value) -> bool {
        let (Some(b), Some(n)) = (before.as_object(), now.as_object()) else {
            return false;
        };
        self.by
            .iter()
            .filter(|e| e.rise)
            .find_map(|e| e.compare(n, b))
            .is_some_and(|o| o == Ordering::Greater)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The rule Lingjing declares (skills/lingjing/SKILL.md `cloud.further`), with a ladder of three.
    fn lingjing() -> Rule {
        // One folder per call: tests run in parallel in one process, and a shared
        // `further-<pid>` let one test's write land under another's read.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("further-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("ladder.json"),
            r#"{"tiers": [{"id": "qi"}, {"id": "foundation"}, {"id": "core"}]}"#,
        )
        .unwrap();
        let declared = json!({"file": "data/state.json", "by": [
            {"count": "done_scenes"},
            {"number": "version", "newest": 19},
            {"rank": "tier", "order": {"file": "ladder.json", "list": "tiers", "key": "id"}, "rise": true},
            {"number": "step", "within": ["tier", "version"], "rise": true},
            {"number": "progress", "within": ["tier", "version"]},
            {"time": "story_at"},
            {"time": "updated"}
        ]});
        rule("g", &root, &declared).unwrap()
    }

    fn files(state: Value) -> Files {
        let mut f = Files::new();
        f.insert("data/state.json".into(), state.to_string().into_bytes());
        f
    }

    fn save(scenes: usize, updated: &str) -> Files {
        files(json!({"done_scenes": vec!["s"; scenes], "updated": updated}))
    }

    fn grind(tier: &str, step: i64, progress: f64, updated: &str, version: i64) -> Files {
        files(
            json!({"done_scenes": ["a", "b"], "tier": tier, "step": step, "progress": progress,
            "version": version, "updated": updated}),
        )
    }

    const EARLY: &str = "2026-11-03T09:00:00Z";
    const LATE: &str = "2026-11-03T10:00:00Z";

    // The phone's cases (linggen-mobile test/lingjing_sync_test.dart), kept in step.
    #[test]
    fn more_scenes_wins_then_the_later_write() {
        let r = lingjing();
        assert!(r.further(&save(7, EARLY), &save(4, LATE)));
        assert!(!r.further(&save(4, LATE), &save(7, EARLY)));
        assert!(r.further(&save(2, LATE), &save(2, EARLY)));
        assert!(!r.further(&save(2, EARLY), &save(2, LATE)));
    }

    #[test]
    fn the_later_story_move_before_the_later_write() {
        let s = |story: &str, updated: &str| {
            files(json!({"done_scenes": ["a"], "story_at": story, "updated": updated}))
        };
        assert!(lingjing().further(
            &s("2026-10-27T11:00:00Z", "2026-10-27T11:00:00Z"),
            &s("2026-10-27T10:00:00Z", "2026-10-27T12:00:00Z")
        ));
    }

    #[test]
    fn total_progress_realm_then_layer_then_xiuwei_over_the_later_write() {
        let r = lingjing();
        assert!(r.further(
            &grind("foundation", 0, 10.0, EARLY, 15),
            &grind("qi", 8, 900.0, LATE, 15)
        ));
        assert!(r.further(
            &grind("qi", 5, 10.0, EARLY, 15),
            &grind("qi", 4, 900.0, LATE, 15)
        ));
        assert!(r.further(
            &grind("qi", 5, 300.0, EARLY, 15),
            &grind("qi", 5, 120.0, LATE, 15)
        ));
        assert!(!r.further(
            &grind("qi", 5, 120.0, LATE, 15),
            &grind("qi", 5, 300.0, EARLY, 15)
        ));
        assert!(r.further(
            &grind("qi", 5, 120.0, LATE, 15),
            &grind("qi", 5, 120.0, EARLY, 15)
        ));
        // Another save version at even scenes: the newer version decides (the
        // layer is read on another ladder there), never the later write.
        assert!(!r.further(
            &grind("qi", 2, 10.0, LATE, 14),
            &grind("qi", 5, 900.0, EARLY, 15)
        ));
        assert!(r.further(
            &grind("qi", 5, 900.0, EARLY, 15),
            &grind("qi", 2, 10.0, LATE, 14)
        ));
        // A realm the ladder does not hold is not compared.
        assert!(!r.further(
            &grind("void", 0, 10.0, EARLY, 15),
            &grind("qi", 8, 900.0, LATE, 15)
        ));
    }

    /// 2026-11-12, by the keys the rule reads: the phone's v18 save, 116
    /// scenes on, against the Mac's 105, migrated to v19 seconds before.
    fn incident(version: i64, scenes: usize, progress: f64, story: &str) -> Files {
        files(
            json!({"version": version, "done_scenes": vec!["s"; scenes], "tier": "foundation",
            "step": 0, "progress": progress, "story_at": story, "updated": story}),
        )
    }

    #[test]
    fn a_save_further_on_in_play_beats_one_only_migrated_first() {
        let r = lingjing();
        let phone = incident(18, 116, 383.0, "2026-11-12T23:19:48Z");
        let mac = incident(19, 105, 56.0, "2026-11-12T20:57:49Z");
        // Mac↔cloud (the engine both ways round): the phone's copy wins.
        assert!(r.further(&phone, &mac));
        assert!(!r.further(&mac, &phone));
        // The opposite: the migrated side really further on.
        let mac = incident(19, 120, 56.0, "2026-11-12T20:57:49Z");
        assert!(r.further(&mac, &phone));
        assert!(!r.further(&phone, &mac));
    }

    #[test]
    fn a_save_past_the_newest_version_wins_outright() {
        let r = lingjing();
        let newer = incident(20, 3, 0.0, "2026-11-12T01:00:00Z");
        let here = incident(19, 116, 383.0, "2026-11-12T23:00:00Z");
        assert!(
            r.further(&newer, &here),
            "an older build never writes over it"
        );
        assert!(!r.further(&here, &newer));
        // Both past it: the order decides as ever.
        assert!(r.further(&incident(21, 9, 0.0, EARLY), &incident(20, 3, 0.0, EARLY)));
    }

    #[test]
    fn no_file_never_wins_and_a_tie_goes_to_the_other_side() {
        let r = lingjing();
        assert!(!r.further(&Files::new(), &save(1, EARLY)));
        assert!(r.further(&save(0, EARLY), &Files::new()));
        assert!(!r.further(&save(3, EARLY), &save(3, EARLY)));
    }

    #[test]
    fn a_rise_is_a_move_up_a_marked_entry_only() {
        let r = lingjing();
        let at = |f: &Files| r.standing(f).unwrap();
        let before = at(&grind("qi", 3, 100.0, EARLY, 15));
        assert!(
            !r.rose(&before, &at(&grind("qi", 3, 900.0, LATE, 15))),
            "修为 alone is no milestone"
        );
        assert!(r.rose(&before, &at(&grind("qi", 4, 0.0, LATE, 15))));
        assert!(r.rose(&before, &at(&grind("foundation", 0, 0.0, LATE, 15))));
        assert!(!r.rose(&before, &at(&grind("qi", 2, 0.0, LATE, 15))));
    }

    #[test]
    fn a_shape_the_engine_cannot_read_is_refused() {
        let root = Path::new("/nowhere");
        for bad in [
            json!({"file": "../x.json", "by": [{"count": "a"}]}),
            json!({"file": "s.json", "by": []}),
            json!({"file": "s.json", "by": [{"count": "a", "time": "b"}]}),
            json!({"file": "s.json", "by": [{"rank": "tier"}]}),
            json!({"file": "s.json", "by": [{"rank": "tier", "order": {"file": "none.json", "list": "x"}}]}),
            json!({"file": "s.json", "by": [{"count": "a", "newest": 3}]}),
            json!({"file": "s.json", "by": [{"number": "v", "newest": "x"}]}),
            json!({"file": "s.json", "by": [{"number": "v", "within": ["t"], "newest": 3}]}),
            json!({"file": "s.json", "by": [{"number": "v", "newest": 3}, {"number": "w", "newest": 3}]}),
        ] {
            assert!(parse(root, &bad).is_err(), "{bad}");
        }
    }
}
