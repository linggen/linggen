//! Event fanout from `events_tx` to the right data channels.
//!
//! Extracted from peer.rs to keep the main event loop focused on str0m
//! orchestration. The logic here is pure: in = a ServerEvent + channel/state
//! references, out = mutations to `pending_dc_writes` / `pending_events` /
//! `dirty_flags` and (rarely) `filter.session_ids`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::server::ServerState;

use super::MAX_DC_WRITE_QUEUE;

/// Max buffered events per session before the channel opens.
pub(super) const MAX_PENDING_EVENTS: usize = 2000;

/// Event filter context — passed to forward_event_to_channels for all peers.
/// Admin peers skip session filtering; non-admin peers only see their own sessions.
/// The `view` and `pinned_session_id` fields further scope broadcasts for embed
/// peers so a skill iframe can't observe activity from the user's other sessions.
pub(super) struct EventFilter<'a> {
    pub(super) session_ids: &'a mut HashSet<String>,
    pub(super) user_id: &'a str,
    pub(super) is_admin: bool,
    /// Which UI entry is connected: "main" | "embed" | "consumer" | None (pre-PR2).
    pub(super) view: Option<&'a str>,
    /// For embed: the pinned session id. Broadcasts from other sessions are dropped.
    pub(super) pinned_session_id: Option<&'a str>,
    /// This peer's id in the Yinyue presenter registry — used to decide whether
    /// THIS surface is the singleton holder (gets her voice/expression) or not.
    pub(super) peer_id: u64,
}

