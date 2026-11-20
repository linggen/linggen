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
