//! Which kind of session a kept one is, read from what its meta holds
//! today — creator, runtime, id shape, bound values, withheld tools. No
//! field is stored for it.

use crate::state_fs::sessions::SessionMeta;

/// The occasions a session is made for: one row each in the spec's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// A person's own chat (New chat, a first message).
    Chat,
    /// An app's chat (`skill` sessions).
    App,
    /// Linggen's daily session (`sess-day-…`).
    Daily,
    /// The companion's own rolling thread (the daily session off).
    Companion,
    /// A thread an agent hands a message into (`sess-handoff-…`).
    Handoff,
    /// A mission's run, and the session it leaves.
    Mission,
    /// An `agent_run` delegate, made by an external host.
    Delegate,
    /// An app's one-off job: bound to its own values or with tools withheld.
    OneOff,
    /// A Claude session from 2.5.1, which the Claude Agent SDK hosted: a
    /// read-only record now (`SessionMeta::is_old_claude`).
    OldClaude,
}

impl Preset {
    /// Whether a session of this kind may carry a work tag: the ones a
    /// person or a caller makes for a job of their own. Every other kind
    /// ignores a tag its file holds.
    pub fn takes_work(self) -> bool {
        matches!(self, Self::Chat | Self::Delegate | Self::Handoff)
    }
}

/// The id prefix of a hand-off's session.
pub const HANDOFF_PREFIX: &str = "sess-handoff-";

/// The first rule that matches names the preset; none is a person's chat.
const RULES: &[(fn(&SessionMeta) -> bool, Preset)] = &[
    (old_claude, Preset::OldClaude),
    (mission_run, Preset::Mission),
    (delegated, Preset::Delegate),
    (day, Preset::Daily),
    (handed_off, Preset::Handoff),
    (companion_thread, Preset::Companion),
    (one_off, Preset::OneOff),
    (app, Preset::App),
];

/// The preset of a session kept on disk.
pub fn preset_of(meta: &SessionMeta) -> Preset {
    RULES
        .iter()
        .find(|(rule, _)| rule(meta))
        .map_or(Preset::Chat, |(_, preset)| *preset)
}

fn old_claude(meta: &SessionMeta) -> bool {
    meta.is_old_claude()
}

fn mission_run(meta: &SessionMeta) -> bool {
    meta.creator == "mission" || meta.mission_id.is_some()
}

fn delegated(meta: &SessionMeta) -> bool {
    meta.creator == "external"
}

fn day(meta: &SessionMeta) -> bool {
    crate::state_fs::day_session::is_day_session(&meta.id)
}

fn handed_off(meta: &SessionMeta) -> bool {
    meta.id.starts_with(HANDOFF_PREFIX)
}

/// The companion's own thread: `sess-<companion>` and its rolled segments.
pub fn is_companion_thread(session_id: &str) -> bool {
    let prefix = format!("sess-{}", crate::engine::agent::COMPANION_AGENT_ID);
    session_id
        .strip_prefix(&prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

fn companion_thread(meta: &SessionMeta) -> bool {
    is_companion_thread(&meta.id)
}

/// Bound to its own values, or made with tools withheld.
pub fn is_kept_apart(meta: &SessionMeta) -> bool {
    !meta.params.is_empty() || !meta.withheld_tools.is_empty()
}

fn one_off(meta: &SessionMeta) -> bool {
    meta.skill.is_some() && is_kept_apart(meta)
}

fn app(meta: &SessionMeta) -> bool {
    meta.skill.is_some()
}
