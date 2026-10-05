//! The session thread, as one member reads it.
//!
//! A session's `messages.jsonl` is its one thread: every member's words,
//! tool use, recall and compaction. Each turn, the speaking member's
//! context is built from it (`doc/shared-session-spec.md` § The thread):
//!
//! - its own lines are its assistant turns; the user's are the user's; a
//!   line of another member's is labeled dialogue (`[Yinyue]: …`), never
//!   words in the reader's mouth;
//! - its own tool calls come back as native call/result pairs (or the text
//!   form for a model without native tools — `restored_tools`); another
//!   member's come back as labeled text (`[Ling used Look → …]`), never as
//!   pairs — a provider rejects or mishandles calls it did not make. Of
//!   that result the reader gets what the tool declares for others
//!   (`others_read`), else the output's head;
//! - recall and compaction rows are system notes, for every member;
//! - another member's hidden kickoff (`[HIDDEN] …`) is that member's alone.
//!
//! The thread starts at the newest compaction row: what came before it is
//! what that summary stands for.

use super::compact_rows::COMPACTION_SENDER;
use super::restored_tools::{self, ToolReplay, ToolRow};
use super::ChatRunCtx;
use crate::engine::{AgentEngine, ThreadMark};
use crate::message::ChatMessage;
use crate::state_fs::sessions::ChatMsg;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Another member's tool calls a reader sees, newest last; older ones drop.
const OTHERS_KEEP_CALLS: usize = 8;
/// How much of another member's tool result a reader sees.
const OTHERS_RESULT_CHARS: usize = 1200;
/// How much of the fields a tool declares for others (`others_read`) a
/// reader sees: chosen facts, so more of them than of a raw head.
const OTHERS_DECLARED_CHARS: usize = 2000;
/// How much of another member's call arguments a reader sees.
const OTHERS_ARG_CHARS: usize = 160;

/// Who reads the thread, and what it can tell about the rows.
pub(crate) struct Reader<'a> {
    /// The member whose turn this is.
    pub id: &'a str,
    /// Every sender in the rows that is an agent (the reader included).
    pub agents: &'a HashSet<String>,
    /// The reader's model calls tools natively.
    pub native: bool,
    /// The session skill's tools that declare what others read of their
    /// result (`others_read`), by name.
    pub others_read: &'a HashMap<String, Vec<String>>,
}

/// Bring the engine's thread up to the session's before a turn. The cached
/// thread stands while only its own member has written since it was last
/// marked; an empty one, or one another member (or a rewrite) has moved
/// past, is rebuilt from the file — without the message this turn answers,
/// which the turn adds itself. Returns whether it was rebuilt.
pub(super) async fn sync(engine: &mut AgentEngine, ctx: &ChatRunCtx) -> bool {
    let sid = ctx.session_id.as_deref().unwrap_or("default");
    let store = &ctx.manager.global_sessions;
    if !engine.chat_history.is_empty() {
        let skip = engine.thread_mark.map_or(usize::MAX, |m| m.rows);
        let (total, since) = store.rows_since(sid, skip).unwrap_or_default();
        let rewrites = store.rewrite_count(sid);
        if !is_stale(engine.thread_mark, rewrites, total, &since, &ctx.agent_id) {
            return false;
        }
    }
    rebuild(engine, ctx).await
}

/// Rebuild the engine's thread from the session's file, without the
/// message this turn answers. False when the file can't be read.
pub(super) async fn rebuild(engine: &mut AgentEngine, ctx: &ChatRunCtx) -> bool {
    let sid = ctx.session_id.as_deref().unwrap_or("default");
    let Ok(mut rows) = ctx.manager.global_sessions.get_chat_history(sid) else {
        return false;
    };
    if rows.last().is_some_and(|m| {
        !m.is_observation && m.from_id == ctx.from_id() && m.content == ctx.clean_msg
    }) {
        rows.pop();
    }
    let agents = speaking_agents(ctx, &rows).await;
    let others_read = declared_for_others(ctx, sid).await;
    let reader = Reader {
        id: &ctx.agent_id,
        agents: &agents,
        native: engine.model_manager.supports_tools(&engine.model_id),
        others_read: &others_read,
    };
    engine.chat_history = build(&rows, &reader);
    tracing::info!(
        "Rebuilt {}'s thread from the session: {} messages",
        ctx.agent_id,
        engine.chat_history.len()
    );
    true
}

