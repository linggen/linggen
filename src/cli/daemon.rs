use crate::config::Config;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn agent_pid_file() -> PathBuf {
    crate::paths::linggen_home().join("ling.pid")
}

fn agent_log_file() -> PathBuf {
    crate::paths::linggen_home().join("ling.log")
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

fn is_process_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn is_port_listening(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

async fn stop_process_by_pid_file(pid_path: &Path, label: &str) -> Result<()> {
    let pid = match fs::read_to_string(pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    {
        Some(p) => p,
        None => {
            println!("{}: no PID file found; may not be running.", label);
            return Ok(());
        }
    };

    if !is_process_running(pid) {
        println!("{}: process {} is not running. Cleaning up PID file.", label, pid);
        let _ = fs::remove_file(pid_path);
        return Ok(());
    }

    // Send SIGTERM
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();

    tokio::time::sleep(Duration::from_millis(500)).await;

    if is_process_running(pid) {
        // Force kill
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let _ = fs::remove_file(pid_path);
    println!("{} stopped (PID {})", label, pid);
    Ok(())
}

// ---------------------------------------------------------------------------
// Agent daemon
// ---------------------------------------------------------------------------

pub async fn start_agent(
    config: &Config,
    port_override: Option<u16>,
    host_override: Option<String>,
    root: Option<PathBuf>,
) -> Result<()> {
    let port = port_override.unwrap_or(config.server.port);

    if is_port_listening(port).await {
        println!("Agent server already running on port {}", port);
        return Ok(());
    }

    let pid_path = agent_pid_file();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("Failed to get current executable path")?;
    let log = agent_log_file();

    let mut args = vec!["--web".to_string(), "--port".to_string(), port.to_string()];
    if let Some(ref h) = host_override {
        args.push("--host".to_string());
        args.push(h.clone());
    }
    if let Some(ref r) = root {
        args.push("--root".to_string());
        args.push(r.display().to_string());
    }

    let log_out = fs::File::create(&log).context("Failed to create daemon log file")?;
    let log_err = log_out.try_clone()?;

    let child = std::process::Command::new(&exe)
        .args(&args)
        .stdout(log_out)
        .stderr(log_err)
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .context("Failed to spawn daemon process")?;

    let pid = child.id();
    fs::write(&pid_path, pid.to_string())?;

    // Poll until ready (30 x 100ms = 3s)
    let mut ready = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if is_port_listening(port).await {
            ready = true;
            break;
        }
    }

    let display_host = host_override.as_deref().unwrap_or("localhost");
    if ready {
        println!("Agent server started on http://{}:{} (PID {})", display_host, port, pid);
    } else {
        println!(
            "Agent server spawned (PID {}) but not yet reachable on port {}",
            pid, port
        );
        println!("Check logs at {}", log.display());
    }

    Ok(())
}

pub async fn stop_agent() -> Result<()> {
    stop_process_by_pid_file(&agent_pid_file(), "Agent server").await
}

