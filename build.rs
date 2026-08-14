// Re-run the build when embedded asset folders change. rust_embed's
// derive only creates cargo dependency edges for files that existed at
// the last macro expansion — a NEW file in an embedded folder doesn't
// trigger a rebuild on its own, so the binary silently ships without it
// (bitten 2026-08-14 by agents/shared/humanize.md). Watching the
// directories closes that hole for agents/, missions/, and ui/dist/.
fn main() {
    println!("cargo:rerun-if-changed=agents");
    println!("cargo:rerun-if-changed=missions");
    println!("cargo:rerun-if-changed=ui/dist");
}
