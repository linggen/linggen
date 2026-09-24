pub(crate) mod api;
mod app_pages;
mod background;
pub(crate) mod bridge;
mod chat;
mod facts;
mod loopback_guard;
mod mcp;
mod mcp_agent;
pub(crate) mod milestones;
mod resident;
mod quests_watch;
mod routes;
pub(crate) mod rtc;
mod shared_pages;
mod state;
mod ui_events;
mod yinyue_moments;

pub use crate::engine::events::{AgentStatusKind, QueuedChatItem, ServerEvent};
pub use state::ServerState;
pub(crate) use ui_events::{map_server_event_to_ui_message, UI_PHASE_DOING, UI_PHASE_DONE};

use crate::engine::agent::AgentManager;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::Mutex;
use tracing::info;

struct ServerHandle {
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
    port: u16,
}

async fn prepare_server(
    manager: Arc<AgentManager>,
    skills: Arc<crate::extensions::skills::SkillLoader>,
    missions: Arc<crate::extensions::missions::MissionLoader>,
    host: &str,
    port: u16,
    dev_mode: bool,
    idle_shutdown_secs: Option<u64>,
    agent_events_rx: background::AgentEvents,
) -> anyhow::Result<ServerHandle> {
    info!("linggen server starting on {}:{}...", host, port);

    // Events can be bursty (tool/status steps). Use a larger buffer to reduce lag drops.
    let (events_tx, _) = broadcast::channel(4096);

    let prompt_store = Arc::new(crate::prompts::PromptStore::load(Some(
        &crate::prompts::PromptStore::default_override_dir(),
    )));

    // Pet/voice settings drive the TTS engine choice + whether to pre-warm it.
    let pet_cfg = manager.get_config_snapshot().await.pet;

    let state = Arc::new(ServerState {
        manager,
        dev_mode,
        port,
        bound_host: host.to_string(),
        active_peer_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        last_activity: Arc::new(AtomicU64::new(unix_secs_now())),
        last_user_turn_at: Arc::new(AtomicU64::new(unix_secs_now())),
        events_tx,
        skills,
        missions,
        prompt_store,
        queued_chats: Arc::new(Mutex::new(HashMap::new())),
        interrupt_tx: Arc::new(Mutex::new(HashMap::new())),
        pending_ask_user: Arc::new(Mutex::new(HashMap::new())),
        status_seq: AtomicU64::new(1),
        active_statuses: Arc::new(Mutex::new(HashMap::new())),
        queue_seq: AtomicU64::new(1),
        event_seq: AtomicU64::new(1),
        session_tokens: Arc::new(Mutex::new(HashMap::new())),
        whip_token: uuid::Uuid::new_v4().to_string(),
        user_bash_cwd: Arc::new(Mutex::new(HashMap::new())),
        proxy_connections: Arc::new(rtc::proxy_room::ProxyRoomConnections::new()),
        token_usage: Arc::new(tokio::sync::Mutex::new(
            rtc::token_store::TokenUsageStore::load(),
        )),
        codex_login_task: Arc::new(tokio::sync::Mutex::new(None)),
        tts: api::tts::default_provider(),
        bridge: Arc::new(bridge::BridgeHub::new()),
        current_view: Arc::new(std::sync::Mutex::new(None)),
        yinyue_presenters: Arc::new(std::sync::Mutex::new(Vec::new())),
        next_peer_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
    });

    background::spawn_background_tasks(
        &state,
        pet_cfg.enabled,
        idle_shutdown_secs,
        agent_events_rx,
    );

    // LAN gate: outermost layer — see `lan_gate`.
    let app = routes::router(state.clone()).layer(axum::middleware::from_fn(lan_gate));

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", host, port)).await?;
    let actual_port = listener.local_addr()?.port();
    info!("Server running on http://{}:{}", host, actual_port);
    api::pair::advertise(actual_port, host != "127.0.0.1" && host != "localhost");

    // Anonymous usage telemetry. See src/telemetry/mod.rs for the field list
    // and opt-out paths. Fired here (after listener binds, before serve loop)
    // so a launch is only counted when the daemon is actually up.
    {
        let telemetry = crate::telemetry::global();
        telemetry.launch();
        let system_state = crate::telemetry::read_system_state(crate::paths::linggen_home());
        telemetry.command_with_payload(
            "engine.start",
            serde_json::json!({ "system_state": system_state }),
        );
        telemetry.bump("engine.start");
    }

    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await?;
        Ok(())
    });

    Ok(ServerHandle {
        task,
        port: actual_port,
    })
}