/// Mark how far the engine's thread has read the session's file — after
/// its turn, everything on file is what it knows.
pub(super) fn mark(engine: &mut AgentEngine, ctx: &ChatRunCtx) {
    let sid = ctx.session_id.as_deref().unwrap_or("default");
    let store = &ctx.manager.global_sessions;
    let Ok((rows, _)) = store.rows_since(sid, usize::MAX) else {
        return;
    };
    engine.thread_mark = Some(ThreadMark {
        rows,
        rewrites: store.rewrite_count(sid),
    });
}

/// Whether a cached thread marked at `mark` no longer matches the file: it
/// was rewritten, cut, or another member wrote after the mark. A thread
/// never marked (seeded before its first turn) stands.
fn is_stale(
    mark: Option<ThreadMark>,
    rewrites: u64,
    total: usize,
    since: &[ChatMsg],
    own: &str,
) -> bool {
    let Some(mark) = mark else {
        return false;
    };
    mark.rewrites != rewrites
        || total < mark.rows
        || since
            .iter()
            .any(|r| r.agent_id != own || r.from_id == COMPACTION_SENDER)
}

/// The senders and recipients in `rows` that are agents, the reader with
/// them. Asked once per id: `agent_exists` reads the agent files.
async fn speaking_agents(ctx: &ChatRunCtx, rows: &[ChatMsg]) -> HashSet<String> {
    let mut agents = HashSet::from([ctx.agent_id.clone()]);
    let mut asked = HashSet::new();
    for id in rows
        .iter()
        .flat_map(|r| [&r.from_id, &r.agent_id, &r.to_id])
    {
        if matches!(id.as_str(), "user" | "system") || !asked.insert(id.as_str()) {
            continue;
        }
        if ctx.manager.agent_exists(&ctx.root, id).await {
            agents.insert(id.clone());
        }
    }
    agents
}

/// The session skill's tools that declare `others_read`: name → paths.
/// Empty for a session without a skill.
async fn declared_for_others(ctx: &ChatRunCtx, sid: &str) -> HashMap<String, Vec<String>> {
    let skill_name = ctx
        .manager
        .global_sessions
        .get_session_meta(sid)
        .ok()
        .flatten()
        .and_then(|m| m.skill);
    let Some(skill) = (match skill_name {
        Some(name) => ctx.manager.skills.get_skill(&name).await,
        None => None,
    }) else {
        return HashMap::new();
    };
    skill
        .tool_defs
        .into_iter()
        .filter(|t| !t.others_read.is_empty())
        .map(|t| (t.name, t.others_read))
        .collect()
}

/// The rows from the newest compaction summary on (all of them when none).
pub(crate) fn since_compaction(rows: &[ChatMsg]) -> &[ChatMsg] {
    let start = rows
        .iter()
        .rposition(|r| !r.is_observation && r.from_id == COMPACTION_SENDER)
        .unwrap_or(0);
    &rows[start..]
}

/// `rows` as `reader`'s thread.
pub(crate) fn build(rows: &[ChatMsg], reader: &Reader) -> Vec<ChatMessage> {
    let rows = since_compaction(rows);
    let own_first = restored_tools::first_kept_call(rows, reader.id, restored_tools::KEEP_CALLS);
    let others_first = first_kept_other_call(rows, reader);
    let mut out = Vec::new();
    let mut own = ToolReplay::new(reader.native);
    let mut others = OthersTools::default();
    for (i, m) in rows.iter().enumerate() {
        if m.is_observation {
            read_tool_row(
                m,
                i,
                reader,
                (own_first, others_first),
                &mut own,
                &mut others,
                &mut out,
            );
            continue;
        }
        own.flush(&mut out);
        others.flush(&mut out);
        if let Some(msg) = spoken(m, reader) {
            out.push(msg);
        }
    }
    own.flush(&mut out);
    others.flush(&mut out);
    out
}

