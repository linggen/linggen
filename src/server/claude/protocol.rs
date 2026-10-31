use super::{runtime::ClaudeSession, state, AGENT_ID};
use crate::engine::agent::{AgentEvent, AgentRunStatus};
use crate::server::ServerEvent;
use crate::state_fs::sessions::ChatMsg;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot, OwnedMutexGuard};

/// How long a runner with nothing to do stays up before it exits.
fn idle_exit() -> Duration {
    std::env::var("LINGGEN_CLAUDE_IDLE_EXIT_SECS")
        .ok()
        .and_then(|secs| secs.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(20 * 60))
}

/// A turn sent where a runner that was ending could not take it: start a
/// fresh runner for it instead.
pub(super) const RETRY: &str = "retry on a fresh runner";

/// One person's message for the session's runner: its run, and where the
/// turn's outcome goes once the SDK answers it.
pub(crate) struct TurnRequest {
    pub run_id: String,
    pub prompt: String,
    pub images: Vec<String>,
    pub uuid: String,
    pub turn_memory: String,
    pub done: oneshot::Sender<std::result::Result<(), String>>,
}

/// A turn the runner ended with: its run and where its outcome goes.
pub(super) type Stranded = (String, oneshot::Sender<std::result::Result<(), String>>);

/// A turn the SDK has, waiting for its result.
struct Pending {
    run_id: String,
    uuid: String,
    done: oneshot::Sender<std::result::Result<(), String>>,
    /// The SDK started on it: a Steer may join it now.
    begun: bool,
}

/// A turn the SDK started on its own (a background task's notification):
/// its run, its life, and the session's turn lock while it runs.
struct Unprompted {
    run_id: String,
    _life: crate::engine::agent::runs_api::RunLife,
    _turn: Option<OwnedMutexGuard<()>>,
}

#[derive(Default)]
struct Turns {
    pending: VecDeque<Pending>,
    unprompted: Option<Unprompted>,
    goal: bool,
}

impl Turns {
    fn idle(&self) -> bool {
        self.pending.is_empty() && self.unprompted.is_none() && !self.goal
    }
}

/// Output from the main thread, not from a subagent: a subagent's events
/// carry its agent_id, its permission and question requests in their input.
fn main_thread(event: &Value) -> bool {
    event["agent_id"].as_str().is_none_or(str::is_empty)
        && event["input"]["agent_id"].as_str().is_none_or(str::is_empty)
}

