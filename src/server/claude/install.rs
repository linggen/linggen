use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::sync::Mutex;

pub(super) const VERSION: &str = "0.3.309";
static INSTALL_LOCK: Mutex<()> = Mutex::const_new(());

pub(super) fn runtime_dir() -> PathBuf {
    crate::paths::linggen_home()
        .join("runtimes/claude-agent-sdk")
        .join(VERSION)
}

pub(super) fn bun_path() -> Result<PathBuf> {
    let bundled = crate::paths::linggen_home().join("bin/bun");
    if bundled.is_file() {
        return Ok(bundled);
    }
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|path| path.join("bun"))
                .find(|path| path.is_file())
        })
        .context("Bun is missing. Reinstall Linggen to restore its runtime.")
}

/// Install once on first use, serialized across agents. A marker is written
/// only after Bun succeeded; an interrupted download is retried next time.
pub(super) async fn ensure_installed() -> Result<PathBuf> {
    let _guard = INSTALL_LOCK.lock().await;
    let dir = runtime_dir();
    tokio::fs::create_dir_all(&dir).await?;
    let manifest = include_str!("../../../runtimes/claude-agent-sdk/package.json");
    tokio::fs::write(dir.join("package.json"), manifest).await?;
    tokio::fs::write(
        dir.join("bun.lock"),
        include_str!("../../../runtimes/claude-agent-sdk/bun.lock"),
    )
    .await?;
    tokio::fs::write(
        dir.join("runner.ts"),
        include_str!("../../../runtimes/claude-agent-sdk/runner.ts"),
    )
    .await?;
    if dir.join("installed").is_file()
        && dir
            .join("node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs")
            .is_file()
    {
        return Ok(dir.join("runner.ts"));
    }
    let output = Command::new(bun_path()?)
        .args([
            "install",
            "--production",
            "--ignore-scripts",
            "--frozen-lockfile",
        ])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "Claude Agent SDK installation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    tokio::fs::write(dir.join("installed"), VERSION).await?;
    Ok(dir.join("runner.ts"))
}
