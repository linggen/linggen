//! Server-pushed page state — aggregates all UI-relevant data into one message.
//!
//! Instead of the frontend making 10+ individual HTTP requests on every state
//! change, the server pushes a single `PageState` message over the WebRTC
//! control data channel at 0.5 Hz (every 2 seconds), only when state changed.

use crate::server::ServerState;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

/// The frontend's current view context — what session/project is active.
/// Sent by the frontend via `set_view_context` control channel message.
#[derive(Debug, Clone, Default)]
pub struct ViewContext {
    pub session_id: Option<String>,
    pub project_root: Option<String>,
    /// Which UI entry is connected: "main" | "embed" | "consumer" |
    /// "channel". None means the client hasn't reported yet — treat as "main".
    pub view: Option<String>,
    /// The app whose page this is, when it is one (an app's chat embed): a
    /// Clear pressed there takes that app's part of the day.
    pub skill: Option<String>,
}

impl ViewContext {
    /// The view `person` is shown: a day only its own person's
    /// (`day_session::may_enter`, as the live channel streams it); another
    /// person's day is an empty view — no session, nothing pinned to it.
    /// The bool says the view was emptied.
    fn shown_to(&self, person: &crate::server::person::Person) -> (ViewContext, bool) {
        let refused = self
            .session_id
            .as_deref()
            .is_some_and(|sid| !crate::server::day_session::may_enter(sid, person));
        let mut shown = self.clone();
        if refused {
            shown.session_id = None;
        }
        (shown, refused)
    }

    /// A `channel` peer is a skill page's data path (shared/channel.js): it
    /// only asks, and renders nothing — no page state, no events.
    pub fn wants_pushes(&self) -> bool {
        self.view.as_deref() != Some("channel")
    }
}

/// Aggregated page state pushed to the frontend.
/// All fields are optional — the server omits fields that haven't changed
/// or aren't applicable to the current view context.
#[derive(Debug, Clone, Serialize)]
pub struct PageState {
    // -- User context (always included) --
    /// User type: "owner" or "consumer". Set at connection time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_type: Option<String>,

    /// User's permission level: "admin", "edit", "read", "chat"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,

    /// Room name (only for room users)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_name: Option<String>,

    /// Whether rooms are on at all (the engine's switch). A page shows
    /// nothing about rooms unless this is true. Not sent to a consumer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rooms_on: Option<bool>,

    /// Whether the owner's room is enabled (accepting consumers); only
    /// told while rooms are on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_enabled: Option<bool>,

    // -- Global (always included unless compact mode) --
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_sessions: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_models: Option<Vec<String>>,

    /// The Claude sessions' default; empty when none is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_default_model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub missions: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_ask_user: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_counts_by_project: Option<std::collections::HashMap<String, usize>>,

    /// Map of session_id → agent status string for all currently-busy sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy_sessions: Option<std::collections::HashMap<String, String>>,

    /// The web UI build the engine serves now (`app_pages::ui_build`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_build: Option<String>,

    /// The agents that appear only once met, and whether each has been
    /// (`engine::agent::met`): `{id, present, met: {at, reason} | null}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents_met: Option<Vec<crate::engine::agent::met::AgentMet>>,

    // -- Scoped (based on ViewContext, Admin only) --
    /// The session the scoped fields below are for. During a switch a push
    /// may still describe the previous one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// That session's runtime; absent when the session is not known yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_runtime: Option<crate::state_fs::sessions::SessionRuntime>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_runs: Option<Vec<serde_json::Value>>,

    /// Messages waiting behind a busy agent in the scoped session, as
    /// `{agent_id, items}` — one entry per agent with a non-empty queue, so an
    /// ABSENT agent means "nothing queued".
    ///
    /// The queue also arrives live as a `queue` event, but an event only helps
    /// a client that was listening. There is no endpoint to re-read it, so
    /// before this a client that missed the event — its channel closed, or it
    /// reconnected as a fresh peer — kept showing a queue that no longer
    /// existed and refused every new message as "agent is busy". A snapshot
    /// cannot diverge that way: it is the whole truth, pushed on every
    /// (re)connect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queued: Option<Vec<serde_json::Value>>,

    /// The session's background tasks (doc/background-tasks-spec.md).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<crate::engine::events::BackgroundTask>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_permission: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<serde_json::Value>>,
}

