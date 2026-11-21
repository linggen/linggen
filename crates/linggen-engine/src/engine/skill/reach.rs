//! Where an app's action is offered: `reach:` in a skill's `tools:`.
//!
//! `here` (the default) — only in the app's own session, as every tool was
//! before `reach` existed. `anywhere` — also from any other session of the
//! owner's, through `AppTool`, and to Yinyue. A closed set held as this type
//! from the moment the skill is read, as the class is
//! (`permission::ActionClass`): the written word is read here and nowhere
//! else. Who may then run what is offered is `skill::judge`'s alone.

use crate::engine::skill_tool::SkillToolDef;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reach {
    /// Only in the app's own session.
    #[default]
    Here,
    /// Also from other sessions, through `AppTool`.
    Anywhere,
}

impl Reach {
    /// The reach a skill's `reach:` word names. A word this engine does not
    /// know is the narrow one, and is said in the log.
    pub fn declared(word: &str) -> Self {
        match word.trim() {
            "anywhere" => Self::Anywhere,
            "here" => Self::Here,
            other => {
                tracing::warn!("skill tool: unknown reach '{other}'; it stays 'here'");
                Self::Here
            }
        }
    }

    /// The word the reach is written as.
    pub fn word(self) -> &'static str {
        match self {
            Self::Here => "here",
            Self::Anywhere => "anywhere",
        }
    }
}

/// Read from the skill's `reach:` — absent or empty is `here`.
impl<'de> Deserialize<'de> for Reach {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let word = Option::<String>::deserialize(d)?;
        Ok(word.as_deref().map(Self::declared).unwrap_or_default())
    }
}

impl Serialize for Reach {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.word())
    }
}

/// What an app's action is offered as to the companion, who reaches it
/// through `AppTool` from a session that is not the app's own. What she
/// may then run of it is `skill::judge`'s alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// Not offered outside the app's own session.
    Nobody,
    /// Hers alone, to read (`pet: true`).
    HerRead,
    /// Offered anywhere (`reach: anywhere`): its class and who started the
    /// turn decide.
    Anywhere,
}

impl Offer {
    pub fn of(tool: &SkillToolDef) -> Self {
        match (tool.reach, tool.pet) {
            (Reach::Anywhere, _) => Self::Anywhere,
            (Reach::Here, true) => Self::HerRead,
            (Reach::Here, false) => Self::Nobody,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_word_is_one_reach() {
        for (word, reach) in [
            ("anywhere", Reach::Anywhere),
            (" anywhere ", Reach::Anywhere),
            ("here", Reach::Here),
            ("", Reach::Here),
            ("everywhere", Reach::Here),
            ("Anywhere", Reach::Here),
        ] {
            assert_eq!(Reach::declared(word), reach, "{word:?}");
        }
        assert_eq!(Reach::default(), Reach::Here, "no word is the narrow one");
    }

    #[test]
    fn a_reach_is_read_from_a_skill_and_published_as_its_word() {
        let read = |yaml: &str| serde_norway::from_str::<Reach>(yaml).unwrap();
        assert_eq!(read("anywhere"), Reach::Anywhere);
        assert_eq!(read("null"), Reach::Here);
        for reach in [Reach::Here, Reach::Anywhere] {
            let json = serde_json::to_value(reach).unwrap();
            assert_eq!(json, serde_json::json!(reach.word()));
            assert_eq!(serde_json::from_value::<Reach>(json).unwrap(), reach);
        }
    }
}
