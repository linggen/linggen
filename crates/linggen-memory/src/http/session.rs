//! `POST /api/memory/session_start` — what every host loads when a session
//! begins, rendered once, in the daemon, so every host injects the same text:
//!
//! - **core** — who the user is (`tier = core`). Every session.
//! - **candidates** — given a `cwd`, the scopes a row written here may take
//!   (see `memory::scope`), as one line the model reads once.
//! - **index** — given a `cwd`, the summaries of `indexed` rows filed at that
//!   directory or a parent, nearest first, within [`INDEX_BUDGET_CHARS`].
//!   One line each; the agent reads a row in full with `memory_get`.
//!
//! Everything else surfaces by subject through per-turn recall. A host that
//! sends no `cwd` gets core alone; `budget_chars` (sent by 1.9.0-era hosts)
//! is ignored.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::memory::scope;
use crate::memory::{AccountScope, Filters, Memory, RecallScope, SortOrder, Tier};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Core is meant to be a handful of rows; the cap keeps a runaway tag from
/// inflating every session.
const CORE_LIMIT: usize = 200;

/// The index's share of a session's start, in characters, across every
/// directory. Nearest directories fill it first.
pub const INDEX_BUDGET_CHARS: usize = 3000;

/// Indexed rows fetched per session start before the budget cut.
const INDEX_FETCH: usize = 500;

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/memory/session_start", post(session_start))
}

#[derive(Debug, Deserialize)]
pub struct SessionStartRequest {
    #[serde(default)]
    pub account: Option<String>,
    /// The session's working directory. Absent = core only.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The session's root (git root, or where it started). Absent = found
    /// from `cwd`.
    #[serde(default)]
    pub root: Option<String>,
    /// Include the core rows. A skill's own session asks for its index
    /// alone (`false`) — core is the person, and an app's prompt stays lean.
    #[serde(default = "yes")]
    pub core: bool,
}

fn yes() -> bool {
    true
}

impl Default for SessionStartRequest {
    fn default() -> Self {
        Self {
            account: None,
            cwd: None,
            root: None,
            core: true,
        }
    }
}

async fn session_start(
    State(state): State<SharedState>,
    Json(req): Json<SessionStartRequest>,
) -> Result<Response, ApiError> {
    let account = AccountScope::from_args(req.account.clone(), false);
    let core = if req.core {
        state
            .store
            .list(
                &Filters {
                    tier: Some(Tier::Core),
                    account: account.clone(),
                    ..Default::default()
                },
                SortOrder::Newest,
                CORE_LIMIT,
                0,
            )
            .await?
    } else {
        Vec::new()
    };
    let place = Place::of(req.cwd.as_deref(), req.root.as_deref());
    let mut index = Vec::new();
    if let Some(p) = &place {
        let filters = Filters {
            indexed: Some(true),
            scope_in: p
                .chain
                .iter()
                .map(|d| d.to_string_lossy().to_string())
                .collect(),
            account,
            ..Default::default()
        };
        for store in [&state.store, &state.episodic] {
            index.extend(
                store
                    .list(&filters, SortOrder::Newest, INDEX_FETCH, 0)
                    .await?,
            );
        }
    }
    Ok(ok(
        SessionStart::build(core, index, place.as_ref()).to_json()
    ))
}

/// Where a session stands, resolved from the paths its host sent. `None`
/// when it stands nowhere a row can be about (`$HOME`, `~/.linggen`, temp).
pub struct Place {
    pub home: PathBuf,
    pub cwd: PathBuf,
    pub root: PathBuf,
    /// `cwd` and its parents that can hold rows, nearest first.
    pub chain: Vec<PathBuf>,
}

impl Place {
    pub fn of(cwd: Option<&str>, root: Option<&str>) -> Option<Self> {
        let home = scope::home();
        let cwd = scope::expand(cwd?, &home)?;
        let root = root
            .and_then(|r| scope::expand(r, &home))
            .unwrap_or_else(|| scope::find_root(&cwd, &home));
        if RecallScope::of(&root.to_string_lossy(), &home) == RecallScope::NoRoot {
            return None;
        }
        let chain = scope::index_chain(&cwd, &home);
        Some(Self {
            home,
            cwd,
            root,
            chain,
        })
    }