/// Queue entries for one session as `{agent_id, items}`.
///
/// Keys are `project_root|session_id|agent_id`, so this matches on the session
/// segment rather than splitting on '|' — a project root is a path and may
/// contain one. Agents with an empty queue are omitted: absent means nothing
/// queued, which is what lets the snapshot express a drained queue.
/// One run as the page reads it. In a day the session's running top-level
/// run names the place its turn stands in (`told`, from
/// `turn_place::told_running`), so a page at another app can say so beside
/// its Stop; its subagents and every other run name none.
fn run_row(run: crate::engine::agent::AgentRunRecord, told: Option<&str>) -> Option<serde_json::Value> {
    let top = run.parent_run_id.is_none() && run.status == crate::engine::agent::AgentRunStatus::Running;
    let mut row = serde_json::to_value(run).ok()?;
    if let Some(place) = told.filter(|_| top) {
        row["place"] = serde_json::Value::from(place);
    }
    Some(row)
}

fn queued_entries(
    all: &std::collections::HashMap<String, Vec<crate::engine::events::QueuedChatItem>>,
    session_id: &str,
) -> Vec<serde_json::Value> {
    all.values()
        .filter_map(|items| {
            let first = items.first().filter(|item| item.session_id == session_id)?;
            Some(serde_json::json!({
                "agent_id": first.agent_id,
                "items": items,
            }))
        })
        .collect()
}

/// One row of the session list. `updated_at` is #[serde(skip)] so it never
/// lands in session.yaml — the derived value is put in by hand for the UI
/// (grouping/sort key on last activity). The day's session is marked
/// `day: true` (`doc/one-ling-spec.md` § Linggen's daily session), and a
/// session of a kind that may carry a work tag `takes_work: true` — the
/// rule is the spec's (`Preset::takes_work`), told to the page so it does
/// not keep a copy.
fn session_row(s: crate::state_fs::sessions::SessionMeta) -> Option<serde_json::Value> {
    let updated_at = s.updated_at;
    let day = crate::server::day_session::is_day(&s.id);
    let takes_work = crate::engine::session_spec::preset_of(&s).takes_work();
    let mut v = serde_json::to_value(s).ok()?;
    v["updated_at"] = serde_json::Value::from(updated_at);
    if day {
        v["day"] = serde_json::Value::Bool(true);
    }
    if takes_work {
        v["takes_work"] = serde_json::Value::Bool(true);
    }
    Some(v)
}

/// Dirty-flag bits for tracking which data categories changed.
pub const DIRTY_GLOBAL: u64 = 0b0000_0001;
pub const DIRTY_SCOPED: u64 = 0b0000_0010;
pub const DIRTY_ALL: u64 = DIRTY_GLOBAL | DIRTY_SCOPED;

/// Whether a session made for `made_for` is listed to the viewer whose id is
/// `viewer`: one made for them; one made for no one (kept before sessions
/// had a person) to the owner at admin (`is_admin`).
fn listed_for(made_for: Option<&str>, viewer: &str, is_admin: bool) -> bool {
    match made_for {
        Some(id) => id == viewer,
        None => is_admin,
    }
}

/// What the page is told of rooms: `(rooms_on, room_enabled)`. A consumer is
/// told neither; the owner always learns whether rooms are on, and whether
/// its own room is enabled only while they are.
fn room_facts(rooms_on: bool, is_consumer: bool, enabled: bool) -> (Option<bool>, Option<bool>) {
    if is_consumer {
        return (None, None);
    }
    (Some(rooms_on), rooms_on.then_some(enabled))
}