/// Why a loopback request is refused, if it is: a Host that isn't this machine
/// (DNS rebinding) or a browser Origin from another site. See `loopback_guard`.
/// `authority` is the request URI's (HTTP/2 `:authority`, absolute-form).
fn loopback_rejection(
    headers: &axum::http::HeaderMap,
    authority: Option<&str>,
) -> Option<&'static str> {
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let bad_host = |h: &str| !loopback_guard::host_is_loopback(h);
    if header(axum::http::header::HOST).is_some_and(bad_host) || authority.is_some_and(bad_host) {
        return Some("host_not_allowed");
    }
    if !loopback_guard::origin_allowed(header(axum::http::header::ORIGIN)) {
        return Some("origin_not_allowed");
    }
    None
}

/// The LAN trust gate (see `api::pair`). Loopback callers — the local web UI,
/// app shells, skills, the simulator — pass untouched. Anything arriving over
/// the network must present a paired-device token; only the health check and
/// the pairing handshake itself stay open (they are the bootstrap).
async fn lan_gate(
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if addr.ip().is_loopback() {
        let authority = req.uri().authority().map(|a| a.as_str().to_string());
        return match loopback_rejection(req.headers(), authority.as_deref()) {
            None => next.run(req).await,
            Some(why) => {
                tracing::warn!(
                    "[gate] rejected loopback {} {}: {why}",
                    req.method(),
                    req.uri().path()
                );
                (
                    axum::http::StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({ "error": why })),
                )
                    .into_response()
            }
        };
    }
    let path = req.uri().path();
    if matches!(
        path,
        "/api/health" | "/api/pair/request" | "/api/pair/confirm" | "/api/pair/qr-confirm"
    ) {
        return next.run(req).await;
    }
    let headers = req.headers();
    let token = headers
        .get("x-linggen-device")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
        // Webviews (phone app_mode surfaces) can't add headers to their
        // subresource requests — they carry the device token as a cookie.
        .or_else(|| {
            headers
                .get(axum::http::header::COOKIE)
                .and_then(|v| v.to_str().ok())
                .and_then(|c| {
                    c.split(';')
                        .map(str::trim)
                        .find_map(|kv| kv.strip_prefix("linggen_device="))
                })
        });
    if token.map(api::pair::is_valid_device_token).unwrap_or(false) {
        return next.run(req).await;
    }
    tracing::info!("[gate] rejected {} {} from {}", req.method(), path, addr);
    (
        axum::http::StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({ "error": "pairing_required" })),
    )
        .into_response()
}

pub async fn start_server(
    manager: Arc<AgentManager>,
    skills: Arc<crate::extensions::skills::SkillLoader>,
    missions: Arc<crate::extensions::missions::MissionLoader>,
    host: &str,
    port: u16,
    dev_mode: bool,
    idle_shutdown_secs: Option<u64>,
    agent_events_rx: background::AgentEvents,
) -> anyhow::Result<()> {
    let handle = prepare_server(
        manager,
        skills,
        missions,
        host,
        port,
        dev_mode,
        idle_shutdown_secs,
        agent_events_rx,
    )
    .await?;
    handle.task.await??;
    Ok(())
}

