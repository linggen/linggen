//! The test embedder never reaches a build that ships.
//!
//! It is compiled only under `cfg(test)` or the `test-support` feature, so
//! the proof is feature resolution: in a normal build (`-e normal,build`)
//! neither `ling` nor memory itself turns `test-support` on, while the
//! dev-dependency edges that this suite and the engine's tests use do (the
//! positive control — without it the negative would pass vacuously).

use std::process::Command;

/// The features cargo resolves for linggen-memory when building `package`
/// over the given dependency edges, e.g. `["default", "telemetry"]`.
fn memory_features(package: &str, edges: &str) -> Vec<String> {
    let out = Command::new(env!("CARGO"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["tree", "--offline", "-p", package, "-e", edges])
        .args(["-i", "linggen-memory", "--depth", "0", "-f", "[{f}]"])
        .output()
        .expect("running cargo tree");
    assert!(
        out.status.success(),
        "cargo tree: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let inside = text
        .trim()
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or_else(|| panic!("unexpected cargo tree line: {text:?}"));
    inside
        .split(',')
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn a_shipping_build_has_no_test_embedder() {
    for package in ["linggen", "linggen-memory"] {
        let shipped = memory_features(package, "normal,build");
        assert!(
            !shipped.iter().any(|f| f == "test-support"),
            "{package} ships memory with {shipped:?}"
        );
    }
    let tested = memory_features("linggen-memory", "all");
    assert!(
        tested.iter().any(|f| f == "test-support"),
        "this suite runs with the test embedder: {tested:?}"
    );
}
