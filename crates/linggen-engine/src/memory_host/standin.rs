//! The `ling-mem` stand-in: memory is part of the engine, but plugin hooks
//! and scripts of sessions already running still call the `ling-mem`
//! binary at the path the old installers used (`$HOME/.local/bin/ling-mem`).
//!
//! Once the engine serves the store, a real `ling-mem` binary found there
//! (Mach-O or ELF) is moved aside as `ling-mem.prev` — kept for a rollback;
//! an older `.prev` is renamed with a timestamp, never overwritten — and a
//! small shell script takes its place:
//!
//! - data verbs run as `ling mem "$@"` through the stable command link when
//!   it points at this engine, otherwise through the writing engine itself;
//! - `--version` answers this engine's version, so the plugin's
//!   `install-bin.sh` (which replaces only an older binary) leaves it alone;
//! - `status` answers what the old autostart hook reads from a running
//!   daemon at that version (`state`, `version`, `port` — the engine's own
//!   port), so the hook neither starts nor restarts anything;
//! - `serve`, `start`, `stop`, `restart`, `upgrade` do nothing, and say so.
//!
//! A rollback to an engine that does not host memory puts the kept binary
//! back over the stand-in (`restore_previous`): that engine starts memory by
//! running `ling-mem start`.
//!
//! Where there is no `ling-mem` at all the stand-in is written too: a plugin
//! from before 2.3 would otherwise install the old daemon and start it beside
//! the engine. A stand-in that differs from what this engine writes (an older
//! version, another port) is rewritten; any other script is left.

use crate::util::set_executable;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Marks a file as ours (and which engine version wrote it).
const MARK: &str = "# linggen-ling-mem-stand-in";

/// The path the old installers and the engine's old resolver used.
fn canonical_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/bin/ling-mem"))
}

/// Where the old installers put `ling-mem` (and where the stand-in goes):
/// `$HOME/.local/bin/ling-mem`.
pub fn ling_mem_path() -> Option<PathBuf> {
    canonical_path()
}

/// Put the stand-in in place, if a real `ling-mem` binary is there; its
/// `status` names `port`, the engine's. Best effort: a failure is logged and
/// changes nothing else.
pub(super) fn install(port: u16) {
    let Some(path) = canonical_path() else {
        return;
    };
    let engine = crate::util::resolved_current_exe()
        .map(|exe| engine_command(&exe, std::env::split_paths(crate::util::shell_path())))
        .unwrap_or_else(|_| PathBuf::from("ling"));
    match install_at(&path, &engine, super::VERSION, port) {
        Ok(Some(what)) => tracing::info!("memory: {what}"),
        Ok(None) => {}
        Err(e) => tracing::warn!("memory: the ling-mem stand-in was not written: {e}"),
    }
}

/// Keep the link itself, not its target, so the stand-in survives app swaps.
fn engine_command(exe: &Path, dirs: impl Iterator<Item = PathBuf>) -> PathBuf {
    dirs.filter_map(|dir| std::path::absolute(dir.join("ling")).ok())
        .find(|candidate| {
            candidate.is_symlink()
                && candidate.canonicalize().is_ok_and(|target| target == exe)
        })
        .unwrap_or_else(|| exe.to_path_buf())
}

#[cfg(all(test, unix))]
#[test]
fn standin_prefers_a_link_to_this_engine_only() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("engine");
    std::fs::write(&exe, "engine").unwrap();
    let exe = exe.canonicalize().unwrap();
    let link = dir.path().join("ling");
    std::os::unix::fs::symlink("missing", &link).unwrap();
    let path = || std::iter::once(dir.path().to_path_buf());
    assert_eq!(engine_command(&exe, path()), exe);
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("engine", &link).unwrap();
    assert_eq!(engine_command(&exe, path()), link);
}

/// What is at a `ling-mem` path.
#[derive(Debug, PartialEq)]
enum Found {
    Nothing,
    Binary,
    StandIn { version: String },
    Other,
}

