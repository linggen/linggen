//! SDK children use the existing SubagentPane contract; no native agent loop.
use super::{runtime::ClaudeSession, AGENT_ID};
use crate::engine::agent::AgentEvent;
use crate::server::{ServerEvent, ServerState};
use crate::state_fs::sessions::SessionStore;
use anyhow::Result;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

const FILE: &str = "claude-subagents.json";
pub(super) fn tracking_id(run: &str, agent: &str) -> String {
    format!("sdk-sub-{run}-{agent}")
}

#[derive(Default)]
pub(super) struct Subagents {
    entries: BTreeMap<String, Value>,
    /// The run each child was spawned in. One runner serves many runs and a
    /// background child outlives the run that spawned it; it is saved under
    /// that run alone, so history shows it once, on the turn that asked.
    origins: BTreeMap<String, String>,
}

impl Subagents {
    /// A child's tracking id, under the run that spawned it; a child not seen
    /// yet belongs to `current`. A background child's events keep arriving
    /// after later runs began, and must not move it to them.
    pub fn tracking(&self, raw: &str, current: &str) -> String {
        tracking_id(self.origins.get(raw).map_or(current, String::as_str), raw)
    }

    pub async fn handle(&mut self, host: &ClaudeSession, event: &Value) -> Result<bool> {
        let kind = event["type"].as_str().unwrap_or_default();
        let Some(raw) = event["agent_id"].as_str().filter(|id| !id.is_empty()) else {
            return Ok(false);
        };
        if !kind.starts_with("subagent_") && !matches!(kind, "tool_start" | "tool_end") {
            return Ok(false);
        }
        let id = self.tracking(raw, &host.run_id);
        if !self.entries.contains_key(raw) {
            let name = event["name"].as_str().unwrap_or("Claude subagent");
            let task = event["task"].as_str().unwrap_or_default();
            self.origins.insert(raw.into(), host.run_id.clone());
            self.entries.insert(
                raw.into(),
                json!({"subagentId":id, "sdkTaskId":raw, "agentName":name, "task":task,
                "status":"running", "toolCount":0, "contextTokens":0, "currentActivity":null,
                "toolSteps":[], "resultText":"", "timestampMs":crate::util::now_ts_ms()}),
            );
            host.emit(AgentEvent::SubagentSpawned {
                parent_id: AGENT_ID.into(),
                subagent_id: name.into(),
                task: task.into(),
                subagent_run_id: Some(id.clone()),
                parent_run_id: Some(host.run_id.clone()),
            })
            .await;
        }
        let origin = self.origins.get(raw).cloned().unwrap_or_else(|| host.run_id.clone());
        let entry = self.entries.get_mut(raw).unwrap();
        match kind {
            "subagent_start" => {
                let before = (entry["agentName"].clone(), entry["task"].clone());
                for (source, target) in [("name", "agentName"), ("task", "task")] {
                    if event[source].as_str().is_some_and(|s| !s.is_empty()) {
                        entry[target] = event[source].clone();
                    }
                }
                if before != (entry["agentName"].clone(), entry["task"].clone()) {
                    host.emit(AgentEvent::SubagentSpawned {
                        parent_id: AGENT_ID.into(),
                        subagent_id: entry["agentName"].as_str().unwrap_or_default().into(),
                        task: entry["task"].as_str().unwrap_or_default().into(),
                        subagent_run_id: Some(id.clone()),
                        parent_run_id: Some(origin.clone()),
                    })
                    .await;
                }
            }
            "subagent_text" => {
                if let Some(text) = event["text"].as_str().filter(|s| !s.is_empty()) {
                    let previous = if entry["hasText"] == true {
                        entry["resultText"].as_str().unwrap_or_default()
                    } else {
                        ""
                    };
                    entry["resultText"] = json!(if previous.is_empty() {
                        text.into()
                    } else {
                        format!("{previous}\n\n{text}")
                    });
                    entry["hasText"] = json!(true);
                    child_message(host, &id, entry["resultText"].as_str().unwrap()).await;
                }
            }
            "subagent_progress" => {
                entry["currentActivity"] = event["activity"].clone();
            }
            "subagent_context" => {
                let tokens = set_context(entry, event);
                host.emit(context_event(&id, tokens)).await;
            }
            "tool_start" => {
                let tool = event["tool"].as_str().unwrap_or("Tool");
                let args =
                    crate::engine::render::sanitize_tool_args_for_display(tool, &event["args"])
                        .to_string();
                entry["toolSteps"].as_array_mut().unwrap().push(json!({"id":event["id"], "toolName":tool, "args":args, "status":"running", "timestampMs":crate::util::now_ts_ms()}));
                entry["toolCount"] = json!(entry["toolSteps"].as_array().unwrap().len());
                host.emit(AgentEvent::ContentBlockStart {
                    agent_id: id.clone(),
                    block_id: event["id"].as_str().unwrap_or_default().into(),
                    block_type: "tool_use".into(),
                    tool: Some(tool.into()),
                    args: Some(args),
                    parent_id: Some(AGENT_ID.into()),
                    run_id: Some(id.clone()),
                    parent_run_id: Some(origin.clone()),
                })
                .await;
            }
            "tool_end" => {
                if let Some(step) = entry["toolSteps"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|step| step["id"] == event["id"])
                {
                    step["status"] = json!(if event["failed"] == true {
                        "failed"
                    } else {
                        "done"
                    });
                }
                host.emit(AgentEvent::ContentBlockUpdate {
                    agent_id: id.clone(),
                    block_id: event["id"].as_str().unwrap_or_default().into(),
                    status: Some(
                        if event["failed"] == true {
                            "failed"
                        } else {
                            "done"
                        }
                        .into(),
                    ),
                    summary: Some(event["result"].to_string()),
                    is_error: Some(event["failed"] == true),
                    parent_id: Some(AGENT_ID.into()),
                    extra: None,
                    run_id: Some(id.clone()),
                    parent_run_id: Some(origin.clone()),
                })
                .await;
            }
            "subagent_report" => {
                let Some(text) = event["text"].as_str().map(str::trim).filter(|s| !s.is_empty())
                else {
                    return Ok(true);
                };
                entry["report"] = json!(text);
                // A report after the end still reaches the open pane.
                if entry["status"] != "running" {
                    host.emit(result_event(entry, &id, &origin)).await;
                }
            }
            "subagent_end" => {
                let failed = matches!(event["status"].as_str(), Some("failed" | "stopped"));
                let first_end = end(entry, event, failed);
                if first_end {
                    let text = entry["resultText"].as_str().unwrap_or_default();
                    child_message(
                        host,
                        &id,
                        &if failed {
                            format!("Error: {text}")
                        } else {
                            text.into()
                        },
                    )
                    .await;
                    host.emit(result_event(entry, &id, &origin)).await;
                }
            }
            _ => {}
        }
        let running = entry["status"] == "running";
        host.state.claude.child_running(&host.meta.id, &id, running);
        save(host, &self.entries, &self.origins)?;
        Ok(true)
    }

