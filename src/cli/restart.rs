//! `ling restart`: an agent restarts the engine it runs in on a build it just
//! made; afterwards its session, and every session the restart interrupted,
//! continues (doc/background-tasks-spec.md § Engine restart). The engine does
//! the waiting and remembers who to wake; `ling engine-restart` (hidden) is
//! the detached helper it starts to swap the binary and bring it back up.

use super::{engine_restart, self_update};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Ask the engine on `port` to restart on `binary`. The calling session is
/// the one in `LINGGEN_SESSION_ID`, which Linggen sets for its agents' shells.
pub async fn request(port: u16, binary: &Path, wait_secs: u64) -> Result<()> {
    let binary = binary
        .canonicalize()
        .with_context(|| format!("No build at {}", binary.display()))?;
    let session = std::env::var(crate::paths::SESSION_ENV).ok().filter(|s| !s.is_empty());
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/engine/restart"))
        .json(&json!({"binary": binary, "session_id": session, "wait_secs": wait_secs}))
        .timeout(Duration::from_secs(wait_secs + 60))
        .send()
        .await
        .context("The engine is not answering")?;
    let ok = response.status().is_success();
    let body = response.text().await.unwrap_or_default();
    if !ok {
        anyhow::bail!("The engine refused the restart: {body}");
    }
    let reply: Value = serde_json::from_str(&body).unwrap_or_default();
    println!(
        "The engine is restarting on {} (waited {}s for other turns). {} session(s) will continue \
         afterwards{}. Do not run anything else now: this turn ends with the restart, and you are \
         told how it went in the next one.",
        binary.display(),
        reply["waited_secs"].as_u64().unwrap_or(0),
        reply["sessions"].as_u64().unwrap_or(0),
        if session.is_some() { ", this one included" } else { "" },
    );
    Ok(())
}

/// Where the helper says how the last restart went.
pub fn status_file() -> PathBuf {
    crate::paths::linggen_home().join("restart/status.json")
}

fn write_status(state: &str, detail: &str) {
    let path = status_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = json!({"state": state, "detail": detail, "at": crate::util::now_ts_secs()});
    let _ = std::fs::write(path, body.to_string());
}

/// Copy `binary` over `target` the way an update does: the old one kept as
/// `<name>.prev`, the new one kept only if it answers `--version`.
fn install(binary: &Path, target: &Path) -> Result<()> {
    let dir = target.parent().context("binary has no folder")?;
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .context("binary has no name")?;
    let staged = dir.join(format!("{name}.new"));
    std::fs::copy(binary, &staged).with_context(|| format!("Could not stage {}", staged.display()))?;
    self_update::commit_with_probe(&staged, target, &dir.join(format!("{name}.prev")), name, Duration::from_secs(10))
}

/// The detached helper: swap `binary` in where the engine `pid` runs from
/// (and into the CLI install), restart it with its own arguments, roll back
/// to `.prev` when it does not come up healthy.
pub async fn helper(binary: &Path, pid: u32, host: String, port: u16) -> Result<()> {
    let result = async {
        let engine = engine_restart::process_info(pid).context("The engine process is gone")?;
        let target = engine.exe.clone().context("The engine's binary is unknown")?;
        install(binary, &target)?;
        let cli = PathBuf::from("/usr/local/bin/ling");
        if cli.exists() && cli.canonicalize().ok() != target.canonicalize().ok() {
            if let Err(error) = install(binary, &cli) {
                println!("[restart] the CLI copy at {} was left as it was: {error:#}", cli.display());
            }
        }
        let dir = target.parent().context("binary has no folder")?;
        let name = target.file_name().and_then(|n| n.to_str()).context("binary has no name")?;
        engine_restart::restart(dir, name, &[engine], &engine_restart::Opts::new(host, port)).await
    }
    .await;
    match &result {
        Ok(()) => write_status("ok", "the new build is up and healthy"),
        Err(error) if error.to_string().contains("rolled back") => write_status("rolled_back", &format!("{error:#}")),
        Err(error) => write_status("failed", &format!("{error:#}")),
    }
    result
}

/// Start the helper detached from the engine (its own process group), so it
/// outlives the engine it stops.
pub fn spawn_helper(binary: &Path, host: &str, port: u16) -> Result<()> {
    let exe = std::env::current_exe().context("Cannot find the running binary")?;
    let log = crate::paths::linggen_home().join("restart/helper.log");
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let out = std::fs::OpenOptions::new().create(true).append(true).open(&log)?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("engine-restart")
        .arg("--binary")
        .arg(binary)
        .arg("--pid")
        .arg(std::process::id().to_string())
        .arg("--host")
        .arg(host)
        .arg("--port")
        .arg(port.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().context("Could not start the restart helper")?;
    Ok(())
}
