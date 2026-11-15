//! `ling mem <verb> …` — memory's data verbs (add, get, search, list, …).
//!
//! The engine holding the store (its lock names its port) → the verb goes to
//! it over loopback HTTP. No engine → the store is opened right here, under
//! the same lock, so an engine starting meanwhile waits rather than opening a
//! second writer. A CLI call never starts the engine. The lifecycle verbs of
//! the old daemon (serve, start, stop, restart, status, upgrade) are not here:
//! memory runs inside the engine.

use anyhow::{anyhow, bail, Result};
use clap::Parser;
use ling_mem::cli::{Cli, Command};
use ling_mem::hosted::StoreLock;
use std::io::Write;
use std::time::Duration;

pub async fn run(args: Vec<String>) -> Result<()> {
    if let Err(e) = refuse_lifecycle(&args) {
        print_error(&e);
        std::process::exit(1);
    }
    let argv = std::iter::once("ling mem".to_string()).chain(args);
    let cli = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    if let Err(e) = dispatch(cli).await {
        print_error(&e);
        std::process::exit(1);
    }
    Ok(())
}

async fn dispatch(mut cli: Cli) -> Result<()> {
    let data_dir = cli
        .data_dir
        .get_or_insert_with(|| crate::paths::linggen_home().clone())
        .clone();
    match StoreLock::acquire(&data_dir, None) {
        // No engine holds the store: open it here, under the lock.
        Ok(_lock) => ling_mem::cli::run_data(cli, None).await,
        Err(busy) => {
            let base = engine_base(&data_dir).await.ok_or(busy)?;
            if needs_the_store(&cli) {
                bail!(
                    "this verb opens the store itself, and the engine at {base} has it open — \
                     stop the engine (`ling stop`) and run it again"
                );
            }
            ling_mem::cli::run_data(cli, Some(base)).await
        }
    }
}

/// The engine holding the store, from what its lock records, if it answers.
async fn engine_base(data_dir: &std::path::Path) -> Option<String> {
    let port = StoreLock::holder(data_dir)?.port?;
    let base = format!("http://127.0.0.1:{port}{}", crate::memory_host::PREFIX);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build()
        .ok()?;
    let health = client.get(format!("{base}/api/health")).send().await.ok()?;
    health.status().is_success().then_some(base)
}

/// Verbs the old CLI always ran on the store directly, never over HTTP.
fn needs_the_store(cli: &Cli) -> bool {
    cli.episodic || matches!(cli.cmd, Command::Export { .. } | Command::Import { .. })
}

/// The subcommand: the first word that is neither a flag nor a global
/// option's value.
fn verb_of(args: &[String]) -> Option<&str> {
    let mut words = args.iter();
    while let Some(w) = words.next() {
        match w.as_str() {
            "--data-dir" | "--format" => {
                words.next();
            }
            w if w.starts_with('-') => {}
            w => return Some(w),
        }
    }
    None
}

/// The old daemon's lifecycle verbs, which `ling mem` does not have.
const LIFECYCLE: &[&str] = &["serve", "start", "stop", "restart", "status", "upgrade"];

fn refuse_lifecycle(args: &[String]) -> Result<()> {
    let Some(verb) = verb_of(args).filter(|v| LIFECYCLE.contains(v)) else {
        return Ok(());
    };
    Err(anyhow!(
        "`ling mem {verb}` is not a memory verb: memory runs inside the Linggen engine \
         (`ling status`, `ling restart`, `ling update`)"
    ))
}

/// The old CLI's error contract: one JSON line on stderr, exit 1.
fn print_error(err: &anyhow::Error) {
    let payload = serde_json::json!({
        "error": err.to_string(),
        "code": ling_mem::cli::error_code(err),
    });
    let mut stderr = std::io::stderr().lock();
    let _ = serde_json::to_writer(&mut stderr, &payload);
    let _ = stderr.write_all(b"\n");
}