    pub async fn finish(&mut self, host: &ClaudeSession) -> Result<()> {
        let pending: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry["status"] == "running")
            .map(|(id, _)| id.clone())
            .collect();
        for id in pending {
            self.handle(host, &json!({"type":"subagent_end", "agent_id":id, "status":"stopped", "text":"Task stopped"})).await?;
        }
        Ok(())
    }
}

/// The SDK's stand-in for a report it delivered as a message: no result.
const HANDED_BACK: &str = "report was delivered to you as a message";

/// Mark a child ended; true the first time it ends (or first fails). Its
/// end text is its result only when nothing better came: no text of its
/// own, and never the SDK's stand-in for a handed-back report.
fn end(entry: &mut Value, event: &Value, failed: bool) -> bool {
    let first_end = entry["status"] == "running" || failed && entry["status"] != "failed";
    entry["status"] = json!(if failed { "failed" } else { "done" });
    entry["currentActivity"] = Value::Null;
    let text = event["text"].as_str().unwrap_or_default();
    if entry["resultText"].as_str().unwrap_or_default().is_empty() && !text.contains(HANDED_BACK) {
        entry["resultText"] = json!(text);
    }
    for step in entry["toolSteps"].as_array_mut().unwrap() {
        if step["status"] == "running" {
            step["status"] = json!("failed");
        }
    }
    first_end
}