    fn display(&self, p: &Path) -> String {
        scope::display(p, &self.root, &self.home)
    }
}

fn line(row: &Memory) -> String {
    format!("- {} (id={})", row.content.trim(), row.id)
}

/// One index line: the summary, or the content's opening when a row has none.
fn index_line(row: &Memory) -> String {
    let text = row.summary.clone().unwrap_or_else(|| {
        let flat = row.content.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.chars().count() <= 80 {
            flat
        } else {
            format!("{}…", flat.chars().take(79).collect::<String>())
        }
    });
    format!("- {text} (id={})", row.id)
}

pub struct SessionStart {
    pub core: Vec<Memory>,
    pub index: Vec<Memory>,
    pub core_block: String,
    pub candidates: Vec<(String, String)>,
    pub candidates_line: String,
    pub index_block: String,
    pub skipped: usize,
    pub block: String,
}

impl SessionStart {
    pub fn build(core: Vec<Memory>, index: Vec<Memory>, place: Option<&Place>) -> Self {
        let core_block = render(&core);
        let (candidates, candidates_line) = match place {
            Some(p) => (
                scope::candidates(&p.root, &p.cwd, &p.home)
                    .iter()
                    .map(|c| (c.to_string_lossy().to_string(), p.display(c)))
                    .collect(),
                scope::candidates_line(&p.root, &p.cwd, &p.home),
            ),
            None => (Vec::new(), String::new()),
        };
        let (index, index_block, skipped) = match place {
            Some(p) => render_index(index, p),
            None => (Vec::new(), String::new(), 0),
        };
        let block = [&core_block, &candidates_line, &index_block]
            .iter()
            .filter(|b| !b.is_empty())
            .map(|b| b.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        Self {
            core,
            index,
            core_block,
            candidates,
            candidates_line,
            index_block,
            skipped,
            block,
        }
    }

    pub fn to_json(&self) -> Value {
        let public = |rows: &[Memory]| -> Vec<Value> {
            rows.iter()
                .map(|r| {
                    let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
                    if let Some(o) = v.as_object_mut() {
                        o.remove("vector");
                    }
                    v
                })
                .collect()
        };
        let candidates: Vec<Value> = self
            .candidates
            .iter()
            .map(|(path, shown)| json!({"path": path, "display": shown}))
            .collect();
        json!({
            "core": public(&self.core),
            "index": public(&self.index),
            "candidates": candidates,
            "candidates_line": self.candidates_line,
            "core_block": self.core_block,
            "index_block": self.index_block,
            "index_skipped": self.skipped,
            "block": self.block,
            "chars": self.block.chars().count(),
        })
    }
}

fn render(core: &[Memory]) -> String {
    if core.is_empty() {
        return String::new();
    }
    let rows: Vec<String> = core.iter().map(line).collect();
    format!("## Core memory — who the user is\n\n{}", rows.join("\n"))
}

/// The index, nearest directory first, newest first within one; cut at
/// [`INDEX_BUDGET_CHARS`]. Returns the rows shown, the block and how many
/// rows the budget skipped.
fn render_index(mut rows: Vec<Memory>, place: &Place) -> (Vec<Memory>, String, usize) {
    let depth = |r: &Memory| {
        r.scope
            .as_deref()
            .and_then(|c| place.chain.iter().position(|d| d.to_string_lossy() == c))
            .unwrap_or(usize::MAX)
    };
    rows.retain(|r| depth(r) != usize::MAX);
    rows.sort_by(|a, b| {
        depth(a)
            .cmp(&depth(b))
            .then(b.activity_timestamp().cmp(&a.activity_timestamp()))
    });
    rows.dedup_by(|a, b| a.id == b.id);
    let mut shown: Vec<Memory> = Vec::new();
    let mut out = String::new();
    let mut used = 0usize;
    let mut current: Option<String> = None;
    let mut skipped = 0usize;
    for row in rows {
        let dir = row.scope.clone().unwrap_or_default();
        let heading = (current.as_deref() != Some(dir.as_str()))
            .then(|| format!("## Index — {}\n", place.display(Path::new(&dir))));
        let entry = format!("{}\n", index_line(&row));
        let cost = heading.as_ref().map_or(0, |h| h.chars().count() + 1) + entry.chars().count();
        if used + cost > INDEX_BUDGET_CHARS {
            skipped += 1;
            continue;
        }
        if let Some(h) = heading {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&h);
            current = Some(dir);
        }
        out.push_str(&entry);
        used += cost;
        shown.push(row);
    }
    if shown.is_empty() {
        return (shown, String::new(), skipped);
    }
    out.push_str("\nThese are standing rows filed for this directory: read one in full with memory_get(id) when its summary bears on the task.");
    if skipped > 0 {
        out.push_str(&format!(
            "\n({skipped} more indexed row(s) over the {INDEX_BUDGET_CHARS}-char budget — memory_search reaches them.)"
        ));
    }
    (shown, out, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryType, Origin};

