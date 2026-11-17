//! What the person has already been told — one record per problem, kept by
//! the device the problem is about (`doc/perception-spec.md` §5, §6).
//!
//! A problem is said once. It is said again only when it is materially worse
//! (its owner's `level` rose) or when it went away and came back. Time passing
//! is not a reason: a fact that was true yesterday and is no worse today is not
//! news, and saying it again is how a companion becomes a notification tray.
//! And it is never dropped: a problem nobody has told is still due, at the next
//! moment the resident's own gates allow.
//!
//! The record lives with the owner. A problem the other device reported about
//! itself is told there; when this Mac says one, it keeps a *report* of having
//! said it and publishes that on the `perception` topic, so the owner can file
//! it. The report is dropped once the owner's record shows it, or once the
//! owner no longer has the problem — so a return is news again.
//!
//! The rule, [`standing`], compares integers only. What a level means — a band
//! of free space, days of room halving — is the owner's to decide; the engine
//! names no problem.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A problem that has been absent this long, on every reading in between,
/// has recovered. One reading under the line is a day the user shot no video,
/// not a disk that was cleared.
pub const RECOVERED_AFTER_SECS: i64 = 86_400;

/// When it was said, how bad it was then, and who said it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Told {
    /// Unix seconds.
    pub at: i64,
    pub level: u32,
    pub by: String,
}

/// Where a problem stands against what has been said about it.
#[derive(Debug, Clone, PartialEq)]
pub enum Standing {
    /// Never said, or said before it went away and came back.
    New,
    /// Said, and worse now than when it was.
    Worse(Told),
    /// Said, and no worse since. Not news.
    Told(Told),
}

impl Standing {
    /// Whether it is worth starting a sentence about.
    pub fn due(&self) -> bool {
        !matches!(self, Standing::Told(_))
    }

    /// How a reader is shown it, beside the problem's own facts. A fact about
    /// the telling, not an instruction: she reads "told 3h ago" and knows.
    pub fn words(&self, now: i64) -> String {
        let ago = |t: &Told| super::activity::ago(now.saturating_sub(t.at));
        match self {
            Standing::New => "not told yet".to_string(),
            Standing::Worse(t) => format!("worse than when told {}", ago(t)),
            Standing::Told(t) => format!("already told {}, no worse since", ago(t)),
        }
    }
}

/// The one rule. No clock in it: when something was said never decides
/// whether it is said again.
pub fn standing(level: u32, told: Option<&Told>) -> Standing {
    match told {
        None => Standing::New,
        Some(t) if level > t.level => Standing::Worse(t.clone()),
        Some(t) => Standing::Told(t.clone()),
    }
}

/// A problem the other device published about itself, as it crossed.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerProblem {
    pub key: String,
    pub level: u32,
    /// The facts, one line, through the reader's door.
    pub line: String,
    /// The owner's record of telling it, if any.
    pub told: Option<Told>,
}

/// One entry of the ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    #[serde(flatten)]
    told: Told,
    /// The first reading the problem was absent on, since it was last present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clear_since: Option<i64>,
}

/// This Mac's record: the problems it owns, keyed by name, and its reports
/// about the other device's, keyed `<owner>/<name>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ledger {
    entries: BTreeMap<String, Entry>,
}

fn report_key(owner: &str, key: &str) -> String {
    format!("{owner}/{key}")
}

impl Ledger {
    pub fn get(&self, key: &str) -> Option<&Told> {
        self.entries.get(key).map(|e| &e.told)
    }

    /// It was said out loud. A telling at a level no higher than the one
    /// already on record changes nothing: "told 3h ago" stays the first time.
    pub fn said(&mut self, key: &str, level: u32, by: &str, now: i64) {
        if self.get(key).is_some_and(|t| t.level >= level) {
            return;
        }
        self.entries.insert(
            key.to_string(),
            Entry {
                told: Told {
                    at: now,
                    level,
                    by: by.to_string(),
                },
                clear_since: None,
            },
        );
    }

    /// One reading of a problem this Mac owns. Present: any recovery under way
    /// is off. Absent: the record goes only once it has stayed absent for
    /// [`RECOVERED_AFTER_SECS`] — then a return is news.
    pub fn reading(&mut self, key: &str, present: bool, now: i64) {
        let Some(e) = self.entries.get_mut(key) else {
            return;
        };
        if present {
            e.clear_since = None;
            return;
        }
        match e.clear_since {
            None => e.clear_since = Some(now),
            Some(since) if now - since >= RECOVERED_AFTER_SECS => {
                self.entries.remove(key);
            }
            Some(_) => {}
        }
    }

    /// Where the other device's problem stands: against the higher of its own
    /// record and this Mac's report of having said it.
    pub fn peer_standing(&self, owner: &str, p: &PeerProblem) -> Standing {
        let ours = self.get(&report_key(owner, &p.key));
        let told = match (p.told.as_ref(), ours) {
            (Some(a), Some(b)) => Some(if b.level > a.level { b } else { a }),
            (a, b) => a.or(b),
        };
        standing(p.level, told)
    }