pub(super) fn forward_event_to_channels(
    event: &crate::server::ServerEvent,
    session_channels: &HashMap<String, str0m::channel::ChannelId>,
    control_channel_id: Option<str0m::channel::ChannelId>,
    inference_channel_id: Option<str0m::channel::ChannelId>,
    pending_events: &mut HashMap<String, (Instant, Vec<String>)>,
    pending_dc_writes: &mut VecDeque<(str0m::channel::ChannelId, String)>,
    state: &Arc<ServerState>,
    dirty_flags: &mut u64,
    filter: &mut EventFilter<'_>,
) {
    // StateUpdated → set dirty flag for page state push, don't forward to DC
    if matches!(event, crate::server::ServerEvent::StateUpdated) {
        *dirty_flags |= crate::server::rtc::page_state::DIRTY_ALL;
        return;
    }

    // Yinyue presenter changed → tell THIS peer whether it now holds the lock.
    // The flag is per-peer (it compares the registry head to our own peer id),
    // so it can't go through the shared event→ui mapping — write the present
    // signal straight to the control channel.
    if matches!(event, crate::server::ServerEvent::YinyuePresenterChanged) {
        if let Some(cid) = control_channel_id {
            let present = state.yinyue_holder() == Some(filter.peer_id);
            let seq = state
                .event_seq
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let msg = serde_json::json!({
                "seq": seq,
                "kind": "yinyue_present",
                "session_id": serde_json::Value::Null,
                "data": { "present": present },
            });
            if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                pending_dc_writes.push_back((cid, msg.to_string()));
            }
        }
        return;
    }

    // Prune expired pending event buffers (older than 60s)
    let now = Instant::now();
    pending_events.retain(|_, (created, _)| now.duration_since(*created) < Duration::from_secs(60));
    // Map the server event to a UI message (same as SSE handler)
    let seq = state
        .event_seq
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ui_msg = match crate::server::map_server_event_to_ui_message(event.clone(), seq) {
        Some(msg) => msg,
        None => return,
    };

    // Yinyue singleton: her voice (`pet_speak`) and expression (`pet_express`)
    // go ONLY to the presenter — the FCFS lock holder — so that with multiple
    // surfaces open (tabs, apps, the desktop pet) she renders and speaks in just
    // one place. Applies to every peer, admin included (the gate sits above the
    // user-filter, which otherwise whitelists these global kinds to all peers).
    if matches!(ui_msg.kind.as_str(), "pet_speak" | "pet_express")
        && state.yinyue_holder() != Some(filter.peer_id)
    {
        return;
    }

    let dbg_sid = ui_msg.session_id.clone().unwrap_or_else(|| "-".to_string());
    let dbg_kind = ui_msg.kind.clone();
    let dbg_view = filter.view.unwrap_or("-");
    let dbg_pinned = filter.pinned_session_id.unwrap_or("-");
    let dbg_user = filter.user_id;

    // Session isolation: non-admin peers only see their own sessions.
    // Admin peers skip filtering entirely.
    if !filter.is_admin {
        match ui_msg.session_id.as_deref() {
            Some("global") | None => {
                // Room chat and Yinyue's voice/expression must reach all peers
                // regardless of permission level — user-level global signals.
                if ui_msg.kind != "room_chat"
                    && ui_msg.kind != "pet_speak"
                    && ui_msg.kind != "pet_express"
                {
                    tracing::debug!(
                        "[fwd] DROP(user-filter global) user={} view={} sid={} kind={}",
                        dbg_user,
                        dbg_view,
                        dbg_sid,
                        dbg_kind
                    );
                    return;
                }
            }
            Some(sid) => {
                // For SessionCreated events, check if the new session belongs to us
                if ui_msg.kind == "session_created" {
                    if let Ok(Some(meta)) = state.manager.global_sessions.get_session_meta(sid) {
                        if meta.user_id.as_deref() == Some(filter.user_id) {
                            filter.session_ids.insert(sid.to_string());
                        }
                    }
                }
                if !filter.session_ids.contains(sid) {
                    tracing::debug!(
                        "[fwd] DROP(sid-not-in-filter) user={} view={} sid={} kind={} known={}",
                        dbg_user,
                        dbg_view,
                        dbg_sid,
                        dbg_kind,
                        filter.session_ids.len()
                    );
                    return;
                }
            }
        }
    }

    // Embed peers are pinned to a single session. They receive that session's
    // events via its sess-{id} channel and never need activity from other
    // sessions — suppress cross-session events entirely (skill iframe must not
    // observe activity, ask_user, or session_created from unrelated sessions).
    //
    // Room chat used to be allowed through to embed peers as "user-level",
    // but embed iframes (skill apps) don't render a RoomChatPanel — and when
    // the user has both a main view AND an embed view connected at the same
    // time (typical: a skill page is open in the dashboard), the embed peer's
    // forward duplicated every room_chat message in the visible panel. Drop
    // it for embed peers; the main-view peer is the only one that displays
    // room chat.
    if filter.view == Some("embed") && ui_msg.kind == "room_chat" {
        tracing::trace!(
            "[fwd] DROP(embed-no-room-chat) user={} pinned={}",
            dbg_user, dbg_pinned
        );
        return;
    }
    if let (Some("embed"), Some(pinned)) = (filter.view, filter.pinned_session_id) {
        if let Some(sid) = ui_msg.session_id.as_deref() {
            let is_user_level = sid == "global";
            if !is_user_level && sid != pinned {
                // TRACE not DEBUG — this branch fires per-event for every
                // background session the embed peer ignores. A single
                // memory-skill extraction run spawns ~10 subagents each
                // emitting dozens of token/activity/run frames; at DEBUG
                // that produces thousands of identical "DROP" lines per
                // minute. The drop itself is correct behavior; we just
                // don't need to scream about each one.
                tracing::trace!(
                    "[fwd] DROP(embed-pin-mismatch) user={} pinned={} sid={} kind={}",
                    dbg_user,
                    dbg_pinned,
                    dbg_sid,
                    dbg_kind
                );
                return;
            }
        }
    }

    let json = match serde_json::to_string(&ui_msg) {
        Ok(j) => j,
        Err(_) => return,
    };

    // Route to session channel if available, buffer if channel not yet open.
    // All writes go through the pending_dc_writes queue — writing directly to
    // str0m channels without a poll_output() in between causes SCTP corruption.
    //
    // Broadcast events (notifications, agent_status, activity, run) fall back
    // to the control channel when the peer has NO session channel for the
    // event's session — so the main page can still react (e.g. session list
    // updates when a skill or mission creates a new session). If the session
    // channel IS open, we must not also push to control, or the peer sees the
    // same event twice and UI handlers that append (subagent tree, activity
    // lines) produce duplicates.
    let is_broadcast = matches!(
        ui_msg.kind.as_str(),
        "agent_status"
            | "activity"
            | "run"
            | "notification"
            | "ask_user"
            | "widget_resolved"
            | "room_chat"
    );
    match ui_msg.session_id.as_deref() {
        Some("global") | None => {
            if let Some(cid) = control_channel_id {
                if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                    if ui_msg.kind == "room_chat" {
                        tracing::info!(
                            "[room_chat] forward → control DC user={} channel={:?}",
                            dbg_user, cid
                        );
                    }
                    pending_dc_writes.push_back((cid, json.clone()));
                }
            }
        }
        Some(sid) => {
            // Send to the session's dedicated channel
            if let Some(&cid) = session_channels.get(sid) {
                if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                    pending_dc_writes.push_back((cid, json.clone()));
                }
            } else if is_broadcast {
                // No session channel — send broadcast events to control so the
                // main page still reacts (session list updates etc).
                if let Some(cid) = control_channel_id {
                    if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                        pending_dc_writes.push_back((cid, json.clone()));
                    }
                } else {
                    tracing::debug!(
                        "[fwd] DROP(broadcast-no-channel) user={} view={} sid={} kind={} known_chans={}",
                        dbg_user, dbg_view, dbg_sid, dbg_kind, session_channels.len()
                    );
                }
            } else if is_ephemeral_stream(&dbg_kind) {
                // Streaming frames (token / text_segment / tool_progress /
                // content_block mid-stream deltas) are high-frequency and
                // stale within seconds. Buffering them just fills the per-
                // session ring and previously drowned the log with
                // BUFFER(channel-not-open) noise — by the time a viewer
                // opens this session's data channel the bash output or
                // token deltas are long since done. Drop silently; the
                // final state still arrives via the stable events
                // (message, turn_complete, content_block update=done).
            } else {
                // Non-broadcast event, session channel not yet open — buffer
                // until the channel opens and flush on open.
                let (_, buf) = pending_events
                    .entry(sid.to_string())
                    .or_insert_with(|| (Instant::now(), Vec::new()));
                if buf.len() < MAX_PENDING_EVENTS {
                    let was_empty = buf.is_empty();
                    buf.push(json.clone());
                    // Log once per (session × buffer-opening), not per
                    // event. TRACE on subsequent pushes so the flood from
                    // any future high-frequency non-ephemeral kind stays
                    // off DEBUG unless explicitly asked for.
                    if was_empty {
                        tracing::debug!(
                            "[fwd] BUFFER(channel-not-open) user={} view={} sid={} kind={} known_chans={}",
                            dbg_user, dbg_view, dbg_sid, dbg_kind, session_channels.len()
                        );
                    } else {
                        tracing::trace!(
                            "[fwd] BUFFER+ user={} sid={} kind={} depth={}",
                            dbg_user, dbg_sid, dbg_kind, buf.len()
                        );
                    }
                }
            }
        }
    }

    // Forward room_chat to inference channel — only on the OWNER side
    // (owner → consumer direction). Consumer → owner is handled by proxy_client loop.
    // We detect the owner side by checking: inference_channel_id is set (proxy peer)
    // AND control_channel_id is NOT set (proxy peers don't have a control channel).
    if ui_msg.kind == "room_chat" && inference_channel_id.is_some() && control_channel_id.is_none()
    {
        if let Some(cid) = inference_channel_id {
            let chat_msg = serde_json::json!({
                "type": "room_chat",
                "data": ui_msg.data,
            });
            if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                tracing::info!(
                    "[room_chat] forward → inference DC user={} channel={:?}",
                    dbg_user, cid
                );
                pending_dc_writes.push_back((cid, chat_msg.to_string()));
            }
        }
    }
}

/// True when an event kind is a high-frequency streaming frame whose value
/// expires within seconds (token deltas, bash stdout chunks, content-block
/// mid-stream progress). These are meaningful only to a viewer watching in
/// real time. Buffering them for a viewer who hasn't opened the session
/// channel yet is pointless: by the time the channel opens the streaming
/// turn is over, and replaying thousands of stale deltas just lags the UI
/// and bloats memory. The terminal/stable events on the same session
/// (`message`, `turn_complete`, `content_block` with phase=`update`)
/// carry the final state and ARE buffered normally.
fn is_ephemeral_stream(kind: &str) -> bool {
    matches!(kind, "token" | "text_segment" | "tool_progress")
}