fn inspect(path: &Path) -> std::io::Result<Found> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Found::Nothing),
        Err(e) => return Err(e),
    };
    if !meta.is_file() {
        return Ok(Found::Other);
    }
    let head = read_head(path)?;
    if is_executable_format(&head) {
        return Ok(Found::Binary);
    }
    let text = String::from_utf8_lossy(&head);
    let version = text
        .lines()
        .find_map(|l| l.strip_prefix(MARK))
        .and_then(|rest| rest.trim().strip_prefix("v="))
        .map(str::to_string);
    Ok(match version {
        Some(version) => Found::StandIn { version },
        None => Found::Other,
    })
}

fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut head = vec![0u8; 256];
    let n = std::fs::File::open(path)?.read(&mut head)?;
    head.truncate(n);
    Ok(head)
}

/// Mach-O (thin or fat, either byte order) or ELF.
fn is_executable_format(head: &[u8]) -> bool {
    const MAGICS: [[u8; 4]; 7] = [
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xfe, 0xed, 0xfa, 0xce],
        [0xca, 0xfe, 0xba, 0xbe],
        [0xbe, 0xba, 0xfe, 0xca],
        [0x7f, b'E', b'L', b'F'],
    ];
    head.len() >= 4 && MAGICS.iter().any(|m| head[..4] == m[..])
}

/// The install itself, on an explicit path. `Ok(Some(..))` says what changed.
fn install_at(
    path: &Path,
    engine: &Path,
    version: &str,
    port: u16,
) -> std::io::Result<Option<String>> {
    match inspect(path)? {
        Found::Other => Ok(None),
        // Nothing there yet: put the stand-in down anyway. A plugin from
        // before 2.3 installs and starts the old daemon when it finds no
        // `ling-mem` — and that daemon knows nothing of the store's lock.
        Found::Nothing => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            write_standin(path, engine, version, port)?;
            Ok(Some(format!(
                "ling-mem stand-in written at {} (no old binary was there)",
                path.display()
            )))
        }
        // Ours, word for word what this engine would write: nothing to do.
        // Any difference (another version, port or engine path) rewrites it.
        Found::StandIn { .. } if is_current(path, engine, version, port) => Ok(None),
        Found::StandIn { .. } => {
            write_standin(path, engine, version, port)?;
            Ok(Some(format!(
                "ling-mem stand-in at {} updated",
                path.display()
            )))
        }
        Found::Binary => {
            let prev = keep_aside(path)?;
            write_standin(path, engine, version, port)?;
            Ok(Some(format!(
                "ling-mem at {} is now a stand-in for `ling mem`; the old binary is {}",
                path.display(),
                prev.display()
            )))
        }
    }
}

/// Whether the stand-in at `path` is exactly the script this engine writes.
fn is_current(path: &Path, engine: &Path, version: &str, port: u16) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| text == script(engine, version, port))
}

/// Move the binary to `ling-mem.prev`, first renaming an older `.prev`
/// with a timestamp so no kept binary is lost.
fn keep_aside(path: &Path) -> std::io::Result<PathBuf> {
    let prev = path.with_file_name("ling-mem.prev");
    if std::fs::symlink_metadata(&prev).is_ok() {
        let stamp = chrono::Local::now().format("%Y%m%d%H%M%S");
        std::fs::rename(&prev, path.with_file_name(format!("ling-mem.prev.{stamp}")))?;
    }
    std::fs::rename(path, &prev)?;
    Ok(prev)
}

/// What a rollback does about the `ling-mem` program.
#[derive(Debug, PartialEq)]
pub enum Restore {
    /// The engine that comes back hosts memory itself: nothing to do.
    TargetHostsMemory,
    /// The stand-in is not at the path: whatever is there stays.
    NoStandIn,
    /// There is no executable `ling-mem.prev` to put back.
    NoPrevious,
    /// Put `ling-mem.prev` back over the stand-in.
    Yes,
}

/// Whether a rollback to an engine puts the old `ling-mem` back, from what
/// is known: does that engine host memory, what is at the `ling-mem` path,
/// what is at `ling-mem.prev`.
fn restore_decision(target_hosts_memory: bool, at_path: &Found, prev: &Found) -> Restore {
    if target_hosts_memory {
        return Restore::TargetHostsMemory;
    }
    if !matches!(at_path, Found::StandIn { .. }) {
        return Restore::NoStandIn;
    }
    if *prev != Found::Binary {
        return Restore::NoPrevious;
    }
    Restore::Yes
}

