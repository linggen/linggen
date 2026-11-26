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
    println!("cargo:rustc-env=LINGGEN_BUILD_COMMIT={}", build_commit());
}

fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    output.status.success().then_some(())?;
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

fn build_commit() -> String {
    // HEAD can point to a loose or packed ref. Resolve Git's paths rather than
    // assuming .git is a directory (linked worktrees have a .git file).
    let head_ref = git(&["symbolic-ref", "-q", "HEAD"]);
    for name in ["HEAD", "packed-refs"].into_iter().chain(head_ref.as_deref()) {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into())
}