/// One observation row, into the reader's replay or the others' notes.
fn read_tool_row(
    m: &ChatMsg,
    i: usize,
    reader: &Reader,
    (own_first, others_first): (usize, usize),
    own: &mut ToolReplay,
    others: &mut OthersTools,
    out: &mut Vec<ChatMessage>,
) {
    match restored_tools::tool_row(m, reader.id) {
        Some(ToolRow::Call { name, args }) if i >= own_first => {
            return own.call(name, args, out);
        }
        Some(ToolRow::Result { name, text }) => return own.result(&name, &text),
        Some(ToolRow::Call { .. }) => return,
        None => {}
    }
    match other_tool_row(m, reader) {
        Some((agent, ToolRow::Call { name, args })) if i >= others_first => {
            others.call(agent, name, args)
        }
        Some((agent, ToolRow::Result { name, text })) => {
            let seen = for_others(&text, reader.others_read.get(&name));
            others.result(&agent, &name, seen)
        }
        _ => {}
    }
}

/// A spoken row as the reader reads it; `None` for a row that isn't part
/// of its thread.
fn spoken(m: &ChatMsg, reader: &Reader) -> Option<ChatMessage> {
    if m.from_id == "system" {
        return None;
    }
    // Another member's hidden kickoff is that member's turn alone.
    if m.content.contains("[HIDDEN]") && m.agent_id != reader.id {
        return None;
    }
    if m.from_id == reader.id {
        return Some(ChatMessage::new("assistant", m.content.clone()));
    }
    if m.from_id == "user" {
        return Some(ChatMessage::new("user", user_line(m, reader)));
    }
    if reader.agents.contains(&m.from_id) {
        return Some(ChatMessage::new(
            "user",
            super::with_sender_label(&m.from_id, &m.content),
        ));
    }
    // Recall, a compaction summary, any other pseudo-sender: context the
    // session was handed — a note for every member, never anyone's words.
    Some(ChatMessage::new("system", m.content.clone()))
}

/// The user's line: plain when it was to the reader, labeled with whom it
/// was for when it was to another member (`@银月 …` addressed her).
fn user_line(m: &ChatMsg, reader: &Reader) -> String {
    let to = m.agent_id.as_str();
    if to == reader.id || !reader.agents.contains(to) {
        return m.content.clone();
    }
    format!("[User → {}]: {}", super::sender_label(to), m.content)
}

/// An observation row of another member's: its call, or a result handed to
/// it. `None` for the reader's own rows and for anything else.
fn other_tool_row(m: &ChatMsg, reader: &Reader) -> Option<(String, ToolRow)> {
    let agent = if m.from_id == "system" {
        &m.to_id
    } else {
        &m.from_id
    };
    if agent == reader.id || !reader.agents.contains(agent) {
        return None;
    }
    restored_tools::tool_row(m, agent).map(|row| (agent.clone(), row))
}

/// Index of the first of another member's calls the reader still sees.
fn first_kept_other_call(rows: &[ChatMsg], reader: &Reader) -> usize {
    let calls: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(other_tool_row(m, reader), Some((_, ToolRow::Call { .. }))))
        .map(|(i, _)| i)
        .collect();
    calls
        .len()
        .checked_sub(OTHERS_KEEP_CALLS)
        .map_or(0, |skip| calls[skip])
}

/// Another member's calls in flight, with their results as they come in;
/// written as one labeled note when the next spoken row (or the end) comes.
#[derive(Default)]
struct OthersTools {
    calls: Vec<OtherCall>,
}

