//! A turn's place, as a value: the app a turn comes from (its skill, or
//! none — "home") and the values the session is bound to for that skill.
//!
//! The data half, kept below the store: the store holds a running turn's
//! place (`running_places`) and reads a row's. What an app gives a turn
//! (its folder, its private talk) needs the skills and is added in
//! `engine::turn_place` as a second `impl TurnPlace`.

use super::sessions::{SessionMeta, SessionParams};

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

    /// Whether two turns stand in the same app. The bound values are the
    /// session's, so they never tell two places of one session apart.
    pub fn same_app(&self, other: &TurnPlace) -> bool {
        self.skill == other.skill
    }
}