fn result_event(entry: &Value, id: &str, origin: &str) -> AgentEvent {
    AgentEvent::SubagentResult {
        parent_id: AGENT_ID.into(),
        subagent_id: entry["agentName"].as_str().unwrap_or_default().into(),
        outcome: crate::engine::AgentOutcome::None,
        report: entry["report"].as_str().map(str::to_owned),
        subagent_run_id: Some(id.into()),
        parent_run_id: Some(origin.into()),
    }
}

/// The window the child's latest request filled, kept with the child.
fn set_context(entry: &mut Value, event: &Value) -> usize {
    let tokens = event["tokens"].as_u64().unwrap_or(0) as usize;
    entry["contextTokens"] = json!(tokens);
    tokens
}

/// A child's window, for its row and detail: its own agent and run, so it
/// never reads as the main thread's.
fn context_event(id: &str, tokens: usize) -> AgentEvent {
    AgentEvent::ContextUsage {
        agent_id: id.into(),
        stage: "sdk".into(),
        message_count: 0,
        char_count: 0,
        estimated_tokens: tokens,
        token_limit: None,
        actual_prompt_tokens: None,
        actual_completion_tokens: None,
        compressed: false,
        summary_count: 0,
        run_id: Some(id.into()),
    }
}

async fn child_message(host: &ClaudeSession, id: &str, text: &str) {
    let _ = host.state.events_tx.send(ServerEvent::Message {
        from: id.into(),
        to: AGENT_ID.into(),
        content: text.into(),
        session_id: Some(host.meta.id.clone()),
        run_id: Some(id.into()),
        parent_agent_id: Some(AGENT_ID.into()),
        client_id: None,
        via: None,
        via_note: None,
        images: vec![],
    });
}

fn read(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(json!({}))
}
/// Each child under the run that spawned it, never under every later run
/// the runner serves: those copies froze at "running" and came back as a
/// second, failed row for the same child.
fn store_by_origin(
    all: &mut Value,
    entries: &BTreeMap<String, Value>,
    origins: &BTreeMap<String, String>,
    current: &str,
) {
    let mut by_run: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for (raw, entry) in entries {
        let run = origins.get(raw).map_or(current, String::as_str);
        by_run.entry(run).or_default().push(entry);
    }
    for (run, list) in by_run {
        all[run] = json!(list);
    }
}

