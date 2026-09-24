//! Everything the daemon runs beside the HTTP server: the loops, watchers
//! and pre-warms `prepare_server` starts once the state exists.

use super::{api, rtc, yinyue_moments, yinyue_watch, ServerState};
use crate::engine::agent::AgentEvent;
use crate::engine::events::{AgentStatusKind, ServerEvent};
use crate::mcp_client::McpServerConfig;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

pub(super) type AgentEvents = mpsc::UnboundedReceiver<(AgentEvent, Option<String>)>;

pub(super) fn spawn_background_tasks(
    state: &Arc<ServerState>,
    pet_enabled: bool,
    idle_shutdown_secs: Option<u64>,
    agent_events_rx: AgentEvents,
) {
    flush_token_usage(state);
    watch_skill_sync(state);
    prewarm_memory(state);
    prewarm_runtime(state, pet_enabled);
    prewarm_voice(state, pet_enabled);
    connect_mcp_servers(state);
    if let Some(timeout) = idle_shutdown_secs.filter(|t| *t > 0) {
        idle_shutdown(state, timeout);
    }
    tokio::spawn(bridge_agent_events(state.clone(), agent_events_rx));
    tokio::spawn(
        crate::extensions::missions::scheduler::mission_scheduler_loop(state.scheduler_host()),
    );
    spawn_companion(state);
    spawn_perception(state);
    sweep_stale_runs(state);

    // Spawn remote relay tasks (heartbeat + offer polling) if remote.toml exists.
    // One-time cleanup of the retired remote.toml (its token was a second copy
    // of the account credential and drifted; see account::migrate_remote_toml).
    crate::account::migrate_remote_toml();
    rtc::relay::spawn_relay_tasks(state.clone());
    // A pairing QR is only good while someone is looking at it.
    api::pair::pair_window_watch();
    auto_connect_rooms(state);
}

/// Flush token usage to disk every 30 seconds.
fn flush_token_usage(state: &Arc<ServerState>) {
    let usage = state.token_usage.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            usage.lock().await.flush();
        }
    });
}

/// Push Mac-side changes to paired devices instead of making them poll.
/// Skill-declared sync dirs get theirs from frontmatter — no app named here.
fn watch_skill_sync(state: &Arc<ServerState>) {
    let sync_state = state.clone();
    tokio::spawn(async move { api::skill_sync::spawn_watchers(sync_state).await });
}

/// Pre-warm ling-mem when any installed skill uses scoped memory
/// (`memory-context`). Those apps auto-recall every turn, so get the
/// possibly-first-time (~100 MB) install + daemon start off the hot path.
/// No-op for daemons with no memory-using skill; on failure the per-call
/// autostart still covers it.
fn prewarm_memory(state: &Arc<ServerState>) {
    let skills = state.skills.clone();
    tokio::spawn(async move {
        let uses_memory = skills.list_skills().await.iter().any(|s| {
            s.memory_context
                .as_deref()
                .map(|c| !c.trim().is_empty())
                .unwrap_or(false)
        });
        if uses_memory {
            match crate::engine::tools::memory_http::autostart().await {
                Ok(()) => info!("ling-mem pre-warmed (a skill uses scoped memory)"),
                Err(e) => {
                    info!("ling-mem pre-warm deferred to first use: {e}")
                }
            }
        }
    });
}

/// Managed Python runtime: fetch/repair in the background so no skill or
/// TTS request ever waits on a download. The voice stages (venv + model,
/// ~2.1 GB) run only when the pet is enabled — a disabled pet never pulls
/// them. Progress goes out live on the `tasks` topic and is retained for
/// surfaces that attach later.
fn prewarm_runtime(state: &Arc<ServerState>, pet_enabled: bool) {
    let state = state.clone();
    let progress: crate::runtime::Progress = std::sync::Arc::new(move |payload| {
        api::topic::publish_topic(&state, "tasks", "runtime", payload.clone());
        api::topic::retain("tasks", "runtime", payload);
    });
    crate::runtime::set_progress_sink(progress.clone());
    tokio::spawn(crate::runtime::prewarm(pet_enabled, progress));
}

/// Pre-warm the voice providers off the hot path when the pet is enabled:
/// Kokoro's weights (~300 MB, the fallback voice) and the TTS sidecar if
/// the runtime prewarm above has already delivered its venv + model.
fn prewarm_voice(state: &Arc<ServerState>, pet_enabled: bool) {
    if pet_enabled && state.tts.prewarm_on_boot() {
        let tts = state.tts.clone();
        tokio::spawn(async move { tts.prewarm().await });
    }
}

