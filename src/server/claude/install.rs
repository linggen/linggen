use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::sync::Mutex;

pub(super) const VERSION: &str = "0.3.309";
const INSTALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
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
    write_asset(dir.join("package.json"), manifest).await?;
    write_asset(
        dir.join("bun.lock"),
        include_str!("../../../runtimes/claude-agent-sdk/bun.lock"),
    )
    .await?;
    write_asset(
        dir.join("subagents.ts"),
        include_str!("../../../runtimes/claude-agent-sdk/subagents.ts"),
    )
    .await?;
    write_asset(
        dir.join("runner.ts"),
        include_str!("../../../runtimes/claude-agent-sdk/runner.ts"),
    )
    .await?;
    if dir.join("installed").is_file()
        && dir
            .join("node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs")
            .is_file()
        && dir.join("node_modules/zod/package.json").is_file()
    {
        return Ok(dir.join("runner.ts"));
    }
    let install = Command::new(bun_path()?)
        .args([
            "install",
            "--production",
            "--ignore-scripts",
            "--frozen-lockfile",
        ])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(INSTALL_TIMEOUT, install)
        .await
        .context("Claude Agent SDK installation timed out")??;
    if !output.status.success() {
        bail!(
            "Claude Agent SDK installation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    tokio::fs::write(dir.join("installed"), VERSION).await?;
    Ok(dir.join("runner.ts"))
}

// Concurrent capability probes and chat turns must never read a half-written
// runner. Unchanged assets need no write; upgrades replace a complete inode.
async fn write_asset(path: PathBuf, content: &str) -> Result<()> {
    if tokio::fs::read(&path).await.ok().as_deref() == Some(content.as_bytes()) {
        return Ok(());
    }
    let staged = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&staged, content).await?;
    tokio::fs::rename(staged, path).await?;
    Ok(())
}
