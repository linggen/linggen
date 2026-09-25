//! Bash command-builders shared by skill install scripts and mission
//! entry scripts. Both paths invoke `bash` with a custom cwd + env, then
//! capture the output. The two callers diverge after `.output()` — skills
//! bail on non-zero exit and return stdout-as-string; missions write
//! stdout/stderr to log files and return the exit code. That post-
//! processing stays at the call site; this module owns the launch.

use std::ffi::OsStr;
use std::path::Path;

/// How to invoke the script: a path to a file, run as `bash <file>`.
pub enum Invocation<'a> {
    File(&'a Path),
}

/// Build a sync `std::process::Command` configured to invoke bash with the
/// given cwd, env, and invocation. Caller chains `.output()` and handles
/// stdout/stderr per their policy.
pub fn sync_command(
    inv: Invocation<'_>,
    cwd: &Path,
    env: &[(&str, &OsStr)],
) -> std::process::Command {
    let mut cmd = std::process::Command::new("bash");
    cmd.current_dir(cwd);
    for (k, v) in env {
        cmd.env(*k, *v);
    }
    let Invocation::File(p) = inv;
    cmd.arg(p);
    cmd
}
