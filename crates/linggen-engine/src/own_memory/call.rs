//! The tool's ops — `index`, `read`, `search`, `titles`, `write` — read from
//! arguments and answered as data. The Linggen sessions' built-in tool and
//! the MCP door's `memory_notes` both call [`call`], so what an op needs and
//! what it answers is written once.

use super::fence::Audience;
use super::find::{self, Found, Read};
use super::format::{Entry, Kind, Scope, Voice};
use super::index::index;
use super::spec::Op;
use super::write::{write, Action, Write};
use super::Store;
use crate::outside_memory as outside;
use serde_json::{json, Value};
use std::path::PathBuf;

const DEFAULT_HITS: usize = 10;
const MOST_HITS: usize = 30;
const DEFAULT_TITLES: usize = 5;

/// Who is calling and from where.
#[derive(Clone, Debug)]
pub struct Caller {
    pub audience: Audience,
    /// Recorded as the writer of a note: `claude-code`, `codex`, `linggen`, `yinyue`, …
    pub source: String,
    /// The session's folder (index order), when it has one.
    pub cwd: Option<PathBuf>,
}

/// One op's answer, or the words a caller is told.
pub fn call(store: &Store, caller: &Caller, op: &str, args: &Value, now: i64) -> Result<Value, String> {
    let op = Op::parse(op).ok_or_else(|| format!("unknown op `{op}`: index, read, search or write"))?;
    match op {
        Op::Index => {
            let cwd = text(args, "cwd").map(PathBuf::from).or_else(|| caller.cwd.clone());
            let got = index(store, caller.audience, cwd.as_deref(), now);
            Ok(json!({ "index": got.text, "shown": got.shown, "left_out": got.left_out }))
        }
        Op::Read => {
            let what = text(args, "id").or_else(|| text(args, "path")).ok_or("name the note's `id` (or a topic file's `path`)")?;
            match find::read(store, caller.audience, what, now).map_err(|e| e.to_string())? {
                Read::Entry(found) => Ok(found_json(&found, true)),
                Read::File { rel, version, scope, topic, entries } => Ok(json!({
                    "path": rel, "version": version, "scope": scope, "topic": topic,
                    "notes": entries.iter().map(|e| entry_json(e, true)).collect::<Vec<_>>(),
                })),
            }
        }
        Op::Search => {
            let words = text(args, "words").ok_or("name the `words` to find")?;
            let limit = args.get("limit").and_then(Value::as_u64).map_or(DEFAULT_HITS, |n| (n as usize).clamp(1, MOST_HITS));
            let own: Vec<Value> = find::search(store, caller.audience, words, limit, now)
                .iter()
                .map(|f| found_json(f, false))
                .collect();
            let others: Vec<Value> = outside::search(store.env(), words, limit)
                .iter()
                .map(|h| json!({ "source": h.source, "path": store.env().tilde(&h.path), "line": h.line, "text": h.text.as_str() }))
                .collect();
            Ok(json!({ "hits": own, "other_agents": others }))
        }
        Op::Titles => {
            let words = text(args, "words").ok_or("name the `words` to find")?;
            let limit = args.get("limit").and_then(Value::as_u64).map_or(DEFAULT_TITLES, |n| (n as usize).clamp(1, MOST_HITS));
            let exclude: Vec<String> = args
                .get("exclude")
                .and_then(Value::as_array)
                .map(|ids| ids.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default();
            let titles: Vec<Value> = find::titles(store, caller.audience, words, &exclude, limit)
                .iter()
                .map(|f| json!({ "id": f.entry.id, "summary": f.entry.summary, "kind": f.entry.kind.word(), "topic": f.topic }))
                .collect();
            Ok(json!({ "titles": titles }))
        }
        Op::Write => {
            let w = write_of(args, caller)?;
            let done = write(store, caller.audience, now, &w).map_err(|r| r.to_string())?;
            Ok(json!({ "id": done.id, "path": done.rel, "version": done.version, "merged": done.merged }))
        }
    }
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

fn flag(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn need<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    text(args, key).ok_or_else(|| format!("write needs `{key}`"))
}

fn write_of(args: &Value, caller: &Caller) -> Result<Write, String> {
    let action = match text(args, "action").unwrap_or(if args.get("id").is_some() { "update" } else { "add" }) {
        "add" => Action::Add {
            scope: Scope::parse(need(args, "scope")?).ok_or("`scope` is person, area:<body|mind|people|music|money|play|home>, project:<absolute path>, moments or chronicle")?,
            topic: text(args, "topic").unwrap_or("notes").to_string(),
            summary: need(args, "summary")?.to_string(),
            kind: Kind::parse(need(args, "kind")?).ok_or("`kind` is person, rule, decision, gotcha, moment or chronicle")?,
            voice: text(args, "voice").map_or(Some(Voice::Agent), Voice::parse).ok_or("`voice` is user or agent")?,
            body: text(args, "body").unwrap_or_default().to_string(),
        },
        "update" => Action::Update {
            id: need(args, "id")?.to_string(),
            summary: text(args, "summary").map(String::from),
            body: args.get("body").and_then(Value::as_str).map(String::from),
            kind: match text(args, "kind") {
                Some(k) => Some(Kind::parse(k).ok_or("`kind` is person, rule, decision, gotcha, moment or chronicle")?),
                None => None,
            },
        },
        "void" => Action::Void { id: need(args, "id")?.to_string(), reason: need(args, "reason")?.to_string() },
        "no_raise" => Action::NoRaise { id: need(args, "id")?.to_string(), on: flag(args, "on").unwrap_or(true) },
        "yearly" => Action::Yearly { id: need(args, "id")?.to_string(), day: text(args, "day").map(String::from) },
        other => return Err(format!("unknown action `{other}`: add, update, void, no_raise or yearly")),
    };
    Ok(Write {
        action,
        user_directed: flag(args, "user_directed").unwrap_or(false),
        source: caller.source.clone(),
        base: text(args, "base").map(String::from),
    })
}

fn entry_json(e: &Entry, with_body: bool) -> Value {
    let mut v = json!({
        "id": e.id, "summary": e.summary, "kind": e.kind.word(), "voice": e.voice.word(),
        "created": e.created, "updated": e.updated, "source": e.source,
        "void": e.void, "no_raise": e.no_raise, "yearly": e.yearly,
    });
    if with_body {
        v["body"] = json!(e.body);
    }
    v
}

fn found_json(f: &Found, with_body: bool) -> Value {
    let mut v = entry_json(&f.entry, with_body);
    v["topic"] = json!(f.topic);
    v["scope"] = json!(f.scope);
    v["path"] = json!(f.rel);
    v["version"] = json!(f.version);
    v
}
