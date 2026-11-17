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

    /// The app a turn standing here keeps its talk to, with one session a
    /// day on (`day_on`): this place's app when it declares
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

pub(crate) mod bind;
pub mod running;

#[cfg(test)]
mod tests;
