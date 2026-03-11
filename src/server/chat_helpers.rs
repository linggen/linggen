use crate::agent_manager::AgentManager;
use crate::engine::AgentOutcome;
use crate::server::{ServerEvent, ServerState};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Message persistence
// ---------------------------------------------------------------------------

/// Emit a `ServerEvent::Message` **and** persist to session files.
pub(crate) async fn persist_and_emit_message(
    manager: &Arc<AgentManager>,
    events_tx: &broadcast::Sender<ServerEvent>,
    root: &Path,
    agent_id: &str,
    from: &str,
    to: &str,
    content: &str,
    session_id: Option<&str>,
    is_observation: bool,
) {
    let _ = events_tx.send(ServerEvent::Message {
        from: from.to_string(),
        to: to.to_string(),
        content: content.to_string(),
        session_id: session_id.map(|s| s.to_string()),
    });
    persist_message_only(manager, root, agent_id, from, to, content, session_id, is_observation)
        .await;
}

/// Emit a `ServerEvent::Message` and persist directly to a `SessionStore`.
/// Used for mission sessions that live outside any project.
pub(crate) async fn persist_and_emit_to_store(
    store: &crate::state_fs::SessionStore,
    events_tx: &broadcast::Sender<ServerEvent>,
    agent_id: &str,
    from: &str,
    to: &str,
    content: &str,
    session_id: Option<&str>,
    is_observation: bool,
) {
    let _ = events_tx.send(ServerEvent::Message {
        from: from.to_string(),
        to: to.to_string(),
        content: content.to_string(),
        session_id: session_id.map(|s| s.to_string()),
    });
    let sid = session_id.unwrap_or("default");
    let msg = crate::state_fs::sessions::ChatMsg {
        agent_id: agent_id.to_string(),
        from_id: from.to_string(),
        to_id: to.to_string(),
        content: content.to_string(),
        timestamp: crate::util::now_ts_secs(),
        is_observation,
    };
    if let Err(e) = store.add_chat_message(sid, &msg) {
        tracing::warn!("Failed to persist chat message to mission store: {}", e);
    }
}

/// Persist to flat-file session store without emitting an SSE event.
pub(crate) async fn persist_message_only(
    manager: &Arc<AgentManager>,
    root: &Path,
    agent_id: &str,
    from: &str,
    to: &str,
    content: &str,
    session_id: Option<&str>,
    is_observation: bool,
) {
    let sid = session_id.unwrap_or("default");
    if let Ok(ctx) = manager.get_or_create_project(root.to_path_buf()).await {
        let msg = crate::state_fs::sessions::ChatMsg {
            agent_id: agent_id.to_string(),
            from_id: from.to_string(),
            to_id: to.to_string(),
            content: content.to_string(),
            timestamp: crate::util::now_ts_secs(),
            is_observation,
        };
        if let Err(e) = ctx.sessions.add_chat_message(sid, &msg) {
            tracing::warn!("Failed to persist chat message: {}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Queue management
// ---------------------------------------------------------------------------

pub(crate) fn queue_key(project_root: &str, session_id: &str, agent_id: &str) -> String {
    format!("{project_root}|{session_id}|{agent_id}")
}

pub(crate) fn queue_preview(message: &str) -> String {
    const LIMIT: usize = 100;
    let trimmed = message.trim();
    if trimmed.len() <= LIMIT {
        trimmed.to_string()
    } else {
        // Find a char boundary at or before LIMIT to avoid panic on multi-byte UTF-8.
        let end = trimmed
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= LIMIT)
            .last()
            .unwrap_or(0);
        format!("{}...", &trimmed[..end])
    }
}

// ---------------------------------------------------------------------------
// Tool status formatting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ToolCallForUi {
    name: String,
    args: Option<serde_json::Value>,
}

fn parse_tool_call_from_json_line(line: &str) -> Option<ToolCallForUi> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    if parsed.get("type")?.as_str()? == "tool" {
        let tool = parsed
            .get("tool")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())?;
        return Some(ToolCallForUi {
            name: tool,
            args: parsed.get("args").cloned(),
        });
    }
    let kind = parsed.get("type").and_then(|v| v.as_str())?;
    if matches!(kind, "finalize_task") {
        return None;
    }
    if parsed.get("args").and_then(|v| v.as_object()).is_some() {
        return Some(ToolCallForUi {
            name: kind.to_string(),
            args: parsed.get("args").cloned(),
        });
    }
    None
}

