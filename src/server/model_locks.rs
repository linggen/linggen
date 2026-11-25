//! Locks on two rules the engine decides in one place each, read from the
//! source (`lock_scan`), tests cut out:
//!
//! - a model's `provider` and `auth_mode` strings are read only in
//!   `config/sign_in.rs` (`ModelConfig::provider_kind`, `sign_in`);
//! - what a provider reported about its cache is read only by `Tokens::of`
//!   (`provider/models/usage.rs`) and the providers that write it.

use super::lock_scan::production_code;

const DIRS: &[&str] = &["src", "crates/linggen-engine/src"];

/// Each production line holding one of `needles`, outside the `allowed`
/// files; and whether `must_scan` was among the files read.
fn lines_with(needles: &[&str], allowed: impl Fn(&str) -> bool, must_scan: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut scanned = false;
    for (rel, code) in production_code(DIRS) {
        let rel = rel.replace('\\', "/");
        scanned |= rel == must_scan;
        if allowed(&rel) || rel == "src/server/model_locks.rs" {
            continue;
        }
        for (n, line) in code.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if needles.iter().any(|needle| line.contains(needle)) {
                found.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(scanned, "the scan did not reach {must_scan}");
    found
}

#[test]
fn provider_and_auth_mode_strings_are_compared_only_in_sign_in() {
    let found = lines_with(
        &[
            "provider == \"",
            "provider != \"",
            "provider.as_str()",
            "\"chatgpt_oauth\"",
            "\"claude_oauth\"",
            "\"linggen_account\"",
        ],
        |rel| rel.ends_with("config/sign_in.rs"),
        "src/cli/status.rs",
    );
    assert!(
        found.is_empty(),
        "match on ModelConfig::provider_kind() / sign_in() instead:\n{}",
        found.join("\n")
    );
}

#[test]
fn a_reports_cache_counts_are_read_only_through_tokens_of() {
    let found = lines_with(
        &[".cached_tokens", ".cache_write_tokens"],
        |rel| rel.starts_with("crates/linggen-engine/src/provider/"),
        "crates/linggen-engine/src/engine/cloud_meter.rs",
    );
    assert!(
        found.is_empty(),
        "read Tokens::of instead:\n{}",
        found.join("\n")
    );
}

/// The v1 public catalog is retired: its file and address are named nowhere
/// (the curated list, `provider/models/curated`, is the one reader and the one
/// owner of its own file and address; a test there counts it).
#[test]
fn the_retired_v1_catalog_is_named_nowhere() {
    let found = lines_with(
        &["/model-catalog/v1.json", "model-catalog-v1.json"],
        |_| false,
        "crates/linggen-engine/src/provider/models/family_catalog.rs",
    );
    assert!(found.is_empty(), "the v1 catalog is gone: {found:?}");
}

#[test]
fn public_family_ranking_has_one_entry_point() {
    let selector = "crates/linggen-engine/src/provider/models/family_catalog/selection.rs";
    let found = lines_with(&["fn family_current(" , "fn family_current<"], |rel| rel == selector, selector);
    assert!(found.is_empty(), "use the typed family selector: {found:?}");
}

const CATALOG_DIR: &str = "crates/linggen-engine/src/provider/models/family_catalog";

/// What the public catalog says about a row (`text_chat`: a plain text
/// conversation runs on it) reaches a row's kind through `provider::kinds`
/// alone; the family selector asks the same fact. A second reader would be a
/// second answer to "is this a chat model".
#[test]
fn the_catalogs_chat_fact_is_read_only_by_kinds_and_the_family_selector() {
    let kinds = "crates/linggen-engine/src/provider/kinds.rs";
    let found = lines_with(
        &[".text_chat("],
        |rel| rel == kinds || rel.starts_with(CATALOG_DIR),
        kinds,
    );
    assert!(found.is_empty(), "ask provider::kinds::kind_of instead: {found:?}");
}

/// A price comes from `provider/models/pricing.rs` alone: the catalog's rates
/// and tiers are read there, and no model name is priced by a hand table.
#[test]
fn the_catalogs_price_facts_are_read_only_by_pricing_and_no_table_prices_a_name() {
    let pricing = "crates/linggen-engine/src/provider/models/pricing.rs";
    let found = lines_with(
        &[".has_unestimated_rates(", ".tiers()", ".base()"],
        |rel| rel == pricing || rel.starts_with(CATALOG_DIR),
        pricing,
    );
    assert!(found.is_empty(), "ask pricing::price_of instead: {found:?}");
    let found = lines_with(&["\"claude-", "\"gpt-", "\"gemini-"], |rel| rel != pricing, pricing);
    assert!(found.is_empty(), "no hand-kept price table: {found:?}");
}

/// A model's kind is decided by `provider::kinds::kind_of`, and the one
/// walk of the config's rows for a list of chat models is
/// `Config::chat_models`. A production file outside the config, the keys,
/// the model registry and the config-save route that walks the raw
/// `models` rows builds a list the live-voice rows can leak into.
#[test]
fn no_file_builds_a_model_list_from_the_raw_config_rows() {
    let walks = [
        ".models.iter()",
        ".models.iter_mut()",
        ".models.into_iter()",
        ".models.first()",
        ".models.last()",
        ".models.values()",
    ];
    let found = lines_with(
        &walks,
        |rel| {
            rel == "crates/linggen-engine/src/config.rs"
                || rel == "crates/linggen-engine/src/credentials.rs"
                || rel == "crates/linggen-engine/src/provider/kinds.rs"
                || rel.starts_with("crates/linggen-engine/src/provider/models/")
                // Saving the config strips and files the keys of every row.
                || rel == "src/server/api/config.rs"
                // Provider edits locate and rewrite filed rows of either kind.
                || rel == "src/server/api/providers.rs"
        },
        "src/cli/status.rs",
    );
    assert!(
        found.is_empty(),
        "ask Config::chat_models() (or the manager's model_ids / list_models) instead:\n{}",
        found.join("\n")
    );
    // The scan sees what it is meant to: a walk, spelled as it is spelled.
    let code = "for m in config.models.iter() {}";
    assert!(walks.iter().any(|w| code.contains(w)));
}

const FAMILIES_DIR: &str = "crates/linggen-engine/src/provider/models/families";

/// A ticked family's binding is written by `families/binding.rs` alone: the
/// row's `family_binding` and the record of started endpoints. Every other
/// file reads them, and the mutating functions are private to the module.
/// A second writer would be a second place deciding what a family row is.
#[test]
fn only_the_binding_writer_writes_a_family_binding() {
    let writer = format!("{FAMILIES_DIR}/binding.rs");
    let found = lines_with(
        &[
            "family_binding =", "family_binding: Some", "family_binding.take", "family_binding.as_mut",
            "family_binding.replace", "family_binding.insert", "family_binding.get_or_insert",
            "FamilyBinding {", "family_starts =", "family_starts.0", "family_starts.push",
            "family_starts.insert", "family_starts.clone_from",
        ],
        |rel| rel == writer || (rel.starts_with(FAMILIES_DIR) && rel.ends_with("/tests.rs")),
        &writer,
    );
    assert!(found.is_empty(), "go through families::binding:\n{}", found.join("\n"));
    // The scan sees what it is meant to: a write, spelled as it is spelled.
    let code = "row.family_binding = Some(binding);";
    assert!(["family_binding ="].iter().any(|w| code.contains(w)));
}

/// The things that change a family row run under the settings write lock:
/// their only callers are the routes and jobs of the three files below, and
/// each of them takes `settings_write` itself.
#[test]
fn families_are_changed_only_by_callers_that_hold_the_settings_write_lock() {
    let callers = [
        "src/server/api/provider_families.rs",
        "src/server/api/providers.rs",
        "src/server/api/config.rs",
    ];
    let found = lines_with(
        &[
            "families.follow(", "families.act(", "families.start_endpoint(", "families.go_back_refused(",
            "Families::keep_bindings(",
        ],
        |rel| callers.contains(&rel) || rel.starts_with(FAMILIES_DIR),
        callers[0],
    );
    assert!(found.is_empty(), "change families only where the settings write lock is held:\n{}", found.join("\n"));
    for file in callers {
        let code = production_code(&["src/server/api"]).into_iter().find(|(rel, _)| rel == file).expect("caller file").1;
        assert!(code.contains("settings_write.lock()"), "{file} calls the families without taking the lock");
    }
}

const CURATED_DIR: &str = "crates/linggen-engine/src/provider/models/curated";

/// Which edition of the model list a person is offered is decided in
/// `curated/edition.rs` alone: the Mac's region is read there, the choice is
/// resolved there, and every other file asks `ModelCatalog::offered()` (or
/// `shown()`). A second `Edition::…` literal, region read or `offered_in` call
/// would be a second place deciding.
#[test]
fn the_offered_edition_is_decided_only_in_the_edition_file() {
    let decider = format!("{CURATED_DIR}/edition.rs");
    let found = lines_with(
        &["Edition::Mainland", "Edition::International", "system_region(", "read_apple_locale(", "region_of_locale(", ".offered_in(", "AppleLocale"],
        // The bundle names its two lists (and tries the offered one first), nothing more.
        |rel| rel == decider || rel == format!("{CURATED_DIR}/bundle.rs") || rel.ends_with("/tests.rs"),
        &decider,
    );
    assert!(found.is_empty(), "ask `config.model_catalog.offered()` instead:\n{}", found.join("\n"));
    // The scan sees what it is meant to: a literal, spelled as it is spelled.
    assert!(["Edition::Mainland"].iter().any(|w| "let e = Edition::Mainland;".contains(w)));
}

/// The owner's choice of edition is written by one route (`providers::set_edition`);
/// every other writer of the config keeps what the engine holds. No file but
/// those builds a `ModelCatalog` with a choice.
#[test]
fn only_the_owners_route_chooses_an_edition() {
    let found = lines_with(
        &["ModelCatalog::chosen(", "model_catalog ="],
        |rel| matches!(rel, "src/server/api/providers.rs" | "src/server/api/config.rs")
            || rel == format!("{CURATED_DIR}/edition.rs") || rel.ends_with("/tests.rs"),
        "src/server/api/providers.rs",
    );
    assert!(found.is_empty(), "a second writer of the edition choice:\n{}", found.join("\n"));
    let config = production_code(&["src/server/api"]).into_iter().find(|(rel, _)| rel == "src/server/api/config.rs").expect("config route").1;
    assert!(config.contains("new_config.model_catalog = current.model_catalog.clone()"), "a saved config must keep the edition choice");
}
