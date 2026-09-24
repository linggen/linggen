pub fn now_ts_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn now_ts_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Bin directories a user's shell has but launchd's stock
/// `PATH=/usr/bin:/bin:/usr/sbin:/sbin` does not.
const EXTRA_BIN_DIRS: [&str; 5] = [
    "~/.local/bin",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
];

/// `$PATH` for shell children, with the user bin dirs above appended.
///
/// A GUI-launched daemon (Linggen.app → launchd) inherits launchd's stock
/// PATH and hands it to every `sh -c` child, so tools the usual installers
/// drop — `ling-mem` in `~/.local/bin`, anything from Homebrew — are simply
/// not found. A daemon started from a terminal inherits the full interactive
/// PATH and works, which is what makes the failure look intermittent.
///
/// Appended, never prepended: system binaries keep the precedence they
/// already have, so this only resolves commands that would otherwise fail.
/// Computed once — the directory set doesn't change while the daemon runs.
pub fn shell_path() -> &'static str {
    static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let inherited = std::env::var("PATH").unwrap_or_default();
        let mut dirs: Vec<String> = inherited
            .split(':')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        for extra in EXTRA_BIN_DIRS {
            let resolved = resolve_path(std::path::Path::new(extra));
            if !resolved.is_dir() {
                continue;
            }
            let dir = resolved.to_string_lossy().to_string();
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        // Managed-runtime bins go last, and unconditionally: the prewarm
        // task creates them while the daemon runs, so an is_dir() gate here
        // would leave a first boot without them until restart.
        for dir in crate::runtime::path_dirs() {
            let dir = dir.to_string_lossy().to_string();
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        dirs.join(":")
    })
}

/// Marks where a wrapped command's output ends and its final `pwd` begins.
pub const CWD_SENTINEL: &str = "__LINGGEN_CWD__";

/// `command`, then the sentinel and `pwd`, keeping the command's exit code.
///
/// Joined on a NEWLINE, never `;`: a command that ends in `&`, in a newline,
/// or in a heredoc terminator made `…; __linggen_ec` a syntax error, and `sh`
/// then ran nothing at all — a background `nohup … &` was simply never
/// started (2026-09-23), a trailing newline failed the same way (2026-07-27).
pub fn wrap_with_cwd_sentinel(command: &str) -> String {
    format!("{command}\n__linggen_ec=$?; echo '{CWD_SENTINEL}'; pwd; exit $__linggen_ec")
}

/// Resolve a path. Expands a leading `~` to `$HOME` first, then follows
/// symlinks via `canonicalize`. On macOS, `/tmp` → `/private/tmp`.
///
/// Falls back to the expanded-but-not-canonicalized path when the target
/// doesn't exist, so callers that want a best-effort absolute path (e.g.
/// mission cwd before directories are created) still get a usable value.
pub fn resolve_path(path: &std::path::Path) -> std::path::PathBuf {
    let expanded: std::path::PathBuf = {
        let s = path.to_string_lossy();
        if let Some(rest) = s.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(rest)
            } else {
                path.to_path_buf()
            }
        } else if s == "~" {
            dirs::home_dir().unwrap_or_else(|| path.to_path_buf())
        } else {
            path.to_path_buf()
        }
    };
    expanded.canonicalize().unwrap_or(expanded)
}

/// The text a panic carried — `panic!("…")` and failed assertions give a
/// `&str` or a `String`; anything else has no words.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_string())
}

/// Await a fallible future, turning a panic inside it into an error. A panic
/// that unwinds a spawned task ends it with no result: a chat turn then never
/// finishes and says nothing, where an error reaches every caller's existing
/// failure handling.
pub async fn panic_as_error<T>(
    fut: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    use futures_util::FutureExt;
    std::panic::AssertUnwindSafe(fut)
        .catch_unwind()
        .await
        .unwrap_or_else(|payload| {
            Err(anyhow::anyhow!(
                "internal error, the turn stopped: {}",
                panic_message(&*payload)
            ))
        })
}

/// Poison-tolerant locking for std `Mutex`.
///
/// A panic while a std lock is held marks it poisoned, and `lock().unwrap()`
/// then panics in every later caller — one bad request would take a registry
/// or cache down for the rest of the daemon's life. The data is whatever the
/// panicking thread left, which for these caches and registries is still
/// usable, so recover the guard instead.
pub trait LockExt<T: ?Sized> {
    fn lock_ok(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T: ?Sized> LockExt<T> for std::sync::Mutex<T> {
    fn lock_ok(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The `RwLock` side of [`LockExt`].
pub trait RwLockExt<T: ?Sized> {
    fn read_ok(&self) -> std::sync::RwLockReadGuard<'_, T>;
    fn write_ok(&self) -> std::sync::RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> RwLockExt<T> for std::sync::RwLock<T> {
    fn read_ok(&self) -> std::sync::RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|e| e.into_inner())
    }
    fn write_ok(&self) -> std::sync::RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|e| e.into_inner())
    }
}

/// Constant-time equality for secrets (device tokens), so a caller cannot
/// learn a token byte by byte from response timing.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod panic_tests {
    use super::*;

    #[tokio::test]
    async fn a_panic_comes_back_as_an_error_with_its_message() {
        let err = panic_as_error(async {
            let s = String::from("月");
            let _ = &s[..1];
            Ok(())
        })
        .await
        .unwrap_err();
        assert!(err
            .to_string()
            .starts_with("internal error, the turn stopped: "));
        assert!(err.to_string().contains("char boundary"), "{err}");
    }

    #[tokio::test]
    async fn results_pass_through_untouched() {
        assert_eq!(panic_as_error(async { Ok(7) }).await.unwrap(), 7);
        let err = panic_as_error(async { Err::<(), _>(anyhow::anyhow!("model timed out")) })
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "model timed out");
    }

    #[test]
    fn a_formatted_panic_keeps_its_words() {
        let payload = std::panic::catch_unwind(|| panic!("tool {} broke", "Read")).unwrap_err();
        assert_eq!(panic_message(&*payload), "tool Read broke");
    }

    /// Runs `command` wrapped, returning (exit code, stdout).
    fn run_wrapped(command: &str) -> (i32, String) {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(wrap_with_cwd_sentinel(command))
            .current_dir("/tmp")
            .output()
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into(),
        )
    }

    #[test]
    fn a_wrapped_command_keeps_its_exit_code_and_reports_its_cwd() {
        let (code, out) = run_wrapped("echo hi; false");
        assert_eq!(code, 1);
        assert!(out.starts_with("hi\n"));
        assert!(out.contains(&format!("{CWD_SENTINEL}\n")));
    }

    #[test]
    fn a_trailing_ampersand_newline_or_heredoc_still_runs() {
        for command in ["sleep 0 &", "echo a\n", "cat <<EOF\nbody\nEOF"] {
            let (code, out) = run_wrapped(command);
            assert_eq!(code, 0, "{command:?} failed: {out}");
            assert!(out.contains(CWD_SENTINEL), "{command:?} lost the sentinel");
        }
    }
}