fn preview_value(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let preview: String = value.chars().take(max_chars).collect();
        format!("{preview}... ({len} chars)", len = value.chars().count())
    }
}

fn basename(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/");
    normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .last()
        .map(|v| v.to_string())
        .unwrap_or_else(|| normalized.to_string())
}

fn first_string_arg(args: Option<&serde_json::Value>, keys: &[&str]) -> Option<String> {
    let obj = args.and_then(|v| v.as_object())?;
    for key in keys {
        if let Some(value) = obj.get(*key).and_then(|v| v.as_str()) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn parse_read_result_path_from_tool_header(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.len() < 10 || !trimmed[..10].eq_ignore_ascii_case("tool read:") {
        return None;
    }
    let mut rest = trimmed[10..].trim();
    if rest.len() >= 5 && rest[..5].eq_ignore_ascii_case("read:") {
        rest = rest[5..].trim();
    }
    let path = rest
        .split(" (truncated:")
        .next()
        .map(str::trim)
        .unwrap_or_default();
    if path.is_empty() {
        None
    } else {
        Some(preview_value(&basename(path), 140))
    }
}

fn tool_target_line(tool: &str, target: Option<&str>) -> String {
    match target.map(str::trim).filter(|v| !v.is_empty()) {
        Some(t) => format!("Used tool: {} · target={}", tool, t),
        None => format!("Used tool: {}", tool),
    }
}

fn parse_grep_target(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed == "(no matches)" {
        return None;
    }
    let path = trimmed.split(':').next().unwrap_or("").trim();
    if path.is_empty() || path == "(no matches)" {
        None
    } else {
        Some(path.to_string())
    }
}

fn summarize_targets(tool: &str, targets: &BTreeSet<String>, empty_label: &str) -> String {
    if targets.is_empty() {
        return tool_target_line(tool, Some(empty_label));
    }
    let first = targets.iter().next().cloned().unwrap_or_default();
    if targets.len() == 1 {
        return tool_target_line(tool, Some(&first));
    }
    let more = targets.len() - 1;
    tool_target_line(tool, Some(&format!("{first} (+{more} more)")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolStatusPhase {
    Start,
    Done,
    Failed,
}

pub(crate) fn tool_status_line(
    tool: &str,
    args: Option<&serde_json::Value>,
    phase: ToolStatusPhase,
) -> String {
    let name = tool.trim().to_lowercase();
    let read_path = first_string_arg(args, &["path", "file", "filepath"])
        .map(|path| preview_value(&basename(&path), 140));
    let bash_cmd = first_string_arg(args, &["cmd", "command"]).map(|cmd| preview_value(&cmd, 140));
    let grep_query =
        first_string_arg(args, &["query", "pattern", "q"]).map(|query| preview_value(&query, 140));
    let delegate_target = first_string_arg(args, &["target_agent_id"]);
    let glob_preview = args
        .and_then(|v| v.get("globs"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str())
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .map(|values| preview_value(&values.join(", "), 140));

    match name.as_str() {
        "read" => match phase {
            ToolStatusPhase::Start => read_path
                .map(|path| format!("Reading file: {path}"))
                .unwrap_or_else(|| "Reading file...".to_string()),
            ToolStatusPhase::Done => read_path
                .map(|path| format!("Read file: {path}"))
                .unwrap_or_else(|| "Read file".to_string()),
            ToolStatusPhase::Failed => read_path
                .map(|path| format!("Read failed: {path}"))
                .unwrap_or_else(|| "Read failed".to_string()),
        },
        "write" => match phase {
            ToolStatusPhase::Start => read_path
                .map(|path| format!("Writing file: {path}"))
                .unwrap_or_else(|| "Writing file...".to_string()),
            ToolStatusPhase::Done => read_path
                .map(|path| format!("Wrote file: {path}"))
                .unwrap_or_else(|| "Wrote file".to_string()),
            ToolStatusPhase::Failed => read_path
                .map(|path| format!("Write failed: {path}"))
                .unwrap_or_else(|| "Write failed".to_string()),
        },
        "edit" => match phase {
            ToolStatusPhase::Start => read_path
                .map(|path| format!("Editing file: {path}"))
                .unwrap_or_else(|| "Editing file...".to_string()),
            ToolStatusPhase::Done => read_path
                .map(|path| format!("Edited file: {path}"))
                .unwrap_or_else(|| "Edited file".to_string()),
            ToolStatusPhase::Failed => read_path
                .map(|path| format!("Edit failed: {path}"))
                .unwrap_or_else(|| "Edit failed".to_string()),
        },
        "bash" => match phase {
            ToolStatusPhase::Start => bash_cmd
                .map(|cmd| format!("Running command: {cmd}"))
                .unwrap_or_else(|| "Running command...".to_string()),
            ToolStatusPhase::Done => bash_cmd
                .map(|cmd| format!("Ran command: {cmd}"))
                .unwrap_or_else(|| "Ran command".to_string()),
            ToolStatusPhase::Failed => bash_cmd
                .map(|cmd| format!("Command failed: {cmd}"))
                .unwrap_or_else(|| "Command failed".to_string()),
        },
        "grep" => match phase {
            ToolStatusPhase::Start => grep_query
                .map(|query| format!("Searching: {query}"))
                .unwrap_or_else(|| "Searching...".to_string()),
            ToolStatusPhase::Done => grep_query
                .map(|query| format!("Searched: {query}"))
                .unwrap_or_else(|| "Searched".to_string()),
            ToolStatusPhase::Failed => grep_query
                .map(|query| format!("Search failed: {query}"))
                .unwrap_or_else(|| "Search failed".to_string()),
        },
        "glob" => match phase {
            ToolStatusPhase::Start => glob_preview
                .map(|globs| format!("Listing files: {globs}"))
                .unwrap_or_else(|| "Listing files...".to_string()),
            ToolStatusPhase::Done => glob_preview
                .map(|globs| format!("Listed files: {globs}"))
                .unwrap_or_else(|| "Listed files".to_string()),
            ToolStatusPhase::Failed => glob_preview
                .map(|globs| format!("List files failed: {globs}"))
                .unwrap_or_else(|| "List files failed".to_string()),
        },
        "webfetch" => {
            let url = first_string_arg(args, &["url"]).map(|u| preview_value(&u, 140));
            match phase {
                ToolStatusPhase::Start => url
                    .map(|u| format!("Fetching URL: {u}"))
                    .unwrap_or_else(|| "Fetching URL...".to_string()),
                ToolStatusPhase::Done => url
                    .map(|u| format!("Fetched URL: {u}"))
                    .unwrap_or_else(|| "Fetched URL".to_string()),
                ToolStatusPhase::Failed => url
                    .map(|u| format!("Fetch failed: {u}"))
                    .unwrap_or_else(|| "Fetch failed".to_string()),
            }
        }
        "websearch" => {
            let query =
                first_string_arg(args, &["query", "q"]).map(|q| preview_value(&q, 140));
            match phase {
                ToolStatusPhase::Start => query
                    .map(|q| format!("Searching web: {q}"))
                    .unwrap_or_else(|| "Searching web...".to_string()),
                ToolStatusPhase::Done => query
                    .map(|q| format!("Searched web: {q}"))
                    .unwrap_or_else(|| "Searched web".to_string()),
                ToolStatusPhase::Failed => query
                    .map(|q| format!("Web search failed: {q}"))
                    .unwrap_or_else(|| "Web search failed".to_string()),
            }
        }
        "task" => match phase {
            ToolStatusPhase::Start => delegate_target
                .map(|target| format!("Delegating to subagent: {target}"))
                .unwrap_or_else(|| "Delegating...".to_string()),
            ToolStatusPhase::Done => delegate_target
                .map(|target| format!("Delegated to subagent: {target}"))
                .unwrap_or_else(|| "Delegated to subagent".to_string()),
            ToolStatusPhase::Failed => delegate_target
                .map(|target| format!("Delegation failed: {target}"))
                .unwrap_or_else(|| "Delegation failed".to_string()),
        },
        "" => match phase {
            ToolStatusPhase::Start => "Calling tool...".to_string(),
            ToolStatusPhase::Done => "Used tool".to_string(),
            ToolStatusPhase::Failed => "Tool failed".to_string(),
        },
        _ => match phase {
            ToolStatusPhase::Start => format!("Calling tool: {}", tool.trim()),
            ToolStatusPhase::Done => format!("Used tool: {}", tool.trim()),
            ToolStatusPhase::Failed => format!("Tool failed: {}", tool.trim()),
        },
    }
}

// ---------------------------------------------------------------------------
// Message sanitization for UI
// ---------------------------------------------------------------------------

pub(crate) fn sanitize_message_for_ui(from: &str, content: &str) -> Option<String> {
    if from == "user" {
        return Some(content.to_string());
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum SuppressedToolBodyKind {
        Read,
        Grep,
        Glob,
    }

    #[derive(Debug, Clone)]
    enum SuppressedToolBody {
        Read,
        Grep {
            summary_idx: usize,
            targets: BTreeSet<String>,
            saw_no_matches: bool,
        },
        Glob {
            summary_idx: usize,
            targets: BTreeSet<String>,
        },
    }

    let finalize_mode = |mode: SuppressedToolBody, cleaned_lines: &mut Vec<String>| match mode {
        SuppressedToolBody::Read => {}
        SuppressedToolBody::Grep {
            summary_idx,
            targets,
            saw_no_matches,
        } => {
            if let Some(entry) = cleaned_lines.get_mut(summary_idx) {
                *entry = if saw_no_matches {
                    tool_target_line("Grep", Some("(no matches)"))
                } else {
                    summarize_targets("Grep", &targets, "(no matches)")
                };
            }
        }
        SuppressedToolBody::Glob {
            summary_idx,
            targets,
        } => {
            if let Some(entry) = cleaned_lines.get_mut(summary_idx) {
                *entry = summarize_targets("Glob", &targets, "(no files)");
            }
        }
    };

    let mode_kind = |mode: &SuppressedToolBody| match mode {
        SuppressedToolBody::Read => SuppressedToolBodyKind::Read,
        SuppressedToolBody::Grep { .. } => SuppressedToolBodyKind::Grep,
        SuppressedToolBody::Glob { .. } => SuppressedToolBodyKind::Glob,
    };

    let mut cleaned_lines: Vec<String> = Vec::new();
    let mut suppressed_tool_body: Option<SuppressedToolBody> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        let lower = line.to_lowercase();

        // Suppress verbose tool result bodies after compact status lines.
        if let Some(mut mode) = suppressed_tool_body.take() {
            let is_boundary = line.starts_with('{')
                || lower.starts_with("tool ")
                || lower.starts_with("tool_error:")
                || lower.starts_with("tool_not_allowed:");
            let should_suppress = match mode_kind(&mode) {
                // Read payloads can contain blank lines; only explicit boundaries end suppression.
                SuppressedToolBodyKind::Read => !is_boundary,
                // Grep/Glob payloads are line-oriented; an empty line can also end suppression.
                SuppressedToolBodyKind::Grep | SuppressedToolBodyKind::Glob => {
                    !is_boundary && !line.is_empty()
                }
            };
            if should_suppress {
                match &mut mode {
                    SuppressedToolBody::Read => {}
                    SuppressedToolBody::Grep {
                        targets,
                        saw_no_matches,
                        ..
                    } => {
                        if line.eq_ignore_ascii_case("(no matches)")
                            || lower.contains("no file candidates found")
                        {
                            *saw_no_matches = true;
                        } else if let Some(target) = parse_grep_target(line) {
                            targets.insert(target);
                        }
                    }
                    SuppressedToolBody::Glob { targets, .. } => {
                        targets.insert(line.to_string());
                    }
                }
                suppressed_tool_body = Some(mode);
                continue;
            }
            finalize_mode(mode, &mut cleaned_lines);
        }

        if line.is_empty() {
            if cleaned_lines.last().map(|v| !v.is_empty()).unwrap_or(false) {
                cleaned_lines.push(String::new());
            }
            continue;
        }

        // Tool call JSON — emit a compact status line instead of raw JSON.
        if let Some(tool_call) = parse_tool_call_from_json_line(line) {
            suppressed_tool_body = None;
            if !tool_call.name.is_empty() {
                cleaned_lines.push(tool_status_line(
                    &tool_call.name,
                    tool_call.args.as_ref(),
                    ToolStatusPhase::Start,
                ));
            }
            continue;
        }

        // Suppress raw JSON objects/arrays that are internal structured data
        // (e.g. finalize payloads) — not meant for display.
        // Exception: plan and finalize_task JSON are rendered as special blocks
        // by the UI (PlanBlock, TaskBlock) so they must pass through.
        if (line.starts_with('{') && line.ends_with('}'))
            || (line.starts_with('[') && line.ends_with(']'))
        {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                let is_special_block = val.get("type").and_then(|t| t.as_str())
                    .map(|t| t == "plan" || t == "finalize_task")
                    .unwrap_or(false);
                if !is_special_block {
                    continue;
                }
            }
        }

        // Tool errors — keep as compact summaries. Permission denials — suppress
        // (internal diagnostics, not useful for the user).
        if lower.starts_with("tool_not_allowed:") {
            suppressed_tool_body = None;
            // Skip — internal engine diagnostic, not user-facing.
            continue;
        }
        if lower.starts_with("tool_error:") {
            suppressed_tool_body = None;
            cleaned_lines.push(raw_line.to_string());
            continue;
        }

        // Tool result header lines ("Tool Read: ...", "Tool Grep: ...", "Tool Bash: ...").
        // Keep compact status lines and suppress verbose bodies for selected tools.
        if lower.starts_with("tool ") {
            if lower.starts_with("tool read:") {
                let status = parse_read_result_path_from_tool_header(line)
                    .map(|path| tool_target_line("Read", Some(&path)))
                    .unwrap_or_else(|| tool_target_line("Read", None));
                cleaned_lines.push(status);
                suppressed_tool_body = Some(SuppressedToolBody::Read);
            } else if lower.starts_with("tool grep:") {
                cleaned_lines.push(tool_target_line("Grep", None));
                let idx = cleaned_lines.len() - 1;
                suppressed_tool_body = Some(SuppressedToolBody::Grep {
                    summary_idx: idx,
                    targets: BTreeSet::new(),
                    saw_no_matches: false,
                });
            } else if lower.starts_with("tool glob:") {
                cleaned_lines.push(tool_target_line("Glob", None));
                let idx = cleaned_lines.len() - 1;
                suppressed_tool_body = Some(SuppressedToolBody::Glob {
                    summary_idx: idx,
                    targets: BTreeSet::new(),
                });
            } else if lower.starts_with("tool websearch:") {
                let query = line
                    .split("WebSearch:")
                    .nth(1)
                    .and_then(|s| s.split('(').next())
                    .map(|s| s.trim().trim_matches('"'))
                    .filter(|s| !s.is_empty());
                cleaned_lines.push(tool_target_line("WebSearch", query));
            } else if lower.contains("tool_not_allowed:") || lower.contains("tool_error:") {
                // Internal diagnostics from observation persistence — suppress.
                suppressed_tool_body = None;
            } else {
                suppressed_tool_body = None;
                cleaned_lines.push(raw_line.to_string());
            }
            continue;
        }

        // Internal boilerplate — hide.
        if lower.starts_with("starting autonomous loop for task:") {
            continue;
        }
        if lower == "(content omitted in chat; open the file viewer for full text)" {
            continue;
        }

        // Everything else — keep (model text, delegation messages, errors, etc.)
        cleaned_lines.push(raw_line.to_string());
    }

    if let Some(mode) = suppressed_tool_body.take() {
        finalize_mode(mode, &mut cleaned_lines);
    }

    let cleaned = cleaned_lines
        .join("\n")
        .replace("\n\n\n", "\n\n")
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned)
}

// ---------------------------------------------------------------------------
// Queue event emission
// ---------------------------------------------------------------------------

pub(crate) async fn emit_queue_updated(
    state: &Arc<ServerState>,
    project_root: &str,
    session_id: &str,
    agent_id: &str,
) {
    let key = queue_key(project_root, session_id, agent_id);
    let items = {
        let guard = state.queued_chats.lock().await;
        guard.get(&key).cloned().unwrap_or_default()
    };
    let _ = state.events_tx.send(ServerEvent::QueueUpdated {
        project_root: project_root.to_string(),
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        items,
    });
}

// ---------------------------------------------------------------------------
// Outcome events
// ---------------------------------------------------------------------------

pub(crate) fn emit_outcome_event(
    outcome: &AgentOutcome,
    events_tx: &broadcast::Sender<ServerEvent>,
    from_id: &str,
) {
    match outcome {
        AgentOutcome::Task(packet) => {
            let _ = events_tx.send(ServerEvent::Message {
                from: from_id.to_string(),
                to: "user".to_string(),
                content: serde_json::json!({
                    "type": "finalize_task",
                    "packet": packet
                })
                .to_string(),
                session_id: None, // enriched via agent_sessions map
            });
        }
        AgentOutcome::Patch(diff) => {
            let _ = events_tx.send(ServerEvent::Message {
                from: from_id.to_string(),
                to: "user".to_string(),
                content: serde_json::json!({
                    "type": "patch",
                    "diff": diff
                })
                .to_string(),
                session_id: None,
            });
        }
        AgentOutcome::Plan(plan) => {
            // Emit plan as a Message so it persists in chat history and
            // renders as a PlanBlock via tryRenderSpecialBlock.
            let _ = events_tx.send(ServerEvent::Message {
                from: from_id.to_string(),
                to: "user".to_string(),
                content: serde_json::json!({
                    "type": "plan",
                    "plan": plan
                })
                .to_string(),
                session_id: None,
            });
            // Note: PlanUpdate SSE is already emitted by the engine's
            // finalize_plan_mode → persist_and_emit_plan. Don't duplicate.
        }
        AgentOutcome::PlanApproved(plan) => {
            let _ = events_tx.send(ServerEvent::Message {
                from: from_id.to_string(),
                to: "user".to_string(),
                content: serde_json::json!({
                    "type": "plan",
                    "plan": plan
                })
                .to_string(),
                session_id: None,
            });
            let _ = events_tx.send(ServerEvent::PlanUpdate {
                agent_id: from_id.to_string(),
                plan: plan.clone(),
            });
        }
        _ => {}
    }
    // Always emit an Outcome event so the UI transitions the run from
    // RUNNING to completed and resets status to idle.
    let _ = events_tx.send(ServerEvent::Outcome {
        agent_id: from_id.to_string(),
        outcome: outcome.clone(),
    });
}

#[cfg(test)]
mod tests {
    use super::sanitize_message_for_ui;

    #[test]
    fn sanitize_hides_read_body_and_keeps_file_status() {
        let input = "Tool Read: Read: doc/storage.md (truncated: false)\n\nFilesystem layout for all persistent state.\nNo database — everything is files.\n\n(content omitted in chat; open the file viewer for full text)";
        let cleaned = sanitize_message_for_ui("system", input).expect("expected sanitized output");
        assert_eq!(cleaned, "Used tool: Read · target=storage.md");
    }

    #[test]
    fn sanitize_handles_tool_read_header_without_path() {
        let input = "Tool Read: Read:\n\nsecret content";
        let cleaned = sanitize_message_for_ui("system", input).expect("expected sanitized output");
        assert_eq!(cleaned, "Used tool: Read");
    }

    #[test]
    fn sanitize_hides_grep_body_and_keeps_compact_status() {
        let input = "Tool Grep: Grep matches:\nsrc/a.rs:10: fn main() {}\nsrc/b.rs:42: let x = 1;";
        let cleaned = sanitize_message_for_ui("system", input).expect("expected sanitized output");
        assert_eq!(cleaned, "Used tool: Grep · target=src/a.rs (+1 more)");
    }

    #[test]
    fn sanitize_hides_glob_body_and_keeps_compact_status() {
        let input = "Tool Glob: files:\nsrc/a.rs\nsrc/b.rs\nsrc/c.rs";
        let cleaned = sanitize_message_for_ui("system", input).expect("expected sanitized output");
        assert_eq!(cleaned, "Used tool: Glob · target=src/a.rs (+2 more)");
    }
}
