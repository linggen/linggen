//! `ling mem scan [WINDOW]` — walk one day's local session logs (Claude Code,
//! Codex, OpenClaw, Linggen) into the scan candidates file,
//! `~/.linggen/memory/.scan-output.jsonl`. No model, no network, no store:
//! it runs the same with the engine up or stopped.
//!
//! The work is three bash scripts (`crates/linggen-memory/scan/`), embedded
//! here. Each run writes them into a private temporary folder (mode 0700,
//! removed afterwards) and runs `bash scan.sh` there. They are not piped to
//! bash: `scan.sh` finds the other two beside itself by path and runs each
//! per session, so they must exist as files — and a folder only this user can
//! open means nothing can swap one between the write and the run. The output
//! file, its header line, the summary line on stdout and the exit codes are
//! the scripts' own.

use clap::Args;
use std::process::Command;

/// The scripts, by the file name `scan.sh` looks for.
const SCRIPTS: [(&str, &str); 3] = [
    ("scan.sh", include_str!("../../scan/scan.sh")),
    (
        "collect_sessions.sh",
        include_str!("../../scan/collect_sessions.sh"),
    ),
    (
        "extract_session.sh",
        include_str!("../../scan/extract_session.sh"),
    ),
];

/// What the scripts run on.
const TOOLS: [&str; 3] = ["bash", "jq", "perl"];

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// One day, `YYYY-MM-DD`; or a window back from today: `today`/`24h`
    /// (the default), `week`, `month`, `<n>d`, `<n>w`, `<n>m`, `<n>y`.
    pub window: Option<String>,
}

/// A scan that did not succeed: the status to exit with, and the one line to
/// print — none when the scripts already printed their own.
#[derive(Debug)]
pub struct ScanExit {
    pub code: i32,
    pub line: Option<String>,
}

impl std::fmt::Display for ScanExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.line {
            Some(line) => f.write_str(line),
            None => write!(f, "scan exited with status {}", self.code),
        }
    }
}

impl std::error::Error for ScanExit {}

pub fn run(args: ScanArgs) -> anyhow::Result<()> {
    if let Some(tool) = TOOLS.iter().find(|t| !on_path(t)) {
        return Err(ScanExit {
            code: 1,
            line: Some(format!("ling mem scan needs {tool}, which is not on PATH")),
        }
        .into());
    }
    let dir = tempfile::Builder::new()
        .prefix("ling-mem-scan-")
        .tempdir()?;
    for (name, text) in SCRIPTS {
        std::fs::write(dir.path().join(name), text)?;
    }
    let status = Command::new("bash")
        .arg(dir.path().join("scan.sh"))
        .args(args.window)
        .status()?;
    match status.code() {
        Some(0) => Ok(()),
        code => Err(ScanExit {
            code: code.unwrap_or(1),
            line: None,
        }
        .into()),
    }
}

/// Is `tool` a file in one of `$PATH`'s folders?
fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(tool).is_file()))
}
