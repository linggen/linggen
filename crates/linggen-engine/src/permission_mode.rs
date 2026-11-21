//! A session's permission mode: the ceiling of what an agent may do.
//!
//! A leaf module: the paired-device store, the config and the engine all name
//! the mode, so none of them has to reach into another to do it. What the
//! modes allow (the checks, the grants) stays in `engine::permission`.

use serde::{Deserialize, Serialize};

/// Session permission mode — defines the ceiling of what the agent can do.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub enum PermissionMode {
    Chat,
    #[default]
    Read,
    Edit,
    Admin,
}

/// The one table of a mode's words: what it prints and stores as
/// ([`Self::as_str`], the same as its serde word) and what reads it back
/// ([`Self::parse`], [`Self::parse_written`]). Each caller narrows the
/// parsed mode to what it takes; no caller matches the words itself
/// (lock: `structure_lock_tests::a_permission_mode_word_is_parsed_in_one_place`).
impl PermissionMode {
    /// Every mode, lowest first.
    pub const ALL: [Self; 4] = [Self::Chat, Self::Read, Self::Edit, Self::Admin];

    /// The word a mode prints, stores and travels as.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Admin => "admin",
        }
    }

    /// The mode `word` names, exactly as [`Self::as_str`] prints it.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == word)
    }

    /// The mode a person wrote in a skill's or mission's YAML: the printed
    /// words, and `write` for edit (people write "write"; the engine says
    /// "edit"). The stored word is kept as written.
    pub fn parse_written(word: &str) -> Option<Self> {
        match word {
            "write" => Some(Self::Edit),
            _ => Self::parse(word),
        }
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
