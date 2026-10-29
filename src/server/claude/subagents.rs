//! SDK children use the existing SubagentPane contract; no native agent loop.
use super::{runtime::ClaudeSession, AGENT_ID};
use crate::engine::agent::AgentEvent;
use crate::server::{ServerEvent, ServerState};
use crate::state_fs::sessions::SessionStore;
use anyhow::Result;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

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
    pub async fn handle(&mut self, host: &ClaudeSession, event: &Value) -> Result<bool> {
        let kind = event["type"].as_str().unwrap_or_default();
        let Some(raw) = event["agent_id"].as_str().filter(|id| !id.is_empty()) else {
            return Ok(false);
        };
        if !kind.starts_with("subagent_") && !matches!(kind, "tool_start" | "tool_end") {
            return Ok(false);
        }
        let id = tracking_id(&host.run_id, raw);
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
                        parent_run_id: Some(host.run_id.clone()),
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
                entry["contextTokens"] =
                    json!(event["usage"]["total_tokens"].as_u64().unwrap_or(0));
                entry["currentActivity"] = event["activity"].clone();
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
                    parent_run_id: Some(host.run_id.clone()),
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
                    parent_run_id: Some(host.run_id.clone()),
                })
                .await;
            }
            "subagent_end" => {
                let failed = matches!(event["status"].as_str(), Some("failed" | "stopped"));
                let first_end =
                    entry["status"] == "running" || failed && entry["status"] != "failed";
                entry["status"] = json!(if failed { "failed" } else { "done" });
                entry["currentActivity"] = Value::Null;
                if entry["resultText"].as_str().unwrap_or_default().is_empty() {
                    entry["resultText"] = json!(event["text"].as_str().unwrap_or_default());
                }
                for step in entry["toolSteps"].as_array_mut().unwrap() {
                    if step["status"] == "running" {
                        step["status"] = json!("failed");
                    }
                }
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
                    host.emit(AgentEvent::SubagentResult {
                        parent_id: AGENT_ID.into(),
                        subagent_id: entry["agentName"].as_str().unwrap_or_default().into(),
                        outcome: crate::engine::AgentOutcome::None,
                        subagent_run_id: Some(id),
                        parent_run_id: Some(host.run_id.clone()),
                    })
                    .await;
                }
            }
            _ => {}
        }
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
pub(crate) fn attach(
    store: &SessionStore,
    sid: &str,
    running: bool,
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
    let mut placed = std::collections::HashSet::new();
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
            if !running && entry["status"] == "running" {
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
        attach(&store, "session", false, &mut rows);
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
        attach(&store, "session", false, &mut rows);
        assert_eq!(rows[0].0["subagents"], json!([child("done")]));
        assert_eq!(rows[1].0["subagents"], json!([{"subagentId":"sdk-sub-r2-y", "status":"done"}]));
        assert!(rows[2].0.get("subagents").is_none());
    }
}
