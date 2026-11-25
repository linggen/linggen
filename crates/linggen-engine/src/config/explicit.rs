//! A setting the person may have chosen, kept apart from one never set.
//!
//! The file says either nothing (the key is absent: the setting follows the
//! default of the version that is running) or a value the person chose. A
//! value that equals today's default is still a choice: a later version may
//! move the default, and the person's word must not move with it. So a
//! saved config writes every `Explicit` that is set and none that is not.
//!
//! This is the one place that decides it. A config type holds `Explicit<T>`
//! and skips it with [`Explicit::is_unset`]; it never compares a value (or a
//! whole section) with `Default` to leave it out — a lock test in
//! `config.rs` (`a_saved_config_never_drops_a_value_for_equalling_the_default`)
//! fails on that.

use serde::{Deserialize, Serialize};

/// `T`'s `#[default]` is the value while unset (`get`); there is no other
/// copy of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Explicit<T>(Option<T>);

impl<T> Default for Explicit<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T: Default + Copy> Explicit<T> {
    /// The person chose `value` (even when it is the default).
    pub fn set(value: T) -> Self {
        Self(Some(value))
    }

    /// What applies now: the choice, else this version's default.
    pub fn get(&self) -> T {
        self.0.unwrap_or_default()
    }
}

impl<T> Explicit<T> {
    /// Never set: absent on disk. The `skip_serializing_if` of every field.
    pub fn is_unset(&self) -> bool {
        self.0.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same words, two versions: the default moved from `Old` to `New`.
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Before {
        #[default]
        Old,
        New,
    }
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum After {
        Old,
        #[default]
        New,
    }

    #[derive(Debug, Default, Serialize, Deserialize)]
    struct File<T: Default + Copy> {
        #[serde(default, skip_serializing_if = "Explicit::is_unset")]
        switch: Explicit<T>,
    }

    fn save<T: Default + Copy + Serialize>(file: &File<T>) -> String {
        toml::to_string(file).unwrap()
    }

    #[test]
    fn a_choice_equal_to_the_default_survives_a_default_that_moves() {
        let chosen: File<Before> = toml::from_str("switch = \"old\"").unwrap();
        let text = save(&chosen);
        assert!(text.contains("switch = \"old\""), "written back: {text}");
        let later: File<After> = toml::from_str(&text).unwrap();
        assert_eq!(
            later.switch.get(),
            After::Old,
            "the person's word, not the new default"
        );
    }

    #[test]
    fn a_setting_never_chosen_stays_absent_and_follows_the_default() {
        let never: File<Before> = toml::from_str("").unwrap();
        let text = save(&never);
        assert!(!text.contains("switch"), "absent: {text}");
        let later: File<After> = toml::from_str(&text).unwrap();
        assert_eq!(
            later.switch.get(),
            After::New,
            "the default moved, so it moved"
        );
    }
}
