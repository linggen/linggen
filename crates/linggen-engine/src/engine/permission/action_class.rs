//! What an app's declared action does to the person's things — its class —
//! and the one place the word a skill writes for it (`tier:` in `SKILL.md`)
//! becomes one.
//!
//! The class is a closed set, held as this type from the moment the skill
//! is read: nothing after that sees the written word, so nothing can read
//! it a second time (lock: `structure_lock_tests`). The one question a
//! class answers for itself is the level it needs ([`ActionClass::level_needed`]);
//! who may run an action of a class, and when a model is asked first, is
//! decided by `engine::skill::judge` alone.

use super::PermissionMode;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The class of an app's action. No order: compare levels
/// ([`Self::level_needed`]), never classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionClass {
    Read,
    Edit,
    /// An action that destroys what it acts on: it needs what `Edit`
    /// needs, and a model that asks for one is always asked of the person
    /// (`engine::skill::judge`).
    Destructive,
    /// What a skill that wrote no class, or one this engine does not know,
    /// gets: the strict default, written here and nowhere else.
    #[default]
    Admin,
}

impl ActionClass {
    /// The class a skill's `tier:` word names: the mode words (`write`
    /// for `edit`, [`PermissionMode::parse_written`]); anything else —
    /// `chat` included, which is no class — is the strict default.
    pub fn declared(word: &str) -> Self {
        if word == "destructive" {
            return Self::Destructive;
        }
        match PermissionMode::parse_written(word) {
            Some(PermissionMode::Read) => Self::Read,
            Some(PermissionMode::Edit) => Self::Edit,
            _ => Self::default(),
        }
    }

    /// The level an action of this class needs, of a grant and of a caller.
    pub fn level_needed(self) -> PermissionMode {
        match self {
            Self::Read => PermissionMode::Read,
            Self::Edit | Self::Destructive => PermissionMode::Edit,
            Self::Admin => PermissionMode::Admin,
        }
    }

    /// Whether an action of this class may change the person's things: a
    /// fact about the class (a skill's save is pushed after one; a tap on
    /// an offer is described by it), not a judgment of who may run it.
    pub fn changes_things(self) -> bool {
        self != Self::Read
    }

    /// Whether the action destroys what it acts on: a model that asks for
    /// one is asked of the person every time.
    pub fn destroys(self) -> bool {
        self == Self::Destructive
    }

    /// The word the class is published as (`mac/tools`, an offer).
    pub fn word(self) -> &'static str {
        match self {
            Self::Destructive => "destructive",
            other => other.level_needed().as_str(),
        }
    }
}

impl std::fmt::Display for ActionClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.word())
    }
}

/// Read from the skill's `tier:` — absent or empty is the default.
impl<'de> Deserialize<'de> for ActionClass {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let word = Option::<String>::deserialize(d)?;
        Ok(word.as_deref().map(Self::declared).unwrap_or_default())
    }
}

impl Serialize for ActionClass {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.word())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_word_is_one_class() {
        for (word, class) in [
            ("read", ActionClass::Read),
            ("edit", ActionClass::Edit),
            ("write", ActionClass::Edit),
            ("admin", ActionClass::Admin),
            ("destructive", ActionClass::Destructive),
            ("chat", ActionClass::Admin),
            ("Admin", ActionClass::Admin),
            ("", ActionClass::Admin),
            ("nonsense", ActionClass::Admin),
        ] {
            assert_eq!(ActionClass::declared(word), class, "{word:?}");
        }
        assert_eq!(ActionClass::default(), ActionClass::Admin, "no word is the strict default");
    }

    #[test]
    fn a_class_is_read_from_a_skill_and_published_as_its_word() {
        let read = |yaml: &str| serde_norway::from_str::<ActionClass>(yaml).unwrap();
        assert_eq!(read("read"), ActionClass::Read);
        assert_eq!(read("write"), ActionClass::Edit);
        assert_eq!(read("null"), ActionClass::Admin);
        for class in [ActionClass::Read, ActionClass::Edit, ActionClass::Destructive, ActionClass::Admin] {
            let json = serde_json::to_value(class).unwrap();
            assert_eq!(json, serde_json::json!(class.word()));
            assert_eq!(serde_json::from_value::<ActionClass>(json).unwrap(), class);
        }
    }
}
