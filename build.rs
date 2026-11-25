// Re-run the build when embedded asset folders change. rust_embed's
// derive only creates cargo dependency edges for files that existed at
// the last macro expansion — a NEW file in an embedded folder doesn't
// trigger a rebuild on its own, so the binary silently ships without it
// (bitten 2026-08-14 by agents/shared/humanize.md). Watching the
// directories closes that hole for agents/, missions/, and ui/dist/.
//
// For a macOS target it also builds `ling-hear`, the helper that hears a clip with Apple's
// on-device speech recognition (src/server/api/apple_speech/); `apple_helper.rs` carries it.
fn main() {
    println!("cargo:rerun-if-changed=agents");
    println!("cargo:rerun-if-changed=missions");
    println!("cargo:rerun-if-changed=ui/dist");
    println!("cargo:rustc-env=LINGGEN_BUILD_COMMIT={}", build_commit());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_hearing_helper();
    }
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

/// Compile the Swift helper for the target's architecture into `$OUT_DIR/ling-hear`, with the
/// usage text embedded and an ad-hoc signature (macOS 26 kills an unsigned or linker-signed
/// binary that is run from somewhere other than where it was built).
///
/// The floor is macOS 12, where the Swift runtime is part of the system. On an older Mac the
/// helper does not start, which stops dictation and nothing else: it is its own process.
fn build_hearing_helper() {
    use std::process::Command;
    let dir = std::path::Path::new("src/server/api/apple_speech");
    println!("cargo:rerun-if-changed={}", dir.display());
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("ling-hear");
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("ling-hear: no Swift target for {other:?}"),
    };
    let built = Command::new("swiftc")
        .args(["-O", "-swift-version", "5", "-target", &format!("{arch}-apple-macos12.0")])
        .args(["-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist", "-Xlinker"])
        .arg(dir.join("Info.plist"))
        .args([dir.join("speech.swift"), dir.join("main.swift")])
        .arg("-o")
        .arg(&out)
        .status()
        .expect("ling-hear: swiftc not found (Xcode or the command line tools build the Mac engine)");
    assert!(built.success(), "ling-hear: swiftc failed");
    let signed = Command::new("codesign")
        .args(["--force", "--sign", "-", "--identifier", "ai.linggen.ling-hear"])
        .arg(&out)
        .status()
        .expect("ling-hear: codesign not found");
    assert!(signed.success(), "ling-hear: codesign failed");
}
