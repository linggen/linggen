//! After a rollback swaps in a previous engine: if that engine does not host
//! memory itself, the old `ling-mem` program has to be back in place before
//! it starts (`memory_host::restore_previous`) — it starts memory by running
//! `ling-mem start`, which is a no-op in the stand-in.
//!
//! Whether an engine hosts memory is asked of the binary: only a build with
//! the `mem` subcommand answers `mem --help` with success; an older build
//! rejects `mem` as an unknown subcommand. Version numbers say nothing here
//! (local builds of both shapes carry the same one).

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// Whether `engine` hosts memory: `engine mem --help` exits 0 within `timeout`.
/// The help text is printed and nothing else happens.
pub fn hosts_memory(engine: &Path, timeout: Duration) -> bool {
    let Ok(mut child) = std::process::Command::new(engine)
        .args(["mem", "--help"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// The rollback step: ask the engine now at `engine` whether it hosts
/// memory, then put the old `ling-mem` back at `ling_mem` if it does not and
/// the stand-in is there. Prints the one line that says what happened.
pub fn restore_after_rollback(engine: &Path, ling_mem: Option<&Path>, timeout: Duration) {
    if ling_mem.is_none() {
        return;
    }
    let hosts = hosts_memory(engine, timeout);
    if let Some(line) = crate::memory_host::restore_previous(hosts, ling_mem) {
        println!("  {line}");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const T: Duration = Duration::from_secs(10);

    fn script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn an_engine_hosts_memory_when_mem_help_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let new = dir.path().join("new");
        script(&new, r#"[ "$1" = mem ] && [ "$2" = --help ] && exit 0; exit 2"#);
        let old = dir.path().join("old");
        script(&old, r#"echo "error: unrecognized subcommand '$1'" >&2; exit 2"#);
        assert!(hosts_memory(&new, T));
        assert!(!hosts_memory(&old, T));
        assert!(!hosts_memory(&dir.path().join("missing"), T));
    }

    #[test]
    fn an_engine_that_does_not_answer_in_time_is_not_taken_to_host_memory() {
        let dir = tempfile::tempdir().unwrap();
        let slow = dir.path().join("slow");
        script(&slow, "sleep 30");
        let started = Instant::now();
        assert!(!hosts_memory(&slow, Duration::from_millis(300)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