fn save(
    host: &ClaudeSession,
    entries: &BTreeMap<String, Value>,
    origins: &BTreeMap<String, String>,
) -> Result<()> {
    let path = host
        .state
        .manager
        .global_sessions
        .session_dir(&host.meta.id)
        .join(FILE);
    let mut all = read(&path);
    store_by_origin(&mut all, entries, origins, &host.run_id);
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, serde_json::to_vec(&all)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

/// Attach the tree only to the last visible parent row of each SDK run.
/// A child still marked running is running only when it is one of `live`:
/// the tracking ids the session's live runner runs. Any other ended unseen
/// (an engine restart took its runner), even while the session has a turn,
/// and even when a later run resumed the same SDK task under a new id.
pub(crate) fn attach(
    store: &SessionStore,
    sid: &str,
    live: &HashSet<String>,
    messages: &mut [(Value, String)],
) {
    if sid.is_empty() || sid.contains('/') || sid.contains("..") {
        return;
    }
    let all = read(&store.session_dir(sid).join(FILE));
    let mut last = BTreeMap::new();
    for (i, (meta, _)) in messages.iter().enumerate() {
        if meta["from"] == AGENT_ID {
            if let Some(run) = meta["run_id"].as_str() {
                last.insert(run.to_owned(), i);
            }
        }
    }
    // Runs in the order they were said. A file written before children were
    // saved by origin run holds a copy of each child under every later run of
    // the same runner: the child goes once on the first run that has it, as
    // the last run that has it last saw it.
    let mut runs: Vec<(String, usize)> = last.into_iter().collect();
    runs.sort_by_key(|(_, i)| *i);
    let mut latest: BTreeMap<String, Value> = BTreeMap::new();
    for (run, _) in &runs {
        for entry in all[run].as_array().into_iter().flatten() {
            if let Some(id) = entry["subagentId"].as_str() {
                latest.insert(id.to_owned(), entry.clone());
            }
        }
    }
    let mut placed = HashSet::new();
    for (run, i) in runs {
        let Some(saved) = all[&run].as_array() else {
            continue;
        };
        let mut entries = Vec::new();
        for entry in saved {
            let id = entry["subagentId"].as_str();
            if id.is_some_and(|id| !placed.insert(id.to_owned())) {
                continue;
            }
            let mut entry = id.and_then(|id| latest.get(id)).unwrap_or(entry).clone();
            if entry["status"] == "running" && !id.is_some_and(|id| live.contains(id)) {
                entry["status"] = json!("failed");
            }
            entries.push(entry);
        }
        if !entries.is_empty() {
            messages[i].0["subagents"] = json!(entries);
        }
    }
}

/// The existing pane's stop button targets the SDK task, never a native run.
pub(crate) async fn cancel(state: &ServerState, run: &str) -> Option<Result<()>> {
    if !run.starts_with("sdk-sub-") {
        return None;
    }
    for session in state.manager.global_sessions.list_sessions().ok()? {
        if session.runtime.is_linggen() {
            continue;
        }
        let all = read(
            &state
                .manager
                .global_sessions
                .session_dir(&session.id)
                .join(FILE),
        );
        for entries in all
            .as_object()
            .into_iter()
            .flat_map(|obj| obj.values())
            .filter_map(Value::as_array)
        {
            if let Some(entry) = entries.iter().find(|entry| entry["subagentId"] == run) {
                let task = entry["sdkTaskId"].as_str()?.to_owned();
                return Some(super::stop_task(state, &session.id, &task).await);
            }
        }
    }
    Some(Err(anyhow::anyhow!("Claude subagent was not found")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_handed_back_report_is_kept_and_the_placeholder_is_no_result() {
        let mut entry = json!({"agentName":"general-purpose", "status":"running",
            "resultText":"", "toolSteps":[]});
        entry["report"] = json!("## Fixed\nAll tests pass.");
        let placeholder = json!({"text":"This agent's report was delivered to you as a message from x"});
        assert!(end(&mut entry, &placeholder, false));
        assert_eq!(entry["resultText"], "");
        assert_eq!(entry["status"], "done");
        let AgentEvent::SubagentResult { report, .. } = result_event(&entry, "sdk-sub-r-x", "r") else {
            panic!("not a result");
        };
        assert_eq!(report.as_deref(), Some("## Fixed\nAll tests pass."));
        // A child without a report keeps its end text as its result.
        let mut plain = json!({"status":"running", "resultText":"", "toolSteps":[]});
        end(&mut plain, &json!({"text":"done"}), false);
        assert_eq!(plain["resultText"], "done");
        assert!(plain.get("report").is_none());
    }

    #[test]
    fn restored_children_attach_to_their_own_last_parent_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().into());
        let session = store.session_dir("session");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(
            session.join(FILE),
            json!({"a":[{"subagentId":"child", "status":"running", "resultText":"private"}]})
                .to_string(),
        )
        .unwrap();
        let mut rows = vec![
            (json!({"from":"claude", "run_id":"a"}), "preamble".into()),
            (
                json!({"from":"claude", "run_id":"a"}),
                "parent answer".into(),
            ),
            (
                json!({"from":"claude", "run_id":"b"}),
                "another turn".into(),
            ),
        ];
        attach(&store, "session", &HashSet::new(), &mut rows);
        assert!(rows[0].0.get("subagents").is_none());
        assert!(rows[2].0.get("subagents").is_none());
        assert_eq!(rows[1].0["subagents"][0]["resultText"], "private");
        assert_eq!(rows[1].0["subagents"][0]["status"], "failed");
        assert_eq!(rows[1].1, "parent answer");
    }

    #[test]
    fn a_child_is_saved_under_the_run_that_spawned_it_only() {
        // One runner, two runs: the child spawned in run a ends during run b.
        let entries = BTreeMap::from([
            ("old".to_owned(), json!({"subagentId":"sdk-sub-a-old", "status":"done"})),
            ("new".to_owned(), json!({"subagentId":"sdk-sub-b-new", "status":"running"})),
        ]);
        let origins = BTreeMap::from([
            ("old".to_owned(), "a".to_owned()),
            ("new".to_owned(), "b".to_owned()),
        ]);
        let mut all = json!({"earlier":[{"subagentId":"kept"}]});
        store_by_origin(&mut all, &entries, &origins, "b");
        assert_eq!(all["a"], json!([{"subagentId":"sdk-sub-a-old", "status":"done"}]));
        assert_eq!(all["b"], json!([{"subagentId":"sdk-sub-b-new", "status":"running"}]));
        assert_eq!(all["earlier"][0]["subagentId"], "kept");
    }

    #[test]
    fn a_child_shows_running_only_while_the_live_runner_runs_it() {
        // `gone` ran when an engine restart took its runner; run r2 resumed
        // the same SDK task under a new tracking id, and that one still runs.
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().into());
        let session = store.session_dir("session");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(
            session.join(FILE),
            json!({
                "r1":[
                    {"subagentId":"sdk-sub-r1-live", "status":"running"},
                    {"subagentId":"sdk-sub-r1-gone", "status":"running"},
                ],
                "r2":[{"subagentId":"sdk-sub-r2-gone", "status":"running"}],
            })
            .to_string(),
        )
        .unwrap();
        let mut rows = vec![
            (json!({"from":"claude", "run_id":"r1"}), "answer".into()),
            (json!({"from":"claude", "run_id":"r2"}), "resumed".into()),
        ];
        let live = HashSet::from(["sdk-sub-r1-live".to_owned(), "sdk-sub-r2-gone".to_owned()]);
        attach(&store, "session", &live, &mut rows);
        assert_eq!(rows[0].0["subagents"][0]["status"], "running");
        assert_eq!(rows[0].0["subagents"][1]["status"], "failed");
        assert_eq!(rows[1].0["subagents"][0]["status"], "running");
    }

    #[test]
    fn a_childs_context_is_saved_and_comes_back_with_it() {
        let mut entry = json!({"subagentId":"sdk-sub-r1-c", "status":"done", "contextTokens":0});
        assert_eq!(set_context(&mut entry, &json!({"tokens":45012})), 45012);
        let entries = BTreeMap::from([("c".to_owned(), entry)]);
        let origins = BTreeMap::from([("c".to_owned(), "r1".to_owned())]);
        let mut all = json!({});
        store_by_origin(&mut all, &entries, &origins, "r1");
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().into());
        std::fs::create_dir_all(store.session_dir("session")).unwrap();
        std::fs::write(store.session_dir("session").join(FILE), all.to_string()).unwrap();
        let mut rows = vec![(json!({"from":"claude", "run_id":"r1"}), "answer".into())];
        attach(&store, "session", &HashSet::new(), &mut rows);
        assert_eq!(rows[0].0["subagents"][0]["contextTokens"], 45012);
    }

    #[test]
    fn a_child_copied_under_later_runs_attaches_once_as_last_seen() {
        // The file older engines wrote: every run of a runner repeats each
        // child, frozen at the state it had when that run ended.
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().into());
        let session = store.session_dir("session");
        std::fs::create_dir_all(&session).unwrap();
        let child = |status: &str| json!({"subagentId":"sdk-sub-r1-x", "task":"Fix it", "status":status});
        std::fs::write(
            session.join(FILE),
            json!({
                "r1":[child("running")],
                "r2":[{"subagentId":"sdk-sub-r2-y", "status":"done"}, child("running")],
                "r3":[{"subagentId":"sdk-sub-r2-y", "status":"done"}, child("done")],
            })
            .to_string(),
        )
        .unwrap();
        let mut rows: Vec<(Value, String)> = ["r1", "r2", "r3"]
            .iter()
            .map(|run| (json!({"from":"claude", "run_id":run}), "answer".into()))
            .collect();
        attach(&store, "session", &HashSet::new(), &mut rows);
        assert_eq!(rows[0].0["subagents"], json!([child("done")]));
        assert_eq!(rows[1].0["subagents"], json!([{"subagentId":"sdk-sub-r2-y", "status":"done"}]));
        assert!(rows[2].0.get("subagents").is_none());
    }
}
