// Re-run the build when an embedded folder (places, the built-in missions)
// changes: rust_embed's
// derive only creates cargo dependency edges for files that existed at the
// last macro expansion, so a NEW file in it would not trigger a rebuild on
// its own (the same hole the `ling` package's build.rs closes for agents/,
// missions/ and ui/dist/).
fn main() {
    println!("cargo:rerun-if-changed=../../agents/places");
    println!("cargo:rerun-if-changed=../../missions");
}