impl ClaudeSession {
    /// Serve the runner until it ends: the first turn is already sent with
    /// `start`; later turns come on `turns`.
    pub async fn consume(
        &mut self,
        lines: &mut Lines<BufReader<ChildStdout>>,
        stdin: &mut ChildStdin,
        first: TurnRequest,
        mut turns: mpsc::UnboundedReceiver<TurnRequest>,
        controls: mpsc::UnboundedReceiver<super::Control>,
        stranded: &mut Vec<Stranded>,
    ) -> Result<()> {
        let mut active = HashMap::new();
        let mut subagents = super::subagents::Subagents::default();
        let mut state = Turns::default();
        state.pending.push_back(Pending {
            run_id: first.run_id,
            uuid: first.uuid,
            done: first.done,
            begun: false,
        });
        let result = self
            .read_events(lines, stdin, &mut active, &mut state, &mut turns, controls, &mut subagents)
            .await;
        if let Err(error) = subagents.finish(self).await {
            tracing::warn!("Could not finish SDK child views: {error}");
        }
        let ids: Vec<String> = self
            .state
            .pending_ask_user
            .lock()
            .await
            .iter()
            .filter(|(_, item)| {
                item.session_id.as_deref() == Some(&self.meta.id)
                    && (item.agent_id == AGENT_ID || item.agent_id.starts_with("sdk-sub-"))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.dismiss_question(&id).await;
        }
        for (id, tool) in active {
            self.tool_end(&id, &tool, true, &json!("Task stopped"))
                .await;
        }
        // The runner is gone, and the SDK's background tasks with it.
        if self.state.tasks.interrupt_running(&self.meta.id) {
            crate::server::tasks::emit(&self.state, &self.meta.id);
        }
        // The runner's turns still waiting fail once its stderr is known.
        stranded.extend(state.pending.drain(..).map(|turn| (turn.run_id, turn.done)));
        if let Some(turn) = state.unprompted.take() {
            let ended = result.as_ref().err().map_or("Claude Agent SDK ended".into(), ToString::to_string);
            self.finish_unprompted(turn, Err(ended)).await;
        }
        // Turns that reached a runner already ending: a fresh one takes them.
        turns.close();
        while let Ok(turn) = turns.try_recv() {
            let _ = turn.done.send(Err(RETRY.into()));
        }
        result
    }

    /// This run's stream is over: the chat stops showing it as typing.
    pub(super) fn end_stream(&self, run_id: &str) {
        let _ = self.state.events_tx.send(ServerEvent::Token {
            agent_id: AGENT_ID.into(),
            token: String::new(),
            done: true,
            thinking: false,
            session_id: Some(self.meta.id.clone()),
            run_id: Some(run_id.into()),
        });
    }

    /// Events belong to the oldest waiting turn, else to the SDK's own.
    fn follow(&mut self, turns: &Turns) {
        if let Some(turn) = turns.pending.front() {
            self.run_id = turn.run_id.clone();
        } else if let Some(turn) = &turns.unprompted {
            self.run_id = turn.run_id.clone();
        }
    }

    /// The SDK started a turn with no message of ours: a background task
    /// ended. It gets a run of its own, and holds the turn lock while it runs
    /// so the person's next message waits for it as for any turn.
    async fn begin_unprompted(&mut self) -> Option<Unprompted> {
        let turn = self
            .state
            .manager
            .session_turn(&self.meta.id)
            .try_lock_owned()
            .ok();
        let run_id = self
            .state
            .manager
            .begin_agent_run(
                std::path::Path::new(&self.meta.cwd),
                Some(&self.meta.id),
                AGENT_ID,
                None,
                Some("A background task ended".into()),
            )
            .await
            .ok()?;
        let _life = self.state.manager.run_life(&run_id);
        Some(Unprompted { run_id, _life, _turn: turn })
    }

    async fn finish_unprompted(&self, turn: Unprompted, outcome: std::result::Result<(), String>) {
        self.end_stream(&turn.run_id);
        let status = if outcome.is_ok() { AgentRunStatus::Completed } else { AgentRunStatus::Failed };
        let _ = self
            .state
            .manager
            .finish_agent_run(&turn.run_id, status, outcome.err())
            .await;
        crate::server::chat::close_turn(
            &self.state,
            AGENT_ID.into(),
            Some(self.meta.id.clone()),
            Some(turn.run_id),
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn read_events(
        &mut self,
        lines: &mut Lines<BufReader<ChildStdout>>,
        stdin: &mut ChildStdin,
        active: &mut HashMap<String, String>,
        turns_state: &mut Turns,
        turns: &mut mpsc::UnboundedReceiver<TurnRequest>,
        mut controls: mpsc::UnboundedReceiver<super::Control>,
        subagents: &mut super::subagents::Subagents,
    ) -> Result<()> {
        let mut has_text = false;
        let mut last_text = String::new();
        // The run the last result ended: its next-prompt hint is stamped with it.
        let mut finished_run: Option<String> = None;
        let mut requests = tokio::task::JoinSet::new();
        let mut request_tasks = HashMap::new();
        let mut questions: HashMap<i64, String> = HashMap::new();
        let mut changes = HashMap::new();
        let mut next_change = 0_i64;
        let mut controls_open = true;
        let mut turns_open = true;
        let mut idle_since = Instant::now();
        let mut ending = false;
        self.follow(turns_state);
        // One clock for the whole runner: a sleep made fresh each pass would
        // never fire while tokens stream.
        let mut tick = super::runtime::cancel_tick();
        loop {
            let line = tokio::select! {
                response = requests.join_next(), if !requests.is_empty() => {
                    if let Some(Ok((id, response))) = response {
                        request_tasks.remove(&id);
                        questions.remove(&id);
                        let response: Result<Value> = response;
                        let mut reply = response.unwrap_or_else(|error| json!({"allow":false,"error":error.to_string(),"reason":error.to_string()}));
                        reply["type"] = json!("reply"); reply["id"] = json!(id);
                        stdin.write_all(format!("{reply}\n").as_bytes()).await?;
                    }
                    continue;
                }
                turn = turns.recv(), if turns_open && !ending => {
                    let Some(turn) = turn else { turns_open = false; continue; };
                    next_change += 1;
                    let (reply, _ack) = oneshot::channel();
                    changes.insert(next_change, reply);
                    let message = json!({"type":"prompt", "id":next_change, "text":turn.prompt,
                        "images":turn.images, "uuid":turn.uuid, "turnMemory":turn.turn_memory});
                    stdin.write_all(format!("{message}\n").as_bytes()).await?;
                    turns_state.pending.push_back(Pending { run_id: turn.run_id, uuid: turn.uuid, done: turn.done, begun: false });
                    if turns_state.pending.len() == 1 && turns_state.unprompted.is_none() {
                        has_text = false;
                        last_text.clear();
                    }
                    self.follow(turns_state);
                    continue;
                }
                change = controls.recv(), if controls_open => {
                    if let Some(change) = change {
                        // A steer whose sender gave up waiting already runs
                        // as the next turn; sending it too would run it twice.
                        // Before the SDK began the turn, a `now` message would
                        // run ahead of it: it waits as the next turn instead.
                        if let super::Control::Steer { reply, .. } = &change {
                            if reply.is_closed() { continue; }
                        }
                        let begun = turns_state.pending.front().is_some_and(|turn| turn.begun);
                        next_change += 1;
                        let (message, reply) = match change {
                            super::Control::Mode(change) => (json!({"type":"mode", "id":next_change, "mode":change.mode}), change.reply),
                            super::Control::StopTask { task_id, reply } => (json!({"type":"stop_task", "id":next_change, "task_id":task_id}), reply),
                            super::Control::Steer { reply, .. } if !begun => {
                                let _ = reply.send(Err("The turn has not begun.".into()));
                                continue;
                            }
                            super::Control::Steer { text, images, delivery, reply } => (json!({"type":"steer", "id":next_change, "text":text, "images":images, "priority":delivery.priority()}), reply),
                            super::Control::Interrupt { reply } => (json!({"type":"interrupt", "id":next_change}), reply),
                            super::Control::End => {
                                ending = true;
                                stdin.write_all(b"{\"type\":\"end\"}\n").await?;
                                continue;
                            }
                        };
                        changes.insert(next_change, reply);
                        stdin.write_all(format!("{message}\n").as_bytes()).await?;
                    } else { controls_open = false; }
                    continue;
                }
                line = lines.next_line() => line?.context("Claude Agent SDK exited without a result")?,
                _ = tick.tick() => {
                    // Nothing to do for a while: the runner exits. A running
                    // background task, a waiting turn or a goal keeps it.
                    let tasks_running = self.state.tasks.list(&self.meta.id).iter().any(|t| t.is_running());
                    if !ending && turns_state.idle() && requests.is_empty() && !tasks_running
                        && idle_since.elapsed() >= idle_exit()
                    {
                        ending = true;
                        stdin.write_all(b"{\"type\":\"end\"}\n").await?;
                    }
                    if !turns_state.idle() || tasks_running { idle_since = Instant::now(); }
                    continue;
                }
            };
            let event: Value = serde_json::from_str(&line).context("Invalid SDK bridge message")?;
            let kind = event["type"].as_str().unwrap_or_default().to_owned();
            // Main-thread output with no turn of ours waiting: the SDK began
            // one itself. A subagent's output never does: a background child
            // runs on after its turn and must not hold the person's next one.
            if matches!(kind.as_str(), "token" | "text" | "tool_start" | "request")
                && main_thread(&event)
            {
                if let Some(turn) = turns_state.pending.front_mut() {
                    turn.begun = true;
                } else if turns_state.unprompted.is_none() {
                    turns_state.unprompted = self.begin_unprompted().await;
                    has_text = false;
                    last_text.clear();
                    self.follow(turns_state);
                }
            }
            if subagents.handle(self, &event).await? {
                continue;
            }
            match kind.as_str() {
                "init" => {
                    state::update_checkpoint(&self.dir(), |checkpoint| {
                        // Where an older conversation's shell went is not this one's.
                        let new_id = Some(&event["session_id"]);
                        if let Some(c) = checkpoint.as_object_mut().filter(|c| c.get("session_id") != new_id) {
                            c.remove("followed");
                        }
                        checkpoint["session_id"] = event["session_id"].clone();
                        checkpoint["cwd"] = json!(self.meta.cwd);
                        state::set_commands(checkpoint, &event["commands"]);
                    })?;
                    self.sync_mode(&event).await?;
                }
                "token" => {
                    if let Some(text) = event["text"].as_str() {
                        let _ = self.state.events_tx.send(ServerEvent::Token {
                            agent_id: AGENT_ID.into(),
                            token: text.into(),
                            done: false,
                            thinking: false,
                            session_id: Some(self.meta.id.clone()),
                            run_id: Some(self.run_id.clone()),
                        });
                    }
                }
                "status" => {
                    self.sync_mode(&event).await?;
                    if event["status"] == "compacting" {
                        self.emit(AgentEvent::AgentStatus {
                            agent_id: AGENT_ID.into(),
                            status: "thinking".into(),
                            detail: Some("Thinking (compacting context)".into()),
                            parent_id: None,
                            run_id: None,
                            parent_run_id: None,
                        })
                        .await;
                    }
                }
                // The main thread's shell moved: the session's folder follows,
                // and the conversation resumes from there too.
                "cwd" => {
                    let cwd = event["cwd"].as_str().context("SDK cwd has no folder")?;
                    state::update_checkpoint(&self.dir(), |checkpoint| {
                        checkpoint["followed"] = json!(cwd);
                    })?;
                    crate::server::background::note_working_folder(
                        &self.state,
                        self.meta.id.clone(),
                        cwd.into(),
                    );
                }
                "commands" => {
                    state::update_checkpoint(&self.dir(), |checkpoint| {
                        state::set_commands(checkpoint, &event["commands"]);
                    })?;
                }
                "text" => {
                    if let Some(text) = event["text"].as_str().filter(|s| !s.is_empty()) {
                        self.text(text).await;
                        last_text = text.into();
                        has_text = true;
                    }
                }
                "tool_start" => {
                    let id = event["id"].as_str().context("SDK tool has no id")?;
                    let tool = event["tool"].as_str().context("SDK tool has no name")?;
                    let args =
                        crate::engine::render::sanitize_tool_args_for_display(tool, &event["args"]);
                    self.emit(AgentEvent::ContentBlockStart {
                        agent_id: AGENT_ID.into(),
                        block_id: id.into(),
                        block_type: "tool_use".into(),
                        tool: Some(tool.into()),
                        args: Some(args.to_string()),
                        parent_id: None,
                        run_id: None,
                        parent_run_id: None,
                    })
                    .await;
                    self.save(
                        AGENT_ID,
                        json!({"type":"tool", "tool":tool, "args":args}).to_string(),
                        true,
                        false,
                    );
                    active.insert(id.into(), tool.into());
                }
                "tool_end" => {
                    let id = event["id"].as_str().unwrap_or_default();
                    if let Some(tool) = active.remove(id) {
                        self.tool_end(
                            id,
                            &tool,
                            event["failed"].as_bool().unwrap_or(false),
                            &event["result"],
                        )
                        .await;
                    }
                    // The main thread's tool round is over: what the person
                    // typed meanwhile goes in before Claude's next request.
                    if active.is_empty() && main_thread(&event) {
                        self.state.claude.deliver_waiting(&self.meta.id);
                    }
                }
                "request" => {
                    let id = event["id"].as_i64().context("SDK request has no id")?;
                    let question_id = super::interaction::new_question_id();
                    questions.insert(id, question_id.clone());
                    let interaction = super::interaction::Interaction {
                        agent_id: event["input"]["agent_id"]
                            .as_str()
                            .map(|id| subagents.tracking(id, &self.run_id))
                            .unwrap_or_else(|| AGENT_ID.into()),
                        state: self.state.clone(),
                        meta: self.meta.clone(),
                        run_id: self.run_id.clone(),
                        question_id,
                    };
                    let task =
                        requests.spawn(async move { (id, interaction.request(&event).await) });
                    request_tasks.insert(id, task);
                }
                "cancel_request" => {
                    let id = event["id"].as_i64().unwrap_or_default();
                    if let Some(task) = request_tasks.remove(&id) {
                        task.abort();
                    }
                    if let Some(question) = questions.remove(&id) {
                        self.dismiss_question(&question).await;
                    }
                }
                "task" | "tasks" => {
                    if super::tasks::apply(&self.state.tasks, &self.meta.id, &self.run_id, &event) {
                        crate::server::tasks::emit(&self.state, &self.meta.id);
                    }
                }
                "mode_result" => {
                    if let Some(change) = changes.remove(&event["id"].as_i64().unwrap_or_default())
                    {
                        let _ = change.send(if event["ok"] == true {
                            Ok(())
                        } else {
                            Err(event["error"].as_str().unwrap_or("Mode unavailable").into())
                        });
                    }
                }
                "reset" => {
                    super::usage::clear(&self.dir())?;
                    let _ = std::fs::remove_file(self.dir().join("claude-subagents.json"));
                    state::update_checkpoint(&self.dir(), |checkpoint| {
                        checkpoint["session_id"] = event["session_id"].clone();
                    })?;
                    self.state.claude.forget_session(&self.meta.id);
                    super::trust::forget_decline(&self.dir());
                    self.state
                        .manager
                        .global_sessions
                        .clear_chat_history(&self.meta.id)?;
                    let _ = self.state.events_tx.send(ServerEvent::SessionReset {
                        session_id: self.meta.id.clone(),
                    });
                    has_text = false;
                    last_text.clear();
                }
                "result" => {
                    super::usage::record(&self.dir(), &event)?;
                    let outcome = if event["success"].as_bool().unwrap_or(false) {
                        Ok(())
                    } else {
                        Err(format!("Claude Agent SDK: {}", event["errors"]))
                    };
                    if outcome.is_ok() && !has_text {
                        if let Some(text) = event["text"].as_str().filter(|s| !s.is_empty()) {
                            self.text(text).await;
                            last_text = text.into();
                        }
                    }
                    if !last_text.is_empty() {
                        self.finalize_text(&last_text).await;
                    }
                    // The turns this result answered: the messages it took in.
                    let answered: Vec<&str> = event["uuids"].as_array().into_iter().flatten()
                        .filter_map(Value::as_str).collect();
                    let mut completed = false;
                    while let Some(at) = turns_state.pending.iter().position(|turn| answered.contains(&turn.uuid.as_str())) {
                        let turn = turns_state.pending.remove(at).expect("position is in range");
                        self.end_stream(&turn.run_id);
                        finished_run = Some(turn.run_id.clone());
                        let _ = turn.done.send(outcome.clone());
                        completed = true;
                    }
                    if !completed {
                        if let Some(turn) = turns_state.unprompted.take() {
                            finished_run = Some(turn.run_id.clone());
                            self.finish_unprompted(turn, outcome).await;
                        } else if let Some(turn) = turns_state.pending.pop_front() {
                            self.end_stream(&turn.run_id);
                            finished_run = Some(turn.run_id.clone());
                            let _ = turn.done.send(outcome);
                        }
                    }
                    has_text = false;
                    last_text.clear();
                    idle_since = Instant::now();
                    self.follow(turns_state);
                }
                // The context window as it stands, after a turn.
                "context" => {
                    let tokens = event["tokens"].as_u64().unwrap_or(0) as usize;
                    let limit = event["limit"].as_u64().filter(|l| *l > 0).map(|l| l as usize);
                    self.emit(AgentEvent::ContextUsage {
                        agent_id: AGENT_ID.into(),
                        stage: "sdk".into(),
                        message_count: 0,
                        char_count: 0,
                        estimated_tokens: tokens,
                        token_limit: limit,
                        // The window, not spend: the chat sums these as usage.
                        actual_prompt_tokens: None,
                        actual_completion_tokens: None,
                        compressed: false,
                        summary_count: 0,
                        run_id: None,
                    })
                    .await;
                }
                "goal" => {
                    turns_state.goal = event["active"] == true;
                }
                // The SDK's guess at the person's next message, after a
                // result: the input's hint, as a native turn's would be.
                "suggestion" => {
                    let text = event["text"].as_str().unwrap_or_default().trim();
                    tracing::debug!("[claude] SDK next-prompt suggestion for {:?}: {text:?}", finished_run);
                    if !text.is_empty() {
                        self.emit(AgentEvent::Followups {
                            agent_id: AGENT_ID.into(),
                            items: vec![text.to_string()],
                            run_id: finished_run.clone(),
                        })
                        .await;
                    }
                }
                "done" => return Ok(()),
                "error" => anyhow::bail!(
                    "Claude Agent SDK: {}",
                    event["message"].as_str().unwrap_or("runtime error")
                ),
                _ => anyhow::bail!("Unknown SDK bridge message"),
            }
        }
    }

    async fn sync_mode(&mut self, event: &Value) -> Result<()> {
        if let Ok(mode) = serde_json::from_value(event["permissionMode"].clone()) {
            if let Some(mut meta) = self
                .state
                .manager
                .global_sessions
                .get_session_meta(&self.meta.id)?
            {
                meta.claude_permission_mode = mode;
                self.meta.claude_permission_mode = mode;
                if mode == crate::state_fs::sessions::ClaudePermissionMode::Auto {
                    state::mark_auto_verified(&self.dir());
                }
                self.state
                    .manager
                    .global_sessions
                    .update_session_meta(&meta)?;
            }
            self.emit(AgentEvent::StateUpdated).await;
        }
        Ok(())
    }

    async fn dismiss_question(&self, id: &str) {
        if self
            .state
            .pending_ask_user
            .lock()
            .await
            .remove(id)
            .is_some()
        {
            let _ = self.state.events_tx.send(ServerEvent::WidgetResolved {
                widget_id: id.into(),
                session_id: Some(self.meta.id.clone()),
            });
        }
    }

    pub async fn text(&self, text: &str) {
        self.save(AGENT_ID, text.into(), false, false);
        self.emit(AgentEvent::TextSegment {
            agent_id: AGENT_ID.into(),
            text: text.into(),
            parent_id: None,
            run_id: None,
        })
        .await;
    }

    pub async fn finalize_text(&self, text: &str) {
        let _ = self.state.events_tx.send(ServerEvent::Message {
            from: AGENT_ID.into(),
            to: "user".into(),
            content: text.into(),
            session_id: Some(self.meta.id.clone()),
            run_id: Some(self.run_id.clone()),
            parent_agent_id: None,
            client_id: None,
            via: None,
            via_note: None,
            images: Vec::new(),
        });
    }

    fn save(&self, from: &str, content: String, observation: bool, failed: bool) {
        let row = ChatMsg {
            run_id: Some(self.run_id.clone()),
            agent_id: AGENT_ID.into(),
            from_id: from.into(),
            to_id: if from == "system" { AGENT_ID } else { "user" }.into(),
            content,
            timestamp: crate::util::now_ts_secs(),
            is_observation: observation,
            failed,
            client_id: None,
            via: None,
            via_note: None,
            images: Vec::new(),
        };
        if let Err(error) = self
            .state
            .manager
            .global_sessions
            .add_chat_message(&self.meta.id, &row)
        {
            tracing::error!("Could not save Claude event: {error}");
        }
    }

    async fn tool_end(&self, id: &str, tool: &str, failed: bool, result: &Value) {
        let rendered: String = result
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| result.to_string())
            .chars()
            .take(16_384)
            .collect();
        self.emit(AgentEvent::ContentBlockUpdate {
            agent_id: AGENT_ID.into(),
            block_id: id.into(),
            status: Some(if failed { "failed" } else { "done" }.into()),
            summary: Some(rendered.clone()),
            is_error: Some(failed),
            parent_id: None,
            extra: None,
            run_id: None,
            parent_run_id: None,
        })
        .await;
        self.save("system", format!("Tool {tool}: {rendered}"), true, failed);
    }
}

#[cfg(test)]
mod tests {
    use super::main_thread;
    use serde_json::json;

    #[test]
    fn only_main_thread_output_counts_as_the_sdks_own_turn() {
        assert!(main_thread(&json!({"type":"text","text":"hi"})));
        assert!(main_thread(&json!({"type":"request","kind":"permission","input":{"tool":"Bash"}})));
        assert!(!main_thread(&json!({"type":"tool_start","agent_id":"c1"})));
        assert!(!main_thread(
            &json!({"type":"request","kind":"permission","input":{"tool":"Bash","agent_id":"c1"}})
        ));
    }
}
