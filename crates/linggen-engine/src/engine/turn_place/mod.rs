//! A turn's place: the app a turn comes from — its skill, or none ("home":
//! Yinyue's own surfaces, a plain chat) — and the values the session is
//! bound to for that skill (a game's save). What an app gives a turn — its
//! instructions, tools, folder, grants, how a second message waits, which
//! memory it recalls — is taken from the turn's place, not from the session.
//!
//! The value itself (`TurnPlace`, `HOME_KEY`, and what is read off a session
//! row alone) lives in `state_fs::turn_place`, below the store, which keeps a
//! running turn's place. What needs the skills and the agent manager is here,
//! as a second `impl TurnPlace`.
//!
//! Today every turn of a session has the session's own place: a page names
//! the skill its session was made for. See `doc/session-spec.md` § A turn's
//! place.

pub use crate::state_fs::turn_place::{TurnPlace, HOME_KEY};

impl TurnPlace {
    /// The app a turn standing here keeps its talk to, with the daily session on (`day_on`): this place's app when it declares
    /// `private-talk: true`, or does not resolve now — when it is not known
    /// to be shared, it is kept. Home's never is. Such an app keeps a
    /// session of its own, and from its turn no tool carries words out
    /// (`tools::exit_gate`).
    pub async fn private_app(&self, skills: &dyn crate::engine::skill::SkillRegistry, day_on: bool) -> Option<String> {
        if !day_on {
            return None;
        }
        let name = self.skill.as_deref()?;
        let shared = skills.get_skill(name).await.is_some_and(|s| !s.private_talk);
        (!shared).then(|| name.to_string())
    }

    /// The folder a turn standing here works in when this is not the
    /// session's own place: `skill`'s (the place's skill as entered), else
    /// home's default workspace. `None` for the session's own place — its
    /// folder is the session's.
    pub fn folder(&self, skill: Option<&crate::engine::skill::Skill>) -> Option<std::path::PathBuf> {
        if self.own {
            return None;
        }
        Some(match skill {
            Some(s) => s.working_dir(),
            None => crate::util::resolve_path(&crate::paths::default_workspace()),
        })
    }
}

/// The folder a page's own command (a `!` line) starts in at `place` of
/// session `session_id` — where a turn standing there works
/// ([`TurnPlace::folder`]): a private app kept out of a day leaves home's
/// workspace, as entering the place leaves it. `None` for the session's
/// own place.
pub async fn folder_at(
    manager: &crate::engine::agent::AgentManager,
    session_id: &str,
    place: &TurnPlace,
) -> Option<std::path::PathBuf> {
    if place.own {
        return None;
    }
    let skill = skill_at(manager, session_id, place).await;
    place.folder(skill.as_ref())
}

/// The skill a door that acts at `place` of session `session_id` (a `!`
/// line, the prompt export) enters, as a turn entering it would: the
/// place's skill when it resolves, and none for a private app a day keeps
/// out (`AgentEngine::enter_place`).
pub async fn skill_at(
    manager: &crate::engine::agent::AgentManager,
    session_id: &str,
    place: &TurnPlace,
) -> Option<crate::engine::skill::Skill> {
    let skill = manager.skills.reload_one(place.skill.as_deref()?).await?;
    let day_on = Some(manager.daily_session());
    (!kept_out(&skill, Some(session_id), day_on)).then_some(skill)
}

/// Whether session `session_id` keeps `skill` out: a day's session, with the
/// daily session on (`day_on`; `None` when it cannot be read, and a day then
/// keeps it out), keeps a private app (`private-talk: true`) out.
fn kept_out(skill: &crate::engine::skill::Skill, session_id: Option<&str>, day_on: Option<bool>) -> bool {
    let in_day = session_id.is_some_and(crate::state_fs::day_session::is_day_session);
    if !skill.private_talk || !in_day {
        return false;
    }
    day_on.unwrap_or(true)
}

/// The place a page is told a turn of session `session_id` stands in — a
/// running turn, a waiting message (`doc/one-ling-spec.md` § Linggen's daily
/// session): the place's key, as its rows keep it, in a day's session with
/// the daily session on (`day_on`); `None` anywhere else, so no payload of
/// another session names one. A private app is never a place in a day (it
/// keeps a session of its own), so none is ever told. Every payload that
/// tells a page a place asks here.
pub fn told_place(place: &TurnPlace, session_id: &str, day_on: bool) -> Option<String> {
    told_in(session_id, day_on).then(|| place.key().to_string())
}

/// [`told_place`] for the turn running in `session_id` now, when its place is
/// kept. Outside a day nothing is looked up.
pub fn told_running(
    places: &crate::state_fs::running_places::RunningPlaces,
    session_id: &str,
    day_on: bool,
) -> Option<String> {
    if !told_in(session_id, day_on) {
        return None;
    }
    told_place(&places.of(session_id)?, session_id, day_on)
}

/// Whether a page viewing `session_id` is told places: a day's, setting on.
fn told_in(session_id: &str, day_on: bool) -> bool {
    day_on && crate::state_fs::day_session::is_day_session(session_id)
}

/// The private app a question waiting in session `session_id` is kept to
/// (`TurnPlace::private_app`): the place of the turn running there, else the
/// session's own. Its words never reach a turn outside that app's own
/// session — at most that the app waits. `None` with the setting off.
pub async fn question_kept_to(
    manager: &crate::engine::agent::AgentManager,
    session_id: Option<&str>,
) -> Option<String> {
    let sid = session_id?;
    let place = match manager.global_sessions.running_places.of(sid) {
        Some(place) => place,
        None => {
            let meta = manager.global_sessions.get_session_meta(sid).ok().flatten();
            TurnPlace::of_session(meta.as_ref())
        }
    };
    let day_on = manager.daily_session();
    place.private_app(&*manager.skills, day_on).await
}

impl crate::engine::tools::Tools {
    /// Whether this session keeps `skill` out: in a day's session, with the daily session on, a private app (`private-talk: true`) never becomes
    /// a turn's place nor its active skill — it keeps a session of its own.
    /// The two setters ask here (`AgentEngine::activate_skill`,
    /// `AgentEngine::enter_place`), and so does the Skill tool before it
    /// reads the SKILL.md out. With no manager to read the setting from, a
    /// day keeps it out.
    pub(crate) async fn kept_out_of_day(&self, skill: &crate::engine::skill::Skill) -> bool {
        let day_on = self.get_manager().map(|m| m.daily_session());
        kept_out(skill, self.session_id.as_deref(), day_on)
    }
}

pub(crate) mod bind;
pub(crate) mod held;
pub use held::Held;
pub mod running;
pub mod turn;
pub use turn::{Kickoff, RunSetup, TurnSettings};

#[cfg(test)]
mod lock_tests;
#[cfg(test)]
mod tests;