/// A file that is a real program and may be run; anything else reads as `Other`.
fn inspect_runnable(path: &Path) -> std::io::Result<Found> {
    let found = inspect(path)?;
    if found == Found::Binary && !is_executable_file(path) {
        return Ok(Found::Other);
    }
    Ok(found)
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(_path: &Path) -> bool {
    true
}

/// After a rollback to an engine that starts memory by running `ling-mem
/// start`: put the old program back where the stand-in is. Returns the one
/// plain line to print (`None` when the engine hosts memory itself or the
/// path is unknown).
pub fn restore_previous(target_hosts_memory: bool, path: Option<&Path>) -> Option<String> {
    let path = path?;
    match restore_previous_at(path, target_hosts_memory) {
        Ok(line) => line,
        Err(e) => Some(format!(
            "memory: the previous ling-mem was not put back ({e}); an engine of this version needs it at {}",
            path.display()
        )),
    }
}

fn restore_previous_at(path: &Path, target_hosts_memory: bool) -> std::io::Result<Option<String>> {
    if target_hosts_memory {
        return Ok(None);
    }
    let prev = path.with_file_name("ling-mem.prev");
    let decision = restore_decision(
        target_hosts_memory,
        &inspect(path)?,
        &inspect_runnable(&prev)?,
    );
    Ok(match decision {
        Restore::TargetHostsMemory => None,
        Restore::NoStandIn => Some(format!(
            "memory: {} is not the stand-in; left as it is",
            path.display()
        )),
        Restore::NoPrevious => Some(format!(
            "memory: no ling-mem.prev to put back beside {}; left as it is",
            path.display()
        )),
        Restore::Yes => {
            // One rename: the stand-in's file goes (a script, written again
            // by an engine that hosts memory), the old program is never
            // missing from the path, and nothing is copied.
            std::fs::rename(&prev, path)?;
            Some(format!(
                "memory: the previous ling-mem is back in place at {}",
                path.display()
            ))
        }
    })
}

/// Write the script beside `path` and rename it over — a new inode, never
/// an overwrite of a file something may be running.
fn write_standin(path: &Path, engine: &Path, version: &str, port: u16) -> std::io::Result<()> {
    let tmp = path.with_file_name(format!(".ling-mem.standin.{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(script(engine, version, port).as_bytes())?;
    }
    set_executable(&tmp)?;
    std::fs::rename(&tmp, path)
}

/// A value for the script, single-quoted for sh.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn script(engine: &Path, version: &str, port: u16) -> String {
    let engine = sh_quote(&engine.to_string_lossy());
    format!(
        r#"#!/bin/sh
{MARK} v={version}
# Memory is part of Linggen now. This stands in for the old ling-mem binary
# (kept beside it as ling-mem.prev): data verbs run as `ling mem`. Written by
# the Linggen engine; it goes once the plugins call `ling mem` themselves.
VERSION='{version}'
ENGINE={engine}
verb=""
skip=0
for a in "$@"; do
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  case "$a" in
    --data-dir|--format) skip=1 ;;
    -V|--version) echo "ling-mem $VERSION"; exit 0 ;;
    -*) ;;
    *) verb="$a"; break ;;
  esac
done
case "$verb" in
  status)
    printf '{{"state":"running","version":"%s","port":{port},"host":"linggen"}}\n' "$VERSION"
    exit 0 ;;
  serve|start|stop|restart|upgrade|self-update)
    echo "ling-mem: memory is part of Linggen now; '$verb' does nothing (ling status)" >&2
    exit 0 ;;
