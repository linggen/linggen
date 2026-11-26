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

/// No engine code reads a model list shipped with it, or prices a key
/// provider's call: there is no list file, no price table and no price
/// lookup, and the words that would bring one back are named nowhere.
/// (Nobody at Linggen maintains a model list: doc/models.md.)
#[test]
fn no_shipped_model_list_and_no_price_for_a_key_provider() {
    let found = lines_with(
        &[
            "model-catalog", "include_bytes!(\"model", "/api/model-price", "price_lookup", "spend_cap",
            "ModelPrice", "fn price_of(", "cost_usd: Some(",
        ],
        |_| false,
        "crates/linggen-engine/src/provider/models/pricing.rs",
    );
    assert!(found.is_empty(), "a shipped model list or a price for a key provider:\n{}", found.join("\n"));
}

/// Linggen Cloud's models are the site's (`GET /api/cloud/models`): no
/// engine file names one but the first-run starter and the retired ids
/// (`provider/models/catalog.rs`) — tests and stand-ins aside.
#[test]
fn linggen_cloud_ids_are_named_only_by_the_site() {
    let found = lines_with(
        &["\"qwen4-max\"", "\"glm-5.3-prime\"", "\"kimi-k3.1-pro\"", "\"qwen3.8-flash\"", "\"deepseek-v4.1-flash\""],
        |rel| rel == "crates/linggen-engine/src/provider/models/catalog.rs",
        "crates/linggen-engine/src/provider/models/cloud_models.rs",
    );
    assert!(found.is_empty(), "a Linggen Cloud model id in engine code:\n{}", found.join("\n"));
}

/// The providers the picker offers are one table (`provider/models/presets.rs`):
/// no other file names a key provider's address.
#[test]
fn provider_addresses_live_in_the_presets_table() {
    let presets = "crates/linggen-engine/src/provider/models/presets.rs";
    let found = lines_with(
        &["api.deepseek.com", "openrouter.ai/api/v1", "api.groq.com"],
        |rel| rel == presets,
        presets,
    );
    assert!(found.is_empty(), "a provider address outside the presets table:\n{}", found.join("\n"));
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
