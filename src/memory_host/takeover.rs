//! Taking the store over from an old `ling-mem` daemon at start-up.
//!
//! Before this version the store was served by a separate `ling-mem`
//! process that wrote `memory/linggen-memory/daemon.json`. It takes no lock,
//! so before the engine opens the store a still-running daemon is stopped:
//! only when that file names a live process whose executable is a
//! `ling-mem` binary, never anything else and never this process. SIGTERM,
//! up to [`STOP_WAIT`] for the pid to go and its port to free, then SIGKILL.
//! The file is removed afterwards; nothing here ever writes it (an old
//! `ling-mem stop` would SIGTERM whatever pid it names).

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const STOP_WAIT: Duration = Duration::from_secs(10);
const KILL_WAIT: Duration = Duration::from_secs(3);

#[derive(Deserialize)]
struct PidFile {
    pid: u32,
    #[serde(default)]
    port: Option<u16>,
}

fn pidfile_path(data_dir: &Path) -> PathBuf {
    data_dir
        .join("memory")
        .join("linggen-memory")
        .join("daemon.json")
}

/// Stop an old daemon serving this store, if one runs. `Err` only when one
/// is running and would not stop: the store must not get a second writer.
pub(super) async fn stop_old_daemon(data_dir: &Path) -> Result<(), String> {
    let path = pidfile_path(data_dir);
    let Some(file) = read_pidfile(&path) else {
        return Ok(());
    };
    if file.pid != std::process::id() && is_ling_mem(file.pid) {
        tracing::info!(
            "memory: stopping the old ling-mem daemon (pid {}) to take its store over",
            file.pid
        );
        stop(file.pid, file.port).await?;
    }
    // Stale now either way: the daemon is gone, or the pid was never one.
    let _ = std::fs::remove_file(&path);
    Ok(())
}

fn read_pidfile(path: &Path) -> Option<PidFile> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

async fn stop(pid: u32, port: Option<u16>) -> Result<(), String> {
    signal(pid, libc::SIGTERM);
    if gone_within(pid, port, STOP_WAIT).await {
        return Ok(());
    }
    tracing::warn!("memory: ling-mem pid {pid} ignored SIGTERM for {STOP_WAIT:?}; SIGKILL");
    signal(pid, libc::SIGKILL);
    if gone_within(pid, port, KILL_WAIT).await {
        return Ok(());
    }
    Err(format!(
        "the old ling-mem daemon (pid {pid}) did not stop; not opening its store beside it"
    ))
}

async fn gone_within(pid: u32, port: Option<u16>, wait: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < wait {
        if !alive(pid) && port.is_none_or(port_is_free) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

fn signal(pid: u32, sig: libc::c_int) {
    // SAFETY: kill(2) on a pid checked to be a ling-mem binary.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(200),
    )
    .is_err()
}

/// Whether `pid` runs a `ling-mem` executable (a copy named `ling-mem-…`
/// counts). A shell script, such as the stand-in, runs its interpreter.
fn is_ling_mem(pid: u32) -> bool {
    exe_name(pid).is_some_and(|name| name.starts_with("ling-mem"))
}

#[cfg(target_os = "macos")]
fn exe_name(pid: u32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    // SAFETY: the buffer is as long as the size passed.
    let n = unsafe {
        libc::proc_pidpath(
            pid as libc::c_int,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as u32,
        )
    };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    let path = PathBuf::from(String::from_utf8(buf).ok()?);
    Some(path.file_name()?.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "macos"))]
fn exe_name(pid: u32) -> Option<String> {
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(path.file_name()?.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_pidfile_naming_something_else_is_dropped_and_nothing_signalled() {
        let dir = tempfile::tempdir().unwrap();
        let path = pidfile_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        std::fs::write(&path, format!(r#"{{"pid":{},"port":1}}"#, child.id())).unwrap();

        stop_old_daemon(dir.path()).await.unwrap();

        assert!(child.try_wait().unwrap().is_none(), "sleep was signalled");
        assert!(!path.exists(), "the stale pidfile stays");
        let _ = child.kill();
    }

    #[test]
    fn this_process_is_not_a_ling_mem() {
        assert!(!is_ling_mem(std::process::id()));
        assert!(exe_name(std::process::id()).is_some());
    }
}