esac
# The engine that wrote this knows `mem`; a `ling` on PATH may be an older one that does not.
if [ -x "$ENGINE" ]; then exec "$ENGINE" mem "$@"; fi
exec ling mem "$@"
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh(path: &Path, args: &[&str], path_env: &str) -> (bool, String, String) {
        let out = Command::new("sh")
            .arg(path)
            .args(args)
            .env("PATH", path_env)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    #[test]
    fn a_real_binary_is_kept_aside_and_the_stand_in_answers_the_old_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("ling-mem");
        std::fs::write(&bin, [0xcf, 0xfa, 0xed, 0xfe, 1, 2, 3]).unwrap();
        std::fs::write(dir.path().join("ling-mem.prev"), b"older").unwrap();
        // A fake engine that prints what it was asked.
        let engine = dir.path().join("fake ling");
        std::fs::write(&engine, "#!/bin/sh\necho \"engine: $*\"\n").unwrap();
        set_executable(&engine).unwrap();

        let changed = install_at(&bin, &engine, "2.3.0", 9527).unwrap();
        assert!(changed.is_some());
        assert_eq!(
            std::fs::read(dir.path().join("ling-mem.prev")).unwrap(),
            [0xcf, 0xfa, 0xed, 0xfe, 1, 2, 3]
        );
        let older: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("ling-mem.prev.")
            })
            .collect();
        assert_eq!(older.len(), 1, "the older .prev is kept under a stamp");

        let no_ling = "/usr/bin:/bin";
        let (ok, out, _) = sh(&bin, &["--version"], no_ling);
        assert!(ok);
        assert_eq!(out.trim(), "ling-mem 2.3.0");
        let (_, out, _) = sh(&bin, &["status", "--format", "json"], no_ling);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["state"], "running");
        assert_eq!(v["version"], "2.3.0");
        assert_eq!(v["port"], 9527);
        let (_, out, _) = sh(&bin, &["--format", "json", "status"], no_ling);
        assert!(out.contains("\"running\""), "{out}");
        for verb in ["start", "restart", "stop", "serve", "upgrade"] {
            let (ok, out, err) = sh(&bin, &[verb], no_ling);
            assert!(
                ok && out.is_empty() && err.contains("part of Linggen"),
                "{verb}"
            );
        }
        let (_, out, _) = sh(&bin, &["search", "a status", "--limit", "1"], no_ling);
        assert_eq!(out.trim(), "engine: mem search a status --limit 1");

        // An older `ling` on PATH (one with no `mem`) is not the one that answers.
        let old = dir.path().join("oldbin");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("ling"), "#!/bin/sh\necho \"old ling: $*\"\n").unwrap();
        set_executable(&old.join("ling")).unwrap();
        let with_old = format!("{}:{no_ling}", old.display());
        let (_, out, _) = sh(&bin, &["list"], &with_old);
        assert_eq!(out.trim(), "engine: mem list");

        // Again: nothing to do. A newer engine rewrites its own stand-in.
        assert!(install_at(&bin, &engine, "2.3.0", 9527).unwrap().is_none());
        assert!(install_at(&bin, &engine, "2.3.1", 9527).unwrap().is_some());
        assert_eq!(sh(&bin, &["-V"], no_ling).1.trim(), "ling-mem 2.3.1");
    }

    /// A stand-in of this same version that names another port — the old
    /// daemon's, as engines that also listened there wrote it — is rewritten,
    /// so its `status` says the engine's port.
    #[test]
    fn a_stand_in_of_the_same_version_on_another_port_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("ling-mem");
        let engine = Path::new("/nonexistent/ling");
        write_standin(&bin, engine, "2.3.0", 9528).unwrap();
        assert!(install_at(&bin, engine, "2.3.0", 4100).unwrap().is_some());
        let (_, out, _) = sh(&bin, &["status"], "/usr/bin:/bin");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["port"], 4100);
        assert!(install_at(&bin, engine, "2.3.0", 4100).unwrap().is_none());
    }

    #[test]
    fn where_nothing_is_the_stand_in_is_written_and_a_foreign_script_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin/ling-mem");
        let engine = Path::new("/nonexistent/ling");
        // No `ling-mem` yet (and no folder for it): an old plugin must find ours, not install the daemon.
        assert!(install_at(&bin, engine, "2.3.0", 9527).unwrap().is_some());
        assert_eq!(
            sh(&bin, &["-V"], "/usr/bin:/bin").1.trim(),
            "ling-mem 2.3.0"
        );
        assert!(!dir.path().join("bin/ling-mem.prev").exists());
        std::fs::write(&bin, "#!/bin/sh\necho mine\n").unwrap();
        assert!(install_at(&bin, engine, "2.3.0", 9527).unwrap().is_none());
        assert_eq!(
            std::fs::read_to_string(&bin).unwrap(),
            "#!/bin/sh\necho mine\n"
        );
    }

    fn stand_in(dir: &Path) -> PathBuf {
        let bin = dir.join("ling-mem");
        install_at(&bin, Path::new("/nonexistent/ling"), "2.3.0", 9527).unwrap();
        bin
    }

    fn old_binary(dir: &Path, name: &str, tail: u8) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, [0xcf, 0xfa, 0xed, 0xfe, tail]).unwrap();
        set_executable(&p).unwrap();
        p
    }

    #[test]
    fn the_old_program_comes_back_only_for_an_engine_without_memory_and_only_over_the_stand_in() {
        let si = Found::StandIn { version: "2.3.0".into() };
        let cases = [
            (true, &si, &Found::Binary, Restore::TargetHostsMemory),
            (false, &Found::Binary, &Found::Binary, Restore::NoStandIn),
            (false, &Found::Nothing, &Found::Binary, Restore::NoStandIn),
            (false, &Found::Other, &Found::Binary, Restore::NoStandIn),
            (false, &si, &Found::Nothing, Restore::NoPrevious),
            (false, &si, &Found::Other, Restore::NoPrevious),
            (false, &si, &Found::StandIn { version: "1".into() }, Restore::NoPrevious),
            (false, &si, &Found::Binary, Restore::Yes),
        ];
        for (hosts, at, prev, want) in cases {
            assert_eq!(restore_decision(hosts, at, prev), want, "{hosts} {at:?} {prev:?}");
        }
    }

    #[test]
    fn restoring_renames_the_kept_binary_over_the_stand_in_and_rolling_forward_keeps_it_aside_again() {
        let dir = tempfile::tempdir().unwrap();
        let bin = old_binary(dir.path(), "ling-mem", 1);
        install_at(&bin, Path::new("/nonexistent/ling"), "2.3.0", 9527).unwrap();
        let prev = dir.path().join("ling-mem.prev");
        assert_eq!(std::fs::read(&prev).unwrap()[4], 1);

        // An engine that hosts memory: nothing happens.
        assert_eq!(restore_previous_at(&bin, true).unwrap(), None);
        assert!(matches!(inspect(&bin).unwrap(), Found::StandIn { .. }));

        let line = restore_previous_at(&bin, false).unwrap().unwrap();
        assert!(line.contains("the previous ling-mem is back"), "{line}");
        assert_eq!(inspect(&bin).unwrap(), Found::Binary);
        assert_eq!(std::fs::read(&bin).unwrap()[4], 1);
        assert!(!prev.exists());
        let kept: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(kept.len(), 1, "only ling-mem is left");

        // Again: the binary is not a stand-in, so nothing changes.
        let line = restore_previous_at(&bin, false).unwrap().unwrap();
        assert!(line.contains("not the stand-in"), "{line}");
        assert_eq!(std::fs::read(&bin).unwrap()[4], 1);

        // Rolling forward: the engine that hosts memory keeps the binary aside again.
        assert!(install_at(&bin, Path::new("/nonexistent/ling"), "2.3.0", 9527)
            .unwrap()
            .is_some());
        assert!(matches!(inspect(&bin).unwrap(), Found::StandIn { .. }));
        assert_eq!(std::fs::read(&prev).unwrap()[4], 1);
    }

    #[test]
    fn without_a_runnable_kept_binary_the_stand_in_stays_and_one_line_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let bin = stand_in(dir.path());
        let before = std::fs::read(&bin).unwrap();

        let line = restore_previous_at(&bin, false).unwrap().unwrap();
        assert!(line.contains("no ling-mem.prev"), "{line}");

        // Present but not executable, or not a program: the same.
        let prev = old_binary(dir.path(), "ling-mem.prev", 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&prev, std::fs::Permissions::from_mode(0o644)).unwrap();
            let line = restore_previous_at(&bin, false).unwrap().unwrap();
            assert!(line.contains("no ling-mem.prev"), "{line}");
        }
        std::fs::write(&prev, "#!/bin/sh\n").unwrap();
        assert!(restore_previous_at(&bin, false).unwrap().unwrap().contains("no ling-mem.prev"));
        assert_eq!(std::fs::read(&bin).unwrap(), before);

        // Nothing at the path at all: nothing is made.
        let none = dir.path().join("empty/ling-mem");
        let line = restore_previous(false, Some(&none)).unwrap();
        assert!(line.contains("not the stand-in"), "{line}");
        assert!(!none.exists());
        assert_eq!(restore_previous(false, None), None);
    }
}
