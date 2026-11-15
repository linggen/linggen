//! The `ling-mem` stand-in: memory is part of the engine, but plugin hooks
//! and scripts of sessions already running still call the `ling-mem`
//! binary at the path the old installers used (`$HOME/.local/bin/ling-mem`).
//!
//! Once the engine serves the store, a real `ling-mem` binary found there
//! (Mach-O or ELF) is moved aside as `ling-mem.prev` — kept for a rollback;
//! an older `.prev` is renamed with a timestamp, never overwritten — and a
//! small shell script takes its place:
//!
//! - data verbs run as `ling mem "$@"` (the engine that wrote the script, else
//!   `ling` on PATH — which may be an older one without `mem`);
//! - `--version` answers this engine's version, so the plugin's
//!   `install-bin.sh` (which replaces only an older binary) leaves it alone;
//! - `status` answers what the old autostart hook reads from a running
//!   daemon at that version (`state`, `version`, `port`), so the hook neither
//!   starts nor restarts anything;
//! - `serve`, `start`, `stop`, `restart`, `upgrade` do nothing, and say so.
//!
//! A stand-in from an older engine is rewritten; any other script is left.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Marks a file as ours (and which engine version wrote it).
const MARK: &str = "# linggen-ling-mem-stand-in";

/// The path the old installers and the engine's old resolver used.
fn canonical_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/bin/ling-mem"))
}

/// Put the stand-in in place, if a real `ling-mem` binary is there. Best
/// effort: a failure is logged and changes nothing else.
pub(super) fn install(legacy_port: u16) {
    let Some(path) = canonical_path() else {
        return;
    };
    let engine = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ling"));
    match install_at(&path, &engine, super::VERSION, legacy_port) {
        Ok(Some(what)) => tracing::info!("memory: {what}"),
        Ok(None) => {}
        Err(e) => tracing::warn!("memory: the ling-mem stand-in was not written: {e}"),
    }
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
    legacy_port: u16,
) -> std::io::Result<Option<String>> {
    match inspect(path)? {
        Found::Nothing | Found::Other => Ok(None),
        Found::StandIn { version: v } if v == version => Ok(None),
        Found::StandIn { .. } => {
            write_standin(path, engine, version, legacy_port)?;
            Ok(Some(format!(
                "ling-mem stand-in at {} updated",
                path.display()
            )))
        }
        Found::Binary => {
            let prev = keep_aside(path)?;
            write_standin(path, engine, version, legacy_port)?;
            Ok(Some(format!(
                "ling-mem at {} is now a stand-in for `ling mem`; the old binary is {}",
                path.display(),
                prev.display()
            )))
        }
    }
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

/// Write the script beside `path` and rename it over — a new inode, never
/// an overwrite of a file something may be running.
fn write_standin(
    path: &Path,
    engine: &Path,
    version: &str,
    legacy_port: u16,
) -> std::io::Result<()> {
    let tmp = path.with_file_name(format!(".ling-mem.standin.{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(script(engine, version, legacy_port).as_bytes())?;
    }
    set_executable(&tmp)?;
    std::fs::rename(&tmp, path)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// A value for the script, single-quoted for sh.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn script(engine: &Path, version: &str, legacy_port: u16) -> String {
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
    printf '{{"state":"running","version":"%s","port":{legacy_port},"host":"linggen"}}\n' "$VERSION"
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

        let changed = install_at(&bin, &engine, "2.3.0", 9528).unwrap();
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
        assert_eq!(v["port"], 9528);
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
        assert!(install_at(&bin, &engine, "2.3.0", 9528).unwrap().is_none());
        assert!(install_at(&bin, &engine, "2.3.1", 9528).unwrap().is_some());
        assert_eq!(sh(&bin, &["-V"], no_ling).1.trim(), "ling-mem 2.3.1");
    }

    #[test]
    fn nothing_and_foreign_scripts_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("ling-mem");
        let engine = Path::new("/nonexistent/ling");
        assert!(install_at(&bin, engine, "2.3.0", 9528).unwrap().is_none());
        assert!(!bin.exists());
        std::fs::write(&bin, "#!/bin/sh\necho mine\n").unwrap();
        assert!(install_at(&bin, engine, "2.3.0", 9528).unwrap().is_none());
        assert_eq!(
            std::fs::read_to_string(&bin).unwrap(),
            "#!/bin/sh\necho mine\n"
        );
    }
}