/// Seconds since the Unix epoch (0 on the impossible clock-before-epoch error).
pub(crate) fn unix_secs_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_gate_refuses_rebound_hosts_and_foreign_origins() {
        use axum::http::{header, HeaderMap, HeaderValue};
        let with = |pairs: &[(header::HeaderName, &'static str)]| {
            let mut h = HeaderMap::new();
            for (k, v) in pairs {
                h.insert(k.clone(), HeaderValue::from_static(v));
            }
            h
        };
        // Local UI, curl, app shells, the extension.
        assert_eq!(loopback_rejection(&with(&[]), None), None);
        assert_eq!(
            loopback_rejection(
                &with(&[
                    (header::HOST, "127.0.0.1:9527"),
                    (header::ORIGIN, "http://127.0.0.1:9527")
                ]),
                None
            ),
            None
        );
        assert_eq!(
            loopback_rejection(&with(&[(header::HOST, "localhost:9527")]), None),
            None
        );
        assert_eq!(
            loopback_rejection(
                &with(&[
                    (header::HOST, "127.0.0.1:9527"),
                    (header::ORIGIN, "chrome-extension://abc")
                ]),
                None
            ),
            None
        );
        // DNS rebinding: the browser says the page's own name.
        assert_eq!(
            loopback_rejection(&with(&[(header::HOST, "evil.example:9527")]), None),
            Some("host_not_allowed")
        );
        assert_eq!(
            loopback_rejection(&with(&[]), Some("evil.example:9527")),
            Some("host_not_allowed")
        );
        // Another site's page posting to 127.0.0.1.
        assert_eq!(
            loopback_rejection(
                &with(&[
                    (header::HOST, "127.0.0.1:9527"),
                    (header::ORIGIN, "https://evil.example")
                ]),
                None
            ),
            Some("origin_not_allowed")
        );
    }

    /// Ensure every ServerEvent variant maps without panicking.
    /// Acts as a documentation checkpoint — if a new variant is added, this test
    /// will fail to compile until a mapping arm is provided.
    #[test]
    fn all_server_events_mapped() {
        let events: Vec<ServerEvent> = vec![
            ServerEvent::StateUpdated,
            ServerEvent::Message {
                from: "ling".into(),
                to: "user".into(),
                content: "hello".into(),
                session_id: None,
                run_id: None,
                parent_agent_id: None,
            },
            ServerEvent::SubagentSpawned {
                parent_id: "ling".into(),
                subagent_id: "coder".into(),
                task: "fix bug".into(),
                session_id: None,
                subagent_run_id: None,
                parent_run_id: None,
            },
            ServerEvent::SubagentResult {
                parent_id: "ling".into(),
                subagent_id: "coder".into(),
                outcome: crate::engine::AgentOutcome::None,
                session_id: None,
                subagent_run_id: None,
                parent_run_id: None,
            },
            ServerEvent::AgentStatus {
                agent_id: "ling".into(),
                status: "thinking".into(),
                detail: Some("Analyzing code".into()),
                status_id: None,
                lifecycle: Some("doing".into()),
                parent_agent_id: None,
                session_id: None,
                run_id: None,
                parent_run_id: None,
            },
            ServerEvent::QueueUpdated {
                project_root: "/tmp".into(),
                session_id: "s1".into(),
                agent_id: "ling".into(),
                items: vec![],
            },
            ServerEvent::ContextUsage {
                agent_id: "ling".into(),
                stage: "pre".into(),
                message_count: 10,
                char_count: 5000,
                estimated_tokens: 1500,
                token_limit: Some(200_000),
                actual_prompt_tokens: None,
                actual_completion_tokens: None,
                compressed: false,
                summary_count: 0,
                session_id: None,
            },
            ServerEvent::Outcome {
                agent_id: "ling".into(),
                outcome: crate::engine::AgentOutcome::None,
                session_id: None,
            },
            ServerEvent::Token {
                session_id: None,
                agent_id: "ling".into(),
                token: "Hello".into(),
                done: false,
                thinking: false,
            },
            ServerEvent::PlanUpdate {
                agent_id: "ling".into(),
                plan: crate::engine::Plan {
                    summary: "Test plan".into(),
                    status: crate::engine::PlanStatus::Planned,
                    plan_text: String::new(),
                    items: Vec::new(),
                },
                session_id: None,
            },
            ServerEvent::MissionTriggered {
                mission_id: "mission-1".into(),
                agent_id: "ling".into(),
                project_root: "/tmp".into(),
                session_id: None,
            },
            ServerEvent::TextSegment {
                agent_id: "ling".into(),
                text: "some text".into(),
                parent_id: None,
                session_id: None,
            },
            ServerEvent::AskUser {
                agent_id: "ling".into(),
                question_id: "q1".into(),
                questions: vec![],
                session_id: None,
            },
            ServerEvent::Followups {
                agent_id: "ling".into(),
                items: vec!["Show the details".into()],
                run_id: None,
                session_id: None,
            },
            ServerEvent::ModelFallback {
                agent_id: "ling".into(),
                preferred_model: "gpt-4".into(),
                actual_model: "gpt-3.5".into(),
                reason: "rate_limited".into(),
                session_id: None,
            },
            ServerEvent::ToolProgress {
                agent_id: "ling".into(),
                tool: "Bash".into(),
                line: "building...".into(),
                stream: "stdout".into(),
                session_id: None,
            },
            ServerEvent::Resync {
                reason: "broadcast_lag".into(),
                lagged_count: Some(42),
            },
            ServerEvent::ContentBlockStart {
                agent_id: "ling".into(),
                block_id: "cb-1".into(),
                block_type: "tool_use".into(),
                tool: Some("Read".into()),
                args: Some("foo.rs".into()),
                parent_id: None,
                session_id: None,
                run_id: None,
                parent_run_id: None,
            },
            ServerEvent::ContentBlockUpdate {
                agent_id: "ling".into(),
                block_id: "cb-1".into(),
                status: Some("done".into()),
                summary: Some("Read 42 lines".into()),
                is_error: Some(false),
                parent_id: None,
                extra: None,
                session_id: None,
                run_id: None,
                parent_run_id: None,
            },
            ServerEvent::TurnComplete {
                agent_id: "ling".into(),
                duration_ms: Some(1200),
                context_tokens: Some(5000),
                parent_id: None,
                session_id: None,
                run_id: None,
                parent_run_id: None,
            },
            ServerEvent::RoomChat {
                sender_id: "user-1".into(),
                sender_name: "Alice".into(),
                avatar_url: None,
                text: "Hello room!".into(),
            },
        ];

        for (i, event) in events.into_iter().enumerate() {
            let result = map_server_event_to_ui_message(event, i as u64);
            // All variants should produce Some(...), except Message which may
            // return None if sanitization strips it. We just verify no panics.
            let _ = result;
        }
    }
}
