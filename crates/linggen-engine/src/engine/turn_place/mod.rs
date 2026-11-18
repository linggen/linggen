//! A turn's place: the app a turn comes from — its skill, or none ("home":
//! Yinyue's own surfaces, a plain chat) — and the values the session is
//! bound to for that skill (a game's save). What an app gives a turn — its
//! instructions, tools, folder, grants, how a second message waits, which
//! memory it recalls — is taken from the turn's place, not from the session.
//!
//! Today every turn of a session has the session's own place: a page names
//! the skill its session was made for. See `doc/session-spec.md` § A turn's
//! place.

use crate::engine::skill::params::SessionParams;
use crate::state_fs::sessions::SessionMeta;

/// The key of the place with no skill. A skill's name is lowercase letters,
/// digits and hyphens (`doc/skill-spec.md` § Frontmatter), never a colon.
pub const HOME_KEY: &str = ":home";

/// Where one turn stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnPlace {
    /// The skill whose page the turn comes from; `None` is home.
    pub skill: Option<String>,
    /// The values the session is bound to for that skill
    /// (`SessionMeta::params`) — empty for home, and for a skill that is not
    /// the session's own: a session's values belong to its own skill.
    pub params: SessionParams,
    /// The session was made for this place (its `skill`, or none). Its grants
    /// are the session's own, its folder the session's.
    pub own: bool,
}

impl TurnPlace {
    /// Home, in a session that is home's own.
    pub fn home() -> Self {
        Self {
            own: true,
            ..Self::default()
        }
    }

    /// The place a page request names: the page's skill when it names one,
    /// else the session's own. `meta` is `None` for a session not made yet —
    /// the request then makes it for this very place.
    pub fn of_request(page_skill: Option<&str>, meta: Option<&SessionMeta>) -> Self {
        let page_skill = page_skill.map(str::trim).filter(|s| !s.is_empty());
        match page_skill {
            Some(name) => Self::of_skill_in(name, meta),
            None => Self::of_session(meta),
        }
    }

    /// The place of a turn no page sent — Yinyue's own turns, her turn at an
    /// app's table, a light turn, a look-again wake, a resumed or relayed
    /// message: the session's own skill (home when it has none). A delegated
    /// run takes its parent turn's place instead; a mission run binds none.
    pub fn of_session(meta: Option<&SessionMeta>) -> Self {
        let Some(meta) = meta else {
            return Self::home();
        };
        Self {
            skill: meta.skill.clone(),
            params: meta.params.clone(),
            own: true,
        }
    }

    /// `name`'s place in the session `meta` (or in a session about to be
    /// made for it).
    fn of_skill_in(name: &str, meta: Option<&SessionMeta>) -> Self {
        let Some(meta) = meta else {
            return Self {
                skill: Some(name.to_string()),
                params: SessionParams::new(),
                own: true,
            };
        };
        if meta.skill.as_deref() == Some(name) {
            return Self::of_session(Some(meta));
        }
        Self {
            skill: Some(name.to_string()),
            params: SessionParams::new(),
            own: false,
        }
    }

    /// The name this place is kept under — in `permission.json`, on a saved
    /// row: the skill's name, or [`HOME_KEY`].
    pub fn key(&self) -> &str {
        self.skill.as_deref().unwrap_or(HOME_KEY)
    }

    /// The slot this place's grants are kept under in the session's
    /// `permission.json`: `None` for the session's own place, whose grants
    /// are the file's top level (as before places).
    pub fn grants_key(&self) -> Option<&str> {
        (!self.own).then(|| self.key())
    }

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

    /// Whether two turns stand in the same app. The bound values are the
    /// session's, so they never tell two places of one session apart.
    pub fn same_app(&self, other: &TurnPlace) -> bool {
        self.skill == other.skill
    }
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
    let day_on = manager.get_config_snapshot().await.agent.daily_session;
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
        let in_day = self
            .session_id
            .as_deref()
            .is_some_and(crate::state_fs::day_session::is_day_session);
        if !skill.private_talk || !in_day {
            return false;
        }
        match self.get_manager() {
            Some(manager) => manager.get_config_snapshot().await.agent.daily_session,
            None => true,
        }
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