struct OtherCall {
    agent: String,
    name: String,
    args: Value,
    result: Option<String>,
}

impl OthersTools {
    fn call(&mut self, agent: String, name: String, args: Value) {
        self.calls.push(OtherCall {
            agent,
            name,
            args,
            result: None,
        });
    }

    /// A result answers the first call of that member and name still waiting.
    fn result(&mut self, agent: &str, name: &str, seen: String) {
        if let Some(c) = self
            .calls
            .iter_mut()
            .find(|c| c.agent == agent && c.name == name && c.result.is_none())
        {
            c.result = Some(seen);
        }
    }

    fn flush(&mut self, out: &mut Vec<ChatMessage>) {
        if self.calls.is_empty() {
            return;
        }
        let lines: Vec<String> = std::mem::take(&mut self.calls)
            .into_iter()
            .map(|c| c.line())
            .collect();
        out.push(ChatMessage::new("user", lines.join("\n")));
    }
}

impl OtherCall {
    /// `[Ling used Look {"at":"well"} → …]`
    fn line(&self) -> String {
        let args = match &self.args {
            Value::Object(m) if !m.is_empty() => {
                format!(" {}", cut(&self.args.to_string(), OTHERS_ARG_CHARS))
            }
            _ => String::new(),
        };
        let result = self.result.as_deref().unwrap_or("(result not kept)");
        format!(
            "[{} used {}{args} → {result}]",
            super::sender_label(&self.agent),
            self.name
        )
    }
}

/// What another member reads of a tool result: the fields the tool
/// declares for others when its output is JSON that has any of them, else
/// the head of its output. A command's frame (`Bash output …/STDOUT:`) is
/// dropped when it succeeded — the reader wants what it said.
fn for_others(text: &str, declared: Option<&Vec<String>>) -> String {
    let body = command_stdout(text).unwrap_or(text);
    match declared.and_then(|paths| declared_fields(body, paths)) {
        Some(fields) => cut(&fields, OTHERS_DECLARED_CHARS),
        None => cut(body, OTHERS_RESULT_CHARS),
    }
}

/// A successful command's stdout, out of its rendered frame.
fn command_stdout(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("Bash output (exit_code: Some(0)):\nSTDOUT:\n")?;
    let (stdout, _) = rest.rsplit_once("\nSTDERR:\n")?;
    Some(stdout.trim_end())
}

/// `{"scene.place": …, …}` — the declared paths `body` has, in declared
/// order; `None` when it isn't a JSON object or has none of them.
fn declared_fields(body: &str, paths: &[String]) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    if !v.is_object() {
        return None;
    }
    let fields: Map<String, Value> = paths
        .iter()
        .filter_map(|p| {
            let keys: Vec<&str> = p.split('.').collect();
            pick(&v, &keys).map(|found| (p.clone(), found))
        })
        .collect();
    (!fields.is_empty()).then(|| Value::Object(fields).to_string())
}

/// The value at `keys` under `v`; through a list, each item's. Null and
/// empty values count as absent.
fn pick(v: &Value, keys: &[&str]) -> Option<Value> {
    let Some((key, rest)) = keys.split_first() else {
        return is_present(v).then(|| v.clone());
    };
    match v {
        Value::Object(m) => pick(m.get(*key)?, rest),
        Value::Array(items) => {
            let found: Vec<Value> = items.iter().filter_map(|i| pick(i, keys)).collect();
            (!found.is_empty()).then_some(Value::Array(found))
        }
        _ => None,
    }
}