/// Build the page state from server state + view context + user context.
/// All data is filtered by user_id. Permission level controls what data categories are included.
/// `person`: the person behind the peer (`peer::control::person_for`) — a
/// day is shown only to its own person. `whose`: whose sessions a peer below
/// the owner's standing is shown (`peer::standing::seen_as`); `None` for the
/// owner, who is shown all.
pub async fn build_page_state(
    state: &Arc<ServerState>,
    ctx: &ViewContext,
    dirty: u64,
    (user, identified): (&super::UserContext, bool),
    (whose, person): (Option<&str>, &crate::server::person::Person),
) -> PageState {
    let (shown, refused) = ctx.shown_to(person);
    let ctx = &shown;
    let theirs = |sid: &str| crate::server::day_session::may_enter(sid, person);
    let include_global = (dirty & DIRTY_GLOBAL) != 0;
    let include_scoped = (dirty & DIRTY_SCOPED) != 0;
    let is_admin = whose.is_none() && user.permission == crate::engine::permission::PermissionMode::Admin;
    let user_id = &whose.unwrap_or(&user.user_id).to_string();

    // Load room config — needed for consumer filtering and owner room_enabled status.
    let room_cfg = super::room_config::load_room_config();
    let (rooms_on, room_enabled) = room_facts(
        crate::server::rooms_switch::on(),
        user.is_consumer(),
        room_cfg.room_enabled,
    );

    let mut ps = PageState {
        // Corrected by what `identify` revealed, like every other surface that
        // names this peer. Reading the raw kind here made page state the one
        // place a paired phone still answered to the owner's name.
        user_type: Some(user.user_type_for(identified).to_string()),
        permission: Some(user.permission.to_string()),
        room_name: user.room_name.clone(),
        rooms_on,
        room_enabled,
        all_sessions: None,
        models: None,
        default_models: None,
        claude_default_model: None,
        skills: None,
        missions: None,
        pending_ask_user: None,
        session_counts_by_project: None,
        busy_sessions: None,
        agents_met: None,
        ui_build: None,
        session_id: None,
        session_runtime: None,
        agents: None,
        agent_runs: None,
        queued: None,
        tasks: None,
        sessions: None,
        session_permission: None,
        files: None,
    };

    // Collect user's session IDs for filtering busy_sessions and pending_ask_user
    let user_session_ids: std::collections::HashSet<String> =
        if let Ok(sessions) = state.manager.global_sessions.list_sessions() {
            sessions
                .iter()
                .filter(|s| listed_for(s.user_id.as_deref(), user_id, is_admin))
                .map(|s| s.id.clone())
                .collect()
        } else {
            std::collections::HashSet::new()
        };

    // Wrap room config for consumer filtering
    let room_cfg = if user.is_consumer() {
        Some(room_cfg)
    } else {
        None
    };

    // -- Global data (skip for embed peers — they're pinned to one session
    // and don't render the sidebar/mission list that these fields feed) --
    let is_embed_view = ctx.view.as_deref() == Some("embed");
    if include_global && !is_embed_view {
        // All sessions — filtered by user_id
        if let Ok(sessions) = state.manager.global_sessions.list_sessions() {
            ps.all_sessions = Some(
                sessions
                    .into_iter()
                    .filter(|s| user_session_ids.contains(&s.id) && theirs(&s.id))
                    .filter_map(session_row)
                    .collect(),
            );
        }

        // Missions — admin only
        if is_admin {
            if let Ok(missions) = state.missions.list_all_missions() {
                ps.missions = Some(
                    missions
                        .into_iter()
                        .filter_map(|m| serde_json::to_value(m).ok())
                        .collect(),
                );
            }
        }
    }

    // Models + skills + pending ask-user + busy sessions
    if include_global {
        ps.ui_build = crate::server::app_pages::ui_build(state.dev_mode);
        ps.agents_met = Some(state.manager.met.status());
        // Models — admin sees all, others see shared_models only
        let models_guard = state.manager.models.read().await;
        let all_models: Vec<(_, bool)> = models_guard
            .list_models()
            .into_iter()
            .map(|m| {
                let auth_ok = models_guard.model_auth_ok(&m.id);
                (m.clone(), auth_ok)
            })
            .collect();
        drop(models_guard);
        let models = if let Some(ref cfg) = room_cfg {
            let shared: std::collections::HashSet<&str> =
                cfg.shared_models.iter().map(|s| s.as_str()).collect();
            all_models
                .into_iter()
                .filter(|(m, _)| shared.contains(m.id.as_str()))
                .collect::<Vec<_>>()
        } else {
            all_models
        };
        ps.models = Some(
            models
                .into_iter()
                .map(|(m, auth_ok)| {
                    // Only send metadata — strip sensitive fields (api_key, url).
                    // auth_ok is a runtime reading (credentials present now) so the
                    // pickers can mark a model that would only bail AUTH_REQUIRED.
                    serde_json::json!({
                        "id": m.id,
                        "provider": m.provider,
                        "model": m.model,
                        "tags": m.tags,
                        "supports_tools": m.supports_tools,
                        "provided_by": m.provided_by,
                        "auth_ok": auth_ok,
                        "supports_claude_sdk": crate::server::claude::supports_model(&m),
                    })
                })
                .collect(),
        );

        // Default models — admin only
        if is_admin {
            if let Ok((config, _)) = crate::config::Config::load_with_path() {
                ps.default_models = Some(config.routing.default_models.clone());
                ps.claude_default_model =
                    Some(config.routing.claude_default_model.clone().unwrap_or_default());
            }
        }

        // Skills — admin sees all, others see allowed_skills only
        let skills = state.skills.list_skills().await;
        let skills = if let Some(ref cfg) = room_cfg {
            let allowed: std::collections::HashSet<&str> =
                cfg.allowed_skills.iter().map(|s| s.as_str()).collect();
            skills
                .into_iter()
                .filter(|s| allowed.contains(s.name.as_str()))
                .collect::<Vec<_>>()
        } else {
            skills
        };
        ps.skills = Some(
            skills
                .into_iter()
                .filter_map(|s| {
                    let mut v = serde_json::to_value(s).ok()?;
                    // Strip large fields — UI only needs metadata for sidebar cards
                    if let Some(m) = v.as_object_mut() {
                        m.remove("content");
                        m.remove("context");
                    }
                    Some(v)
                })
                .collect(),
        );

        // Embed peers are pinned to one session — narrow user_session_ids to
        // just that session so pending_ask_user and busy_sessions can't leak
        // across sessions owned by the same user.
        let is_embed = ctx.view.as_deref() == Some("embed");
        let pinned = if is_embed {
            ctx.session_id.as_deref()
        } else {
            None
        };
        let scope_check = |sid: &str| -> bool {
            if refused || !theirs(sid) {
                return false;
            }
            if let Some(pin) = pinned {
                sid == pin
            } else {
                is_admin || user_session_ids.contains(sid)
            }
        };

        // Pending ask-user — filtered to user's sessions (+ pinned for embed)
        let pending = state.pending_ask_user.lock().await;
        let items: Vec<serde_json::Value> = pending
            .iter()
            .filter(|(_, entry)| {
                entry
                    .session_id
                    .as_deref()
                    .map_or(is_admin && pinned.is_none() && !refused, &scope_check)
            })
            .map(|(qid, entry)| crate::server::ui_events::pending_ask(qid, entry, &state.manager))
            .collect();
        ps.pending_ask_user = Some(items);

        // Busy sessions — filtered to user's sessions (+ pinned for embed)
        let active = state.active_statuses.lock().await;
        let mut busy: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for (key, record) in active.iter() {
            if let Some(sid) = key.split('|').next() {
                if !sid.is_empty() && scope_check(sid) {
                    busy.entry(sid.to_string())
                        .or_insert_with(|| record.status.as_str().to_string());
                }
            }
        }
        drop(active);
        // A session whose turn ended still works while its background
        // subagents or tasks run.
        for sid in state.tasks.sessions_running().into_iter().filter(|sid| scope_check(sid)) {
            busy.entry(sid).or_insert_with(|| "working".into());
        }
        ps.busy_sessions = Some(busy);
    }

    // -- Scoped data (admin only — project/session context) --
    if include_scoped && is_admin {
        if let Some(ref root) = ctx.project_root {
            let root_buf = PathBuf::from(root);

            if let Ok(agents) = state.manager.list_agents(&root_buf).await {
                ps.agents = Some(
                    agents
                        .into_iter()
                        .filter_map(|a| serde_json::to_value(a).ok())
                        .collect(),
                );
            }

            if let Ok(all) = state.manager.global_sessions.list_sessions() {
                let project_sessions: Vec<_> = all
                    .into_iter()
                    .filter(|s| s.project.as_deref() == Some(root.as_str()) || s.cwd == *root)
                    .filter(|s| theirs(&s.id))
                    .take(50)
                    .filter_map(|s| serde_json::to_value(s).ok())
                    .collect();
                ps.sessions = Some(project_sessions);
            }
        }

        if let Some(ref session_id) = ctx.session_id {
            let meta = state
                .manager
                .global_sessions
                .get_session_meta(session_id)
                .ok()
                .flatten();
            ps.session_id = Some(session_id.clone());
            ps.session_runtime = meta.as_ref().map(|m| m.runtime);
            let root_buf = PathBuf::from(ctx.project_root.as_deref().unwrap_or(""));
            if let Ok(runs) = state
                .manager
                .list_agent_runs(&root_buf, Some(session_id.as_str()))
                .await
            {
                let places = &state.manager.global_sessions.running_places;
                let told = crate::engine::turn_place::told_running(places, session_id, state.manager.daily_session());
                ps.agent_runs = Some(runs.into_iter().filter_map(|r| run_row(r, told.as_deref())).collect());
            }

            let queued = {
                let guard = state.queued_chats.lock().await;
                queued_entries(&guard, session_id)
            };
            ps.queued = Some(queued);
            ps.tasks = Some(state.tasks.list(session_id));

            let session_dir = crate::paths::global_sessions_dir().join(session_id);
            let perms = crate::engine::permission::SessionPermissions::load(&session_dir);
            let mut perm_val = serde_json::to_value(&perms).unwrap_or_default();

            // Prefer the session's own cwd (from SessionMeta) over the UI's
            // ctx.project_root. Skill-embed sessions send empty project_root;
            // without falling back to session.cwd, effective_mode and zone
            // would always be computed against an empty path.
            let effective_root: Option<String> = meta
                .map(|m| m.cwd)
                .or_else(|| ctx.project_root.clone().filter(|s| !s.is_empty()));

            if let Some(ref root) = effective_root {
                // Effective mode at the session's working folder. No covering
                // grant means no tools there — emit an explicit "chat" rather
                // than omitting the field: the UI keeps the last value it saw,
                // so an absent field would leave the previous folder's mode on
                // the chip after the agent cd's into an ungranted folder.
                let mode = crate::engine::permission::effective_mode_for_path(
                    &perms.path_modes,
                    std::path::Path::new(root),
                )
                .unwrap_or(crate::engine::permission::PermissionMode::Chat);
                perm_val.as_object_mut().map(|m| {
                    m.insert(
                        "effective_mode".to_string(),
                        serde_json::Value::String(mode.to_string()),
                    )
                });
            }
            ps.session_permission = Some(perm_val);
        }
    }

    ps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::events::QueuedChatItem;
    use std::collections::HashMap;

    #[test]
    fn a_page_is_told_of_rooms_only_as_the_switch_allows() {
        assert_eq!(room_facts(false, false, true), (Some(false), None));
        assert_eq!(room_facts(true, false, true), (Some(true), Some(true)));
        assert_eq!(room_facts(true, false, false), (Some(true), Some(false)));
        assert_eq!(room_facts(true, true, true), (None, None));
    }

    fn item(session: &str, agent: &str) -> QueuedChatItem {
        QueuedChatItem {
            id: "q1".into(),
            agent_id: agent.into(),
            session_id: session.into(),
            preview: "hello".into(),
            timestamp: 0,
            steerable: false,
            place: None,
            text: "hello".into(),
            images: Vec::new(),
        }
    }

    /// In a day the running top-level run names its turn's place; a
    /// subagent's, a finished one and any run with no place told name none.
    #[test]
    fn a_days_running_run_names_its_turns_place() {
        use crate::engine::agent::{AgentRunRecord, AgentRunStatus};
        let run = |id: &str, parent: Option<&str>, status| AgentRunRecord {
            run_id: id.into(),
            repo_path: String::new(),
            session_id: "sess-day-2026-11-18".into(),
            agent_id: "ling".into(),
            agent_kind: None,
            parent_run_id: parent.map(String::from),
            status,
            detail: None,
            started_at: 0,
            ended_at: None,
            model: None,
        };
        let top = run_row(run("r1", None, AgentRunStatus::Running), Some("dice")).unwrap();
        assert_eq!(top["place"], "dice");
        let child = run_row(run("r2", Some("r1"), AgentRunStatus::Running), Some("dice")).unwrap();
        assert!(child.get("place").is_none(), "{child}");
        let ended = run_row(run("r3", None, AgentRunStatus::Completed), Some("dice")).unwrap();
        assert!(ended.get("place").is_none(), "{ended}");
        let elsewhere = run_row(run("r4", None, AgentRunStatus::Running), None).unwrap();
        assert!(elsewhere.get("place").is_none(), "{elsewhere}");
    }

    /// A page that says it views a day is shown it only when it is its
    /// person's; another person's is an empty view. Other sessions as before.
    #[test]
    fn a_day_is_shown_only_to_its_own_person() {
        let day = ViewContext {
            session_id: Some("sess-day-2026-11-16".into()),
            ..Default::default()
        };
        let friend = crate::server::person::Person::for_test(Some("u-friend"));
        let (shown, refused) = day.shown_to(&friend);
        assert!(refused && shown.session_id.is_none());
        let (shown, refused) = day.shown_to(&crate::server::person::Person::test_owner());
        assert!(!refused && shown.session_id.is_some());
        let other = ViewContext {
            session_id: Some("sess-1-a".into()),
            ..Default::default()
        };
        assert!(!other.shown_to(&friend).1);
    }

    /// A session made for a person is listed to that person; one made for
    /// no one only to the owner at admin — never below it, whatever id the
    /// viewer goes by.
    #[test]
    fn a_session_made_for_no_one_is_listed_only_at_the_owners_standing() {
        assert!(listed_for(Some("u-friend"), "u-friend", false));
        assert!(!listed_for(Some("u-friend"), "u-other", true));
        assert!(listed_for(None, "u-owner", true));
        assert!(!listed_for(None, "u-friend", false));
        let local = crate::server::person::Person::from_wire(None).wire();
        assert!(!listed_for(None, &local, false), "{local}");
    }

    /// The day's row in the session list is marked; no other row changes.
    #[test]
    fn the_days_row_is_marked() {
        let day = crate::state_fs::sessions::SessionMeta {
            id: "sess-day-2026-11-16".into(),
            ..Default::default()
        };
        assert_eq!(session_row(day).unwrap()["day"], true);
        let other = crate::state_fs::sessions::SessionMeta {
            id: "sess-1-a".into(),
            ..Default::default()
        };
        assert!(session_row(other).unwrap().get("day").is_none());
    }

    /// The page is told which sessions may carry a work tag; it keeps no
    /// copy of the rule.
    #[test]
    fn a_row_says_whether_its_session_takes_a_work_tag() {
        use crate::state_fs::sessions::SessionMeta;
        let chat = SessionMeta {
            id: "sess-1-a".into(),
            ..Default::default()
        };
        assert_eq!(session_row(chat).unwrap()["takes_work"], true);
        let app = SessionMeta {
            id: "sess-2-b".into(),
            skill: Some("dice".into()),
            ..Default::default()
        };
        assert!(session_row(app).unwrap().get("takes_work").is_none());
        let mission = SessionMeta {
            id: "sess-3-c".into(),
            creator: "mission".into(),
            ..Default::default()
        };
        assert!(session_row(mission).unwrap().get("takes_work").is_none());
    }

    #[test]
    fn a_channel_peer_is_pushed_nothing() {
        let view = |v: Option<&str>| ViewContext {
            view: v.map(String::from),
            ..Default::default()
        };
        assert!(!view(Some("channel")).wants_pushes());
        for v in [None, Some("main"), Some("embed"), Some("consumer")] {
            assert!(view(v).wants_pushes(), "{v:?}");
        }
    }

    #[test]
    fn names_the_agent_its_items_are_for() {
        let mut all = HashMap::new();
        all.insert("sess-1|ling".to_string(), vec![item("sess-1", "ling")]);
        let out = queued_entries(&all, "sess-1");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["agent_id"], "ling");
    }

    #[test]
    fn a_drained_queue_yields_no_entry() {
        // The snapshot must be able to say "nothing queued" — that is the state
        // a cancelled run leaves behind, and the one clients used to miss.
        let mut all = HashMap::new();
        all.insert("sess-1|ling".to_string(), Vec::new());
        assert!(queued_entries(&all, "sess-1").is_empty());
    }

    #[test]
    fn other_sessions_are_not_leaked() {
        let mut all = HashMap::new();
        all.insert("sess-1|ling".to_string(), vec![item("sess-1", "ling")]);
        all.insert("sess-2|ling".to_string(), vec![item("sess-2", "ling")]);
        let out = queued_entries(&all, "sess-1");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["items"][0]["session_id"], "sess-1");
    }
}