    #[test]
    fn the_block_is_core_rows_with_ids() {
        let mut core = Memory::new("Alex — founder", MemoryType::Fact, Origin::User);
        core.tier = Tier::Core;
        let s = SessionStart::build(vec![core.clone()], vec![], None);
        assert_eq!(
            s.block,
            format!(
                "## Core memory — who the user is\n\n- Alex — founder (id={})",
                core.id
            )
        );
        let json = s.to_json();
        assert_eq!(json["chars"], json!(s.block.chars().count()));
        assert!(json.get("rules").is_none());
        assert!(json["core"][0].get("vector").is_none());
    }

    #[test]
    fn an_old_host_sending_cwd_and_budget_still_parses() {
        let req: SessionStartRequest =
            serde_json::from_value(json!({"cwd": "/u/w/p", "budget_chars": 6000})).unwrap();
        assert!(req.account.is_none());
        assert!(req.core);
    }

    fn place(cwd: &str) -> Place {
        let home = PathBuf::from("/Users/alex");
        let cwd = PathBuf::from(cwd);
        Place {
            chain: scope::index_chain(&cwd, &home),
            root: PathBuf::from("/Users/alex/w/linggen/skills"),
            cwd,
            home,
        }
    }

    fn indexed(content: &str, cwd: &str, summary: Option<&str>) -> Memory {
        let mut m = Memory::new(content, MemoryType::Preference, Origin::User);
        m.scope = Some(cwd.into());
        m.summary = summary.map(str::to_string);
        m.indexed = true;
        m
    }

    #[test]
    fn the_index_is_nearest_first_under_dir_headings() {
        let p = place("/Users/alex/w/linggen/skills/lingjing");
        let far = indexed(
            "commit on main",
            "/Users/alex/w",
            Some("commit straight to main"),
        );
        let near = indexed(
            "rules in DESIGN",
            "/Users/alex/w/linggen/skills/lingjing",
            Some("写作规则见 DESIGN.md"),
        );
        let elsewhere = indexed("sanji", "/Users/alex/w/rust/sanji", Some("sanji"));
        let (shown, block, skipped) = render_index(vec![far.clone(), elsewhere, near.clone()], &p);
        assert_eq!(shown.len(), 2);
        assert_eq!(skipped, 0);
        let near_at = block.find("## Index — skills/lingjing").unwrap();
        let far_at = block.find("## Index — ~/w").unwrap();
        assert!(near_at < far_at, "{block}");
        assert!(block.contains(&format!("- 写作规则见 DESIGN.md (id={})", near.id)));
        assert!(!block.contains("sanji"));
    }

    #[test]
    fn the_index_respects_its_budget() {
        let p = place("/Users/alex/w/linggen/skills");
        let rows: Vec<Memory> = (0..200)
            .map(|i| {
                indexed(
                    &format!("row {i}"),
                    "/Users/alex/w/linggen/skills",
                    Some(&"x".repeat(70)),
                )
            })
            .collect();
        let (shown, block, skipped) = render_index(rows, &p);
        assert!(skipped > 0);
        assert_eq!(shown.len() + skipped, 200);
        assert!(block.contains("more indexed row(s)"));
    }

    #[test]
    fn a_hookless_row_shows_its_opening() {
        let r = indexed(&"y".repeat(200), "/x", None);
        let l = index_line(&r);
        assert!(l.starts_with("- yyyy"));
        assert!(l.contains("…"));
    }

    #[test]
    fn an_empty_store_renders_nothing() {
        let s = SessionStart::build(vec![], vec![], None);
        assert_eq!(s.block, "");
    }
}
