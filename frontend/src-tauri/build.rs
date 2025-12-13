fn main() {
    // Ensure the `linggen-server` sidecar exists locally for dev builds.
    //
    // We do NOT commit sidecar binaries to git (they are large), but Tauri's
    // `bundle.externalBin` requires the per-target file to be present at build time,
    // e.g. `src-tauri/binaries/linggen-server-aarch64-apple-darwin`.
    //
    // This hook builds the backend server (debug/release depending on PROFILE)
    // and copies it into `binaries/` if missing.
    prepare_sidecar_binary("linggen-server");
    tauri_build::build()
}

fn prepare_sidecar_binary(bin_name: &str) {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let manifest_dir = PathBuf::from(manifest_dir);

    // Cargo always provides TARGET (build target triple) for build scripts.
    // Tauri also *injects* TAURI_ENV_TARGET_TRIPLE later via cargo:rustc-env,
    // which is not visible to this build script, so we must rely on TARGET.
    let target_triple = std::env::var("TARGET")
        .or_else(|_| std::env::var("TAURI_ENV_TARGET_TRIPLE"))
        .unwrap_or_default();
    if target_triple.is_empty() {
        return;
    }

    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());

    // Repo root: <repo>/frontend/src-tauri -> parent (frontend) -> parent (repo)
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let Some(repo_root) = repo_root else {
        return;
    };

    let binaries_dir = manifest_dir.join("binaries");
    let dest = binaries_dir.join(format!("{bin_name}-{target_triple}"));

    let dest_exists = dest.exists();
    let debug = std::env::var("LINGGEN_SIDECAR_DEBUG")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    if debug {
        // Helpful diagnostics (shows up in cargo output).
        println!(
            "cargo:warning=[sidecar] manifest_dir={} target={} profile={} dest={} dest_exists={}",
            manifest_dir.display(),
            target_triple,
            profile,
            dest.display(),
            dest_exists
        );
        let alt = manifest_dir.join(format!("{bin_name}-{target_triple}"));
        println!(
            "cargo:warning=[sidecar] alt_expected={} alt_exists={}",
            alt.display(),
            alt.exists()
        );
    }

    if dest_exists {
        return;
    }

    let backend_dir = repo_root.join("backend");
    let backend_manifest = backend_dir.join("Cargo.toml");
    let built = backend_dir.join("target").join(&profile).join(bin_name);

    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("--manifest-path")
        .arg(backend_manifest);
    if profile == "release" {
        cmd.arg("--release");
    }
    cmd.arg("--bin").arg(bin_name);

    match cmd.status() {
        Ok(s) if s.success() => {}
        _ => {
            // Don't hard-fail the desktop build; tauri_build::build() will emit a clearer
            // error if the sidecar still doesn't exist.
            return;
        }
    }

    if !built.exists() {
        return;
    }

    let _ = std::fs::create_dir_all(&binaries_dir);
    let _ = std::fs::copy(&built, &dest);

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&dest) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&dest, perms);
        }
    }
}
