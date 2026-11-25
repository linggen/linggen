//! Which edition of the list a person is offered: decided here, nowhere else.
//!
//! The Mac's Region (Language & Region, the current locale's region, not the
//! language, time zone or IP) says it: `CN` is the mainland edition, anything
//! else or nothing is the international one. The owner may choose either
//! edition instead (`[model_catalog] edition`, "auto" by default).
//!
//! The choice changes what is OFFERED for a new setup (the providers to add,
//! their starters, their marks). It never touches what a person already has:
//! no key, row or default moves, and a connection the other edition names
//! stays on the page under its own address, marked kept. A lock test
//! (`edition_is_decided_only_in_this_file`) holds that.

use super::bundle::Edition;
use crate::config::Explicit;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// What the owner said: follow the Mac's region, or one edition outright.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditionChoice {
    #[default]
    Auto,
    International,
    Mainland,
}

impl EditionChoice {
    /// The word a page is told; `parse` is its inverse.
    pub fn as_str(self) -> &'static str {
        match self {
            EditionChoice::Auto => "auto",
            EditionChoice::International => "international",
            EditionChoice::Mainland => "mainland",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        [Self::Auto, Self::International, Self::Mainland].into_iter().find(|c| c.as_str() == word)
    }
}

/// `[model_catalog]` in the engine config. The field is private: the owner's
/// settings route is the only writer (`chosen`); a config saved from any page
/// keeps what the engine holds.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelCatalog {
    #[serde(skip_serializing_if = "Explicit::is_unset")]
    edition: Explicit<EditionChoice>,
}

impl ModelCatalog {
    pub fn chosen(edition: EditionChoice) -> Self {
        Self { edition: Explicit::set(edition) }
    }

    pub fn choice(&self) -> EditionChoice {
        self.edition.get()
    }

    /// Nothing chosen: no `[model_catalog]` on disk.
    pub fn is_unset(&self) -> bool {
        self.edition.is_unset()
    }

    /// The edition this person is offered now. The one decision.
    pub(crate) fn offered(&self) -> Edition {
        self.offered_in(system_region().as_deref())
    }

    /// What a page is told: the edition in force and the choice behind it.
    pub fn shown(&self) -> serde_json::Value {
        serde_json::json!({"edition": self.offered().as_str(), "choice": self.edition.get().as_str()})
    }

    /// [`offered`](Self::offered) for a given Mac region (what the tests hold fixed).
    pub(crate) fn offered_in(&self, region: Option<&str>) -> Edition {
        match self.edition.get() {
            EditionChoice::International => Edition::International,
            EditionChoice::Mainland => Edition::Mainland,
            EditionChoice::Auto => match region {
                Some(code) if code.eq_ignore_ascii_case("CN") => Edition::Mainland,
                _ => Edition::International,
            },
        }
    }
}

/// The Mac's Region, read once per process (as the language is): it rarely
/// changes, and the owner can choose an edition meanwhile.
fn system_region() -> Option<String> {
    static REGION: OnceLock<Option<String>> = OnceLock::new();
    REGION.get_or_init(read_apple_locale).clone()
}

fn read_apple_locale() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::process::Command::new("defaults").args(["read", "-g", "AppleLocale"]).output().ok()?;
    out.status.success().then(|| region_of_locale(&String::from_utf8_lossy(&out.stdout))).flatten()
}

/// The region of a macOS locale identifier: `en_CN` is `CN`; a Region chosen
/// apart from the language is written `en_US@rg=cnzzzz` and is `CN`.
fn region_of_locale(identifier: &str) -> Option<String> {
    let identifier = identifier.trim();
    let region_override = identifier.split('@').skip(1).flat_map(|part| part.split(';'))
        .find_map(|setting| setting.strip_prefix("rg="))
        .map(|rg| rg.chars().take(2).collect::<String>());
    let from_name = identifier.split(['@', '.']).next().and_then(|name| name.split('_').nth(1))
        .map(str::to_owned);
    region_override.or(from_name).filter(|r| r.len() == 2 && r.chars().all(|c| c.is_ascii_alphabetic()))
        .map(|r| r.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_macs_region_is_read_from_its_locale_identifier() {
        for (identifier, region) in [
            ("en_CN\n", Some("CN")), ("zh_CN", Some("CN")), ("zh-Hans_CN", Some("CN")), ("en_US", Some("US")),
            ("en_US@rg=cnzzzz", Some("CN")), ("zh_CN@rg=hkzzzz", Some("HK")), ("en_US@calendar=gregorian", Some("US")),
            ("en_US.UTF-8", Some("US")), ("en", None), ("", None), ("C", None), ("xx_9", None),
        ] {
            assert_eq!(region_of_locale(identifier).as_deref(), region, "{identifier:?}");
        }
    }

    #[test]
    fn the_mainland_edition_is_offered_for_cn_or_by_choice_and_never_otherwise() {
        let auto = ModelCatalog::default();
        assert_eq!(auto.choice(), EditionChoice::Auto);
        for (region, edition) in [(Some("CN"), Edition::Mainland), (Some("cn"), Edition::Mainland), (Some("US"), Edition::International),
            (Some("HK"), Edition::International), (None, Edition::International)] {
            assert_eq!(auto.offered_in(region), edition, "{region:?}");
        }
        // A choice wins over the Mac's region, both ways.
        assert_eq!(ModelCatalog::chosen(EditionChoice::International).offered_in(Some("CN")), Edition::International);
        assert_eq!(ModelCatalog::chosen(EditionChoice::Mainland).offered_in(Some("US")), Edition::Mainland);
        assert_eq!(ModelCatalog::chosen(EditionChoice::Mainland).offered_in(None), Edition::Mainland);
    }

    #[test]
    fn the_setting_is_written_only_when_chosen_and_an_unknown_word_is_refused() {
        assert!(ModelCatalog::default().is_unset());
        assert!(!ModelCatalog::chosen(EditionChoice::Mainland).is_unset());
        let read = |text: &str| toml::from_str::<ModelCatalog>(text);
        assert_eq!(read("edition = \"mainland\"").unwrap().choice(), EditionChoice::Mainland);
        assert_eq!(read("").unwrap().choice(), EditionChoice::Auto);
        assert!(read("edition = \"world\"").is_err());
        for choice in [EditionChoice::Auto, EditionChoice::International, EditionChoice::Mainland] {
            assert_eq!(EditionChoice::parse(choice.as_str()), Some(choice));
        }
        assert_eq!(EditionChoice::parse("Mainland"), None);
    }
}