fn is_present(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
        _ => true,
    }
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(agent: &str, from: &str, to: &str, content: &str, obs: bool) -> ChatMsg {
        ChatMsg {
            agent_id: agent.into(),
            from_id: from.into(),
            to_id: to.into(),
            content: content.into(),
            timestamp: 0,
            is_observation: obs,
        }
    }

    fn call(agent: &str, name: &str) -> ChatMsg {
        let json = format!(r#"{{"type":"tool","tool":"{name}","args":{{"said":"去临淄"}}}}"#);
        row(agent, agent, "user", &json, true)
    }

    fn result(agent: &str, name: &str, text: &str) -> ChatMsg {
        row(
            agent,
            "system",
            agent,
            &format!("Tool {name}: {text}"),
            true,
        )
    }

    fn agents() -> HashSet<String> {
        ["ling", "yinyue"].into_iter().map(String::from).collect()
    }

    /// A Lingjing table: the user's line to Ling, his Look, his narration,
    /// the user's line to her, her answer.
    fn table() -> Vec<ChatMsg> {
        vec![
            row("ling", "user", "ling", "去临淄", false),
            row(
                "ling",
                "memory-recall",
                "ling",
                "From memory (fact…): likes 临淄",
                false,
            ),
            row(
                "ling",
                "system",
                "ling",
                "Starting autonomous loop for task: 去临淄",
                true,
            ),
            call("ling", "Look"),
            result("ling", "Look", r#"{"scene":"临淄城门"}"#),
            row("ling", "ling", "user", "临淄到了。", false),
            row("yinyue", "user", "yinyue", "这是哪里?", false),
            row("yinyue", "yinyue", "user", "临淄，齐国的都城。", false),
        ]
    }

    fn shape(thread: &[ChatMessage]) -> Vec<(String, String)> {
        thread
            .iter()
            .map(|m| {
                let calls: Vec<&str> = m
                    .tool_calls
                    .iter()
                    .map(|c| c.function.name.as_str())
                    .collect();
                let text = if calls.is_empty() {
                    m.content.clone()
                } else {
                    format!("calls:{}", calls.join(","))
                };
                (m.role.clone(), text)
            })
            .collect()
    }

    /// Her thread: Ling's Look reaches her as a labeled note, never a pair;
    /// his words are labeled his; recall is a system note; her own words
    /// are hers.
    #[test]
    fn she_reads_his_tool_use_as_labeled_text_and_her_own_words_as_hers() {
        let agents = agents();
        let reader = Reader {
            id: "yinyue",
            agents: &agents,
            native: true,
            others_read: &HashMap::new(),
        };
        let got = shape(&build(&table(), &reader));
        let want: Vec<(String, String)> = [
            ("user", "[User → Ling]: 去临淄"),
            ("system", "From memory (fact…): likes 临淄"),
            (
                "user",
                r#"[Ling used Look {"said":"去临淄"} → {"scene":"临淄城门"}]"#,
            ),
            ("user", "[Ling]: 临淄到了。"),
            ("user", "这是哪里?"),
            ("assistant", "临淄，齐国的都城。"),
        ]
        .into_iter()
        .map(|(r, c)| (r.to_string(), c.to_string()))
        .collect();
        assert_eq!(got, want);
        assert!(build(&table(), &reader)
            .iter()
            .all(|m| m.tool_calls.is_empty() && m.role != "tool"));
    }

    /// His thread: his own Look comes back as a native pair; her line is
    /// labeled hers, and the user's line to her says so.
    #[test]
    fn he_reads_his_own_tool_use_as_pairs_and_her_words_as_hers() {
        let agents = agents();
        let reader = Reader {
            id: "ling",
            agents: &agents,
            native: true,
            others_read: &HashMap::new(),
        };
        let got = shape(&build(&table(), &reader));
        let want: Vec<(String, String)> = [
            ("user", "去临淄"),
            ("system", "From memory (fact…): likes 临淄"),
            ("assistant", "calls:Look"),
            ("tool", r#"{"scene":"临淄城门"}"#),
            ("assistant", "临淄到了。"),
            ("user", "[User → Yinyue]: 这是哪里?"),
            ("user", "[Yinyue]: 临淄，齐国的都城。"),
        ]
        .into_iter()
        .map(|(r, c)| (r.to_string(), c.to_string()))
        .collect();
        assert_eq!(got, want);
    }

    /// A compaction summary and recall are system notes for every member —
    /// never an assistant turn — and the thread starts at the newest
    /// summary.
    #[test]
    fn compaction_and_recall_are_system_notes_and_the_thread_starts_at_the_summary() {
        let rows = vec![
            row("ling", "user", "ling", "很久以前", false),
            row("ling", "ling", "user", "旧事", false),
            row("ling", COMPACTION_SENDER, "ling", "- went to 临淄", false),
            row("yinyue", "memory-recall", "yinyue", "From memory: x", false),
            row("ling", "user", "ling", "去碣石", false),
        ];
        let agents = agents();
        for id in ["ling", "yinyue"] {
            let reader = Reader {
                id,
                agents: &agents,
                native: true,
                others_read: &HashMap::new(),
            };
            let thread = build(&rows, &reader);
            assert_eq!(thread[0].role, "system", "{id}");
            assert_eq!(thread[0].content, "- went to 临淄");
            assert_eq!(thread[1].role, "system", "{id}");
            assert!(!thread.iter().any(|m| m.content == "旧事"));
        }
    }

    /// Another member's hidden kickoff (an app moment for her) is not his to
    /// read; his own hidden kickoff is.
    #[test]
    fn another_members_hidden_kickoff_is_hers_alone() {
        let rows = vec![
            row(
                "yinyue",
                "user",
                "yinyue",
                "[HIDDEN] a moment for her",
                false,
            ),
            row("yinyue", "yinyue", "user", "别怕。", false),
            row("ling", "user", "ling", "[HIDDEN] answer her once", false),
        ];
        let agents = agents();
        let ling = Reader {
            id: "ling",
            agents: &agents,
            native: true,
            others_read: &HashMap::new(),
        };
        let got: Vec<String> = build(&rows, &ling).into_iter().map(|m| m.content).collect();
        assert_eq!(got, ["[Yinyue]: 别怕。", "[HIDDEN] answer her once"]);
    }

    /// The cached thread stands while only its own member writes; another
    /// member's row, a summary or a rewrite after the mark makes it stale.
    #[test]
    fn a_cached_thread_is_stale_once_another_member_or_a_rewrite_moved_past_it() {
        let mark = Some(ThreadMark {
            rows: 4,
            rewrites: 1,
        });
        let own = vec![row("ling", "user", "ling", "go", false)];
        assert!(!is_stale(mark, 1, 5, &own, "ling"));
        assert!(!is_stale(None, 9, 0, &[], "ling"), "never marked: stands");
        let hers = vec![row("yinyue", "yinyue", "user", "别怕。", false)];
        assert!(is_stale(mark, 1, 5, &hers, "ling"));
        let to_her = vec![row("yinyue", "user", "yinyue", "@银月 hi", false)];
        assert!(is_stale(mark, 1, 5, &to_her, "ling"));
        let summary = vec![row("ling", COMPACTION_SENDER, "ling", "S", false)];
        assert!(is_stale(mark, 1, 5, &summary, "ling"));
        assert!(is_stale(mark, 2, 5, &own, "ling"), "rewritten");
        assert!(is_stale(mark, 1, 3, &[], "ling"), "cut");
    }

    /// Only another member's newest calls come through, each result cut.
    #[test]
    fn another_members_old_calls_drop_and_long_results_are_cut() {
        let mut rows = vec![row("ling", "user", "ling", "go", false)];
        for i in 0..12 {
            rows.push(call("ling", "Look"));
            rows.push(result("ling", "Look", &format!("r{i}{}", "x".repeat(2000))));
        }
        rows.push(row("ling", "ling", "user", "done", false));
        let agents = agents();
        let reader = Reader {
            id: "yinyue",
            agents: &agents,
            native: false,
            others_read: &HashMap::new(),
        };
        let thread = build(&rows, &reader);
        let note = &thread[1].content;
        assert_eq!(note.lines().count(), OTHERS_KEEP_CALLS);
        assert!(note.lines().next().unwrap().contains("→ r4"));
        assert!(note
            .lines()
            .all(|l| l.chars().count() < OTHERS_RESULT_CHARS + 120));
    }

    /// A command result as the engine renders it.
    fn framed(stdout: &str) -> String {
        format!("Bash output (exit_code: Some(0)):\nSTDOUT:\n{stdout}\n\nSTDERR:\n")
    }

    /// A long Look: instructions to its caller first, the facts deep inside.
    fn long_look() -> String {
        serde_json::json!({
            "then": "Tell the scene first. ".repeat(80),
            "ask": null,
            "chapter": {"id": "h04", "title": "第三回"},
            "scene": {
                "place": "沉鼎观 · 山门 · 石壁",
                "setup": "一面石壁，刻着九个空格。",
                "people": [{"id": "qulao", "name": "瞿老"}, {"id": "a", "name": "阿禾"}],
                "lines": []
            },
            "guide": {"look": "x".repeat(9000)}
        })
        .to_string()
    }

    fn her_read_of(rows: &[ChatMsg], declared: &HashMap<String, Vec<String>>) -> String {
        let agents = agents();
        let reader = Reader {
            id: "yinyue",
            agents: &agents,
            native: true,
            others_read: declared,
        };
        build(rows, &reader)[1].content.clone()
    }

    fn look_rows(text: &str) -> Vec<ChatMsg> {
        vec![
            row("ling", "user", "ling", "go", false),
            call("ling", "Look"),
            result("ling", "Look", text),
            row("ling", "ling", "user", "到了山门。", false),
        ]
    }

    /// What a tool declares for others is what she reads of it — the
    /// place and scene, not its caller's instructions — in declared order,
    /// through lists, with absent and empty fields left out.
    #[test]
    fn she_reads_the_fields_a_tool_declares_for_others() {
        let paths = [
            "scene.place",
            "scene.setup",
            "scene.people.name",
            "scene.lines",
            "page_did",
        ];
        let declared = HashMap::from([(
            "Look".to_string(),
            paths.iter().map(|p| p.to_string()).collect(),
        )]);
        let note = her_read_of(&look_rows(&framed(&long_look())), &declared);
        assert_eq!(
            note,
            r#"[Ling used Look {"said":"去临淄"} → {"scene.place":"沉鼎观 · 山门 · 石壁","scene.setup":"一面石壁，刻着九个空格。","scene.people.name":["瞿老","阿禾"]}]"#
        );
    }

    /// Without a declaration she reads the head of what the command said,
    /// out of its frame; so too when the output has none of the fields (a
    /// refusal) or isn't JSON.
    #[test]
    fn undeclared_or_unmatched_results_read_as_the_outputs_head() {
        let none = HashMap::new();
        let note = her_read_of(&look_rows(&framed(&long_look())), &none);
        assert!(
            note.contains(r#"→ {"then":"Tell the scene first."#),
            "{note}"
        );
        assert!(!note.contains("Bash output"));
        assert!(!note.contains("沉鼎观"), "the head only");

        let declared = HashMap::from([("Look".to_string(), vec!["scene.place".to_string()])]);
        let refusal = framed(r#"{"ok":false,"why":"no-such-exit"}"#);
        let note = her_read_of(&look_rows(&refusal), &declared);
        assert!(
            note.ends_with(r#"→ {"ok":false,"why":"no-such-exit"}]"#),
            "{note}"
        );

        let failed = "Bash output (exit_code: Some(1)):\nSTDOUT:\n\nSTDERR:\nboom\n";
        let note = her_read_of(&look_rows(failed), &declared);
        assert!(
            note.contains("exit_code: Some(1)") && note.contains("boom"),
            "{note}"
        );
    }
}