/// Connect the MCP servers this engine is a client of, off the hot path.
/// Best effort by design (see mcp_client::registry): a third-party server
/// that hangs costs its own tools and a log line, never a turn — so this
/// is spawned rather than awaited, and startup never waits on it.
fn connect_mcp_servers(state: &Arc<ServerState>) {
    let manager = state.manager.clone();
    tokio::spawn(async move {
        // `[mcp_servers]` only. A repo's own .mcp.json is deliberately NOT
        // read: a stdio entry is `command` + `args`, so honouring a file
        // inside a cloned repo means launching whatever it names, and this
        // daemon resolves one workspace root at boot — there is nowhere
        // sound to put the approval prompt that would make it safe.
        //
        // Memory joins that list rather than sitting beside it: one
        // mechanism for every tool. Never empty as a result, so the
        // early return is gone.
        let cfg = manager.get_config_snapshot().await;
        let servers = crate::mcp_client::with_builtin(&cfg.mcp_servers, &cfg.agent.ling_mem_url);
        crate::mcp_client::registry().connect_all(&servers).await;
        watch_for_late_servers(servers).await;
    });
}

/// Keep asking the MCP servers that weren't listening at boot.
///
/// `connect_all` is best-effort by design, which is right for a server the
/// user configured and wrong for the one this engine depends on: the app
/// starts `ling-mem` beside this process, so a few seconds of start order
/// decided whether the daemon had memory at all for the rest of its life.
/// A refused connect costs a syscall, so the loop is cheap; it backs off to
/// minutes and stops as soon as everything enabled is connected.
async fn watch_for_late_servers(servers: std::collections::BTreeMap<String, McpServerConfig>) {
    const FIRST: std::time::Duration = std::time::Duration::from_secs(5);
    const CEILING: std::time::Duration = std::time::Duration::from_secs(300);

    let mut wait = FIRST;
    loop {
        tokio::time::sleep(wait).await;
        let registry = crate::mcp_client::registry();
        if registry.all_connected() {
            return;
        }
        let arrived = registry.retry_failed(&servers).await;
        if !arrived.is_empty() {
            info!(
                "MCP late arrival: {} — its tools are live for this daemon now",
                arrived.join(", ")
            );
        }
        wait = (wait * 2).min(CEILING);
    }
}

/// Idle-shutdown watcher: when --idle-shutdown-secs is set, exit the
/// process after that many seconds with zero connected WebRTC peers.
/// Used by bundled apps so the daemon doesn't outlive its last client.
fn idle_shutdown(state: &Arc<ServerState>, timeout: u64) {
    let peers = state.active_peer_count.clone();
    let last_activity = state.last_activity.clone();
    info!("idle-shutdown enabled: exit after {timeout}s with no peers and no activity");
    tokio::spawn(async move {
        let check_interval = std::time::Duration::from_secs(15);
        loop {
            tokio::time::sleep(check_interval).await;
            let now = super::unix_secs_now();
            // A connected peer is itself a sign of life — keep the activity
            // clock fresh so dropping a peer never trips an instant exit on
            // a stale timestamp.
            if peers.load(Ordering::Relaxed) > 0 {
                last_activity.store(now, Ordering::Relaxed);
                continue;
            }
            // No peers: exit only after `timeout` of no activity at all.
            // Bundled-app shells ping /api/health while their window is open
            // (see health_handler), so the daemon survives a window that is
            // open but transiently disconnected.
            if now.saturating_sub(last_activity.load(Ordering::Relaxed)) >= timeout {
                info!("idle-shutdown: no peers and no activity for {timeout}s, exiting");
                std::process::exit(0);
            }
        }
    });
}

/// Bridge internal AgentManager events to the UI (broadcast channel → WebRTC).
async fn bridge_agent_events(state: Arc<ServerState>, mut rx: AgentEvents) {
    while let Some((event, session_id)) = rx.recv().await {
        match event {
            // Special cases that need extra logic beyond a 1:1 mapping.
            AgentEvent::AgentStatus {
                agent_id,
                status,
                detail,
                parent_id,
                run_id,
                parent_run_id,
            } => {
                state
                    .send_agent_status_with_ids(
                        agent_id,
                        AgentStatusKind::from_str_loose(&status),
                        detail,
                        parent_id,
                        session_id,
                        run_id,
                        parent_run_id,
                    )
                    .await;
            }
            AgentEvent::TaskUpdate { .. } => {
                let _ = state.events_tx.send(ServerEvent::StateUpdated);
            }
            // A tool moved the session's working folder: `__cwd_changed__`
            // progress carries it (line = cwd, stream = "project|project_name").
            // Recorded on the session, surfaced as WorkingFolderChanged, and
            // never forwarded as ToolProgress.
            AgentEvent::ToolProgress {
                ref tool,
                ref line,
                ref stream,
                ..
            } if tool == "__cwd_changed__" => {
                if let Some(sid) = session_id {
                    note_working_folder(&state, sid, line.clone(), stream);
                }
            }
            // All other variants have a 1:1 ServerEvent equivalent.
            other => {
                count_tokens(&state, &other, &session_id).await;
                if let Some(se) = ServerEvent::from_agent_event(other, session_id) {
                    let _ = state.events_tx.send(se);
                }
            }
        }
    }
}

