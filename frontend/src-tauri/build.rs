fn main() {
    // Ensure the `linggen-server` sidecar exists locally for dev builds.
    prepare_sidecar_binary("linggen-server");
    tauri_build::build()
}

fn prepare_sidecar_binary(bin_name: &str) {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::SystemTime;

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let manifest_dir = PathBuf::from(manifest_dir);

    // Get target triple from Cargo
    let target_triple = std::env::var("TARGET")
        .or_else(|_| std::env::var("TAURI_ENV_TARGET_TRIPLE"))
        .expect("Could not determine TARGET triple");

    // IMPORTANT:
    // We always build the backend sidecar in *release* mode, even when the desktop app
    // is built in debug (tauri dev). This avoids missing `backend/target/<triple>/debug/...`
    // and gives a consistent, fast sidecar.
    let backend_profile = "release";

    // Repo root: <repo>/frontend/src-tauri -> parent (frontend) -> parent (repo)
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("Could not determine repo root");

    let sidecar_filename = format!("{bin_name}-{target_triple}");
    // NOTE: despite some Tauri v2 docs mentioning `binaries/`, tauri-build in our
    // current setup validates the sidecar exists at `src-tauri/<name>-<target>`.
    // We'll copy to BOTH locations to keep compatibility.
    let dest_root = manifest_dir.join(&sidecar_filename);
    let binaries_dir = manifest_dir.join("binaries");
    let dest_binaries = binaries_dir.join(&sidecar_filename);

    // Tell Cargo when to re-run this build script
    println!("cargo:rerun-if-changed=../../backend/api/src");
    println!("cargo:rerun-if-changed=../../backend/api/Cargo.toml");
    println!("cargo:rerun-if-changed=../../backend/Cargo.toml");
    println!("cargo:rerun-if-changed=../../backend/storage/src");
    println!("cargo:rerun-if-changed=../../backend/core/src");
    // Allow forcing a rebuild by toggling this env var.
    println!("cargo:rerun-if-env-changed=LINGGEN_SIDECAR_FORCE_BUILD");
    // Allow skipping the sidecar build for faster iteration (only works if the sidecar already exists).
    println!("cargo:rerun-if-env-changed=LINGGEN_SKIP_SIDECAR_BUILD");

    let forced = std::env::var("LINGGEN_SIDECAR_FORCE_BUILD").is_ok();
    let skip = std::env::var("LINGGEN_SKIP_SIDECAR_BUILD")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    if skip && !forced && dest_root.exists() {
        return;
    }

    let backend_dir = repo_root.join("backend");
    let backend_manifest = backend_dir.join("Cargo.toml");

    fn newest_mtime(path: &Path) -> Option<SystemTime> {
        let meta = std::fs::metadata(path).ok()?;
        let mut newest = meta.modified().ok();
        if meta.is_dir() {
            let mut stack = vec![path.to_path_buf()];
            while let Some(dir) = stack.pop() {
                let entries = match std::fs::read_dir(&dir) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let p = entry.path();
                    let m = match entry.metadata().and_then(|m| m.modified()) {
                        Ok(m) => Some(m),
                        Err(_) => None,
                    };
                    if let (Some(n), Some(m)) = (newest, m) {
                        if m > n {
                            newest = Some(m);
                        }
                    } else if newest.is_none() {
                        newest = m;
                    }

                    if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                        stack.push(p);
                    }
                }
            }
        }
        newest
    }

    // Keep the two expected destinations in sync without rebuilding.
    // (tauri-build currently validates the root path, while packaging uses binaries/)
    if !dest_root.exists() && dest_binaries.exists() {
        let _ = std::fs::copy(&dest_binaries, &dest_root);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&dest_root) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(&dest_root, perms);
            }
        }
        return;
    }
    if dest_root.exists() && !dest_binaries.exists() {
        let _ = std::fs::create_dir_all(&binaries_dir);
        let _ = std::fs::copy(&dest_root, &dest_binaries);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&dest_binaries) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(&dest_binaries, perms);
            }
        }
        return;
    }

    // Only rebuild the sidecar when backend sources are newer than the existing sidecar.
    let sidecar_mtime = std::fs::metadata(&dest_root)
        .ok()
        .and_then(|m| m.modified().ok());
    let mut backend_newest: Option<SystemTime> = None;
    for p in [
        backend_dir.join("api/src"),
        backend_dir.join("storage/src"),
        backend_dir.join("core/src"),
        backend_dir.join("api/Cargo.toml"),
        backend_dir.join("Cargo.toml"),
    ] {
        if let Some(m) = newest_mtime(&p) {
            backend_newest = Some(match backend_newest {
                Some(cur) if cur > m => cur,
                _ => m,
            });
        }
    }

    let needs_rebuild = forced
        || sidecar_mtime.is_none()
        || backend_newest
            .and_then(|bn| sidecar_mtime.map(|sm| bn > sm))
            .unwrap_or(true);

    if !needs_rebuild {
        return;
    }

    // 1. Build the backend (release, target-specific)
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("--manifest-path")
        .arg(&backend_manifest)
        .arg("--bin")
        .arg(bin_name)
        .arg("--target")
        .arg(&target_triple)
        .arg("--release");

    // Isolate from parent cargo flags
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env_remove("RUSTDOC")
        .env_remove("CARGO_MAKEFLAGS");

    let status = cmd.status().expect("Failed to run backend build command");
    if !status.success() {
        panic!("Backend build failed with status: {}", status);
    }

    // 2. Find and copy the built binary
    let built = backend_dir
        .join("target")
        .join(&target_triple)
        .join(backend_profile)
        .join(bin_name);

    if !built.exists() {
        panic!(
            "Backend build succeeded but binary not found at: {}",
            built.display()
        );
    }

    // Copy to BOTH locations:
    // - src-tauri/<name>-<target> (what tauri-build currently validates)
    // - src-tauri/binaries/<name>-<target> (what our packaging script uses)
    let _ = std::fs::create_dir_all(&binaries_dir);
    std::fs::copy(&built, &dest_root).expect("Failed to copy sidecar to src-tauri root");
    std::fs::copy(&built, &dest_binaries).expect("Failed to copy sidecar to src-tauri/binaries");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for d in [&dest_root, &dest_binaries] {
            if let Ok(meta) = std::fs::metadata(d) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(d, perms);
            }
        }
    }
}