    /// The first of the other device's problems that is due, in its order.
    pub fn peer_due<'a>(
        &self,
        owner: &str,
        problems: &'a [PeerProblem],
    ) -> Option<(&'a PeerProblem, Standing)> {
        problems
            .iter()
            .map(|p| (p, self.peer_standing(owner, p)))
            .find(|(_, s)| s.due())
    }

    /// This Mac said the other device's problem. Kept until the owner files it.
    pub fn reported(&mut self, owner: &str, key: &str, level: u32, now: i64) {
        self.said(&report_key(owner, key), level, "mac", now);
    }

    /// Drop the reports the owner no longer needs: filed (its record is at
    /// least as high) or moot (it no longer has the problem). `None` is an
    /// owner that said nothing about problems — no peer, or a build that
    /// predates them — and settles nothing.
    pub fn prune_reports(&mut self, owner: &str, problems: Option<&[PeerProblem]>) {
        let Some(problems) = problems else {
            return;
        };
        let prefix = report_key(owner, "");
        self.entries.retain(|k, e| {
            let Some(name) = k.strip_prefix(&prefix) else {
                return true;
            };
            match problems.iter().find(|p| p.key == name) {
                None => false,
                Some(p) => !p.told.as_ref().is_some_and(|t| t.level >= e.told.level),
            }
        });
    }

    /// This Mac's reports about `owner`'s problems, as they cross.
    pub fn reports(&self, owner: &str) -> Vec<serde_json::Value> {
        let prefix = report_key(owner, "");
        self.entries
            .iter()
            .filter_map(|(k, e)| {
                let name = k.strip_prefix(&prefix)?;
                let at = chrono::DateTime::from_timestamp(e.told.at, 0)?
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                Some(serde_json::json!({
                    "key": name,
                    "owner": owner,
                    "level": e.told.level,
                    "at": at,
                    "by": e.told.by,
                }))
            })
            .collect()
    }

    /// Read a ledger from its file's text. The shape before levels — topic to
    /// the unix second it was said — reads as a telling at the first level, so
    /// an upgrade does not repeat what was already said.
    fn parse(text: &str) -> Self {
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(text).unwrap_or_default();
        let entries = map
            .into_iter()
            .filter_map(|(k, v)| {
                let entry = match v.as_i64() {
                    Some(at) => Entry {
                        told: Told {
                            at,
                            level: 1,
                            by: "mac".into(),
                        },
                        clear_since: None,
                    },
                    None => serde_json::from_value(v).ok()?,
                };
                Some((k, entry))
            })
            .collect();
        Self { entries }
    }

    fn body(&self) -> Option<Vec<u8>> {
        serde_json::to_vec_pretty(&self.entries).ok()
    }
}

/// The ledger's one file. It kept the name it had when it held only the
/// machine's conditions; it is the same record, grown.
fn path() -> PathBuf {
    super::activity::activity_dir().join("conditions.json")
}

pub fn load() -> Ledger {
    std::fs::read_to_string(path())
        .map(|t| Ledger::parse(&t))
        .unwrap_or_default()
}

pub fn store(ledger: &Ledger) {
    let dir = super::activity::activity_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Some(body) = ledger.body() {
        let _ = std::fs::write(path(), body);
    }
}

/// Bounds on what a peer may publish about its problems, at the reader's door.
const PROBLEMS_MAX: usize = 4;
const KEY_MAX: usize = 32;
const BY_MAX: usize = 16;
const LEVEL_MAX: u32 = 9;

/// The other device's `problems`, through the reader's door. `None` when the
/// payload has no such field — unknown, which is not the same as "none".
pub(crate) fn parse_problems(v: &serde_json::Value, line_max: usize) -> Option<Vec<PeerProblem>> {
    let arr = v.get("problems")?.as_array()?;
    Some(
        arr.iter()
            .take(PROBLEMS_MAX)
            .filter_map(|p| {
                let key = super::activity::clean(p.get("key")?.as_str()?, KEY_MAX);
                let line = super::activity::clean(p.get("line")?.as_str()?, line_max);
                let level = p.get("level")?.as_u64()?.min(LEVEL_MAX as u64) as u32;
                if key.is_empty() || line.is_empty() || level == 0 {
                    return None;
                }
                let told = p.get("told").and_then(parse_told);
                Some(PeerProblem {
                    key,
                    level,
                    line,
                    told,
                })
            })
            .collect(),
    )
}

fn parse_told(v: &serde_json::Value) -> Option<Told> {
    let at = chrono::DateTime::parse_from_rfc3339(v.get("at")?.as_str()?)
        .ok()?
        .timestamp();
    let level = v.get("level")?.as_u64()?.min(LEVEL_MAX as u64) as u32;
    let by = super::activity::clean(v.get("by").and_then(|b| b.as_str()).unwrap_or(""), BY_MAX);
    Some(Told { at, level, by })
}

#[cfg(test)]
pub(crate) mod tests;