fn note_working_folder(state: &Arc<ServerState>, session_id: String, cwd: String, stream: &str) {
    let parts: Vec<&str> = stream.splitn(2, '|').collect();
    let part = |i: usize| {
        parts
            .get(i)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    let (project, project_name) = (part(0), part(1));
    let sessions = &state.manager.global_sessions;
    if let Ok(Some(mut meta)) = sessions.get_session_meta(&session_id) {
        meta.cwd = Some(cwd.clone());
        meta.project = project.clone();
        meta.project_name = project_name.clone();
        let _ = sessions.update_session_meta(&meta);
    }
    let _ = state.events_tx.send(ServerEvent::WorkingFolderChanged {
        session_id,
        cwd,
        project,
        project_name,
    });
}

/// Accumulate token usage from ContextUsage events.
async fn count_tokens(state: &Arc<ServerState>, event: &AgentEvent, session_id: &Option<String>) {
    let AgentEvent::ContextUsage {
        actual_prompt_tokens: Some(prompt),
        actual_completion_tokens: Some(completion),
        ..
    } = event
    else {
        return;
    };
    let sid = session_id.clone().unwrap_or_else(|| "current".to_string());
    let mut tokens = state.session_tokens.lock().await;
    let entry = tokens.entry(sid).or_insert((0, 0));
    entry.0 += prompt;
    entry.1 += completion;
}

/// Yinyue's event-reactive loops. The watch loop taps the event bus and, on
/// a coarse trigger (first slice: a non-Yinyue mission finishing), wakes the
/// Yinyue agent to decide whether to tell the user. Guards against self-loops
/// and ignores the per-token firehose. See server/yinyue_watch.rs.
fn spawn_companion(state: &Arc<ServerState>) {
    tokio::spawn(yinyue_watch::yinyue_watch_loop(state.clone()));
    // Ambient life-signs: on a jittered cadence she glances at the day and,
    // now and then, makes one small unprompted remark in her own voice
    // (mostly she stays quiet). Sibling to the watch loop, not a mission.
    tokio::spawn(yinyue_watch::yinyue_ambient_loop(state.clone()));
    // App moments: what an app posts to /api/yinyue/event waits until the
    // user has gone quiet, then she is woken once to judge a word. See
    // server/yinyue_moments.rs.
    tokio::spawn(yinyue_moments::yinyue_moment_loop(state.clone()));
}

/// Perception. Two loops: one tells the user's connected devices what is
/// true on this Mac (§6, and only when a device is there to hear it), the
/// other rotates the day — a trend sample, the day's activities handed to
/// the dream pass, then the sweep (§7).
fn spawn_perception(state: &Arc<ServerState>) {
    // First, account for the last run: a daemon that died holding a device
    // could not write that device's departure, and an unexplained gap in
    // the log is worse than the restart it hides.
    crate::perception::devices::note_restart();
    let topic_state = state.clone();
    tokio::spawn(crate::perception::publish::publish_loop(
        move |topic, op, payload| {
            api::topic::retain(topic, op, &payload);
            api::topic::publish_topic(&topic_state, topic, op, payload);
        },
    ));
    tokio::spawn(crate::perception::rotation::rotation_loop(
        state.manager.clone(),
    ));
}

/// The agent_run sweeper. Reaps `Running` rows older than the threshold
/// that never got `finish_agent_run` called (panic, dropped future, missing
/// finish on a new exit path). Without this, a stale row keeps the UI
/// spinner stuck on the affected session forever.
fn sweep_stale_runs(state: &Arc<ServerState>) {
    const SWEEP_INTERVAL_SECS: u64 = 60;
    const STALE_THRESHOLD_SECS: u64 = 15 * 60; // 15 min — well beyond any normal turn
    let sweep_state = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(SWEEP_INTERVAL_SECS));
        tick.tick().await; // skip the immediate first tick
        loop {
            tick.tick().await;
            let now = crate::util::now_ts_secs();
            let reaped = sweep_state
                .manager
                .run_store
                .sweep_stale_running(now, STALE_THRESHOLD_SECS);
            if !reaped.is_empty() {
                tracing::warn!(
                    count = reaped.len(),
                    run_ids = ?reaped,
                    threshold_secs = STALE_THRESHOLD_SECS,
                    "run/sweep: reaped stale Running rows"
                );
                let _ = sweep_state.events_tx.send(ServerEvent::StateUpdated);
            }
        }
    });
}

/// Auto-connect to joined proxy rooms (linggen server consumer mode)
/// if auto_connect is enabled in room_config.toml.
fn auto_connect_rooms(state: &Arc<ServerState>) {
    let room_cfg = rtc::room_config::load_room_config();
    if !room_cfg.auto_connect {
        return;
    }
    let auto_state = state.clone();
    tokio::spawn(async move {
        // Small delay to let relay establish first.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        rtc::proxy_room::auto_connect_joined_rooms(auto_state).await;
    });
}
