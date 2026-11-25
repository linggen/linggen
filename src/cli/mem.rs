//! `ling mem <verb> …` — memory's data verbs (add, get, search, list, …).
//!
//! The engine holding the store (its lock names its port) → the verb goes to
//! it over loopback HTTP. No engine → the store is opened right here, under
//! the same lock, so an engine starting meanwhile waits rather than opening a
//! second writer. The verbs that have no HTTP form (`--episodic`, `export`,
//! `import`) open the store themselves beside a running engine, as they did
//! beside the old daemon. `scan` touches no store at all: it runs without the
//! lock, engine or not. A CLI call never starts the engine. The lifecycle verbs of
//! the old daemon (serve, start, stop, restart, status, upgrade) are not here:
//! memory runs inside the engine.

use anyhow::{anyhow, Result};
use clap::Parser;
use ling_mem::cli::scan::ScanExit;
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
        exit_with(&e);
    }
    Ok(())
}

/// A scan exits as its scripts did (they printed their own error), or with
/// its one plain line; every other verb by the old CLI's JSON contract.
fn exit_with(err: &anyhow::Error) -> ! {
    if let Some(scan) = err.downcast_ref::<ScanExit>() {
        if let Some(line) = &scan.line {
            eprintln!("{line}");
        }
        std::process::exit(scan.code);
    }
    print_error(err);
    std::process::exit(1);
}

async fn dispatch(mut cli: Cli) -> Result<()> {
    if let Command::Scan(args) = cli.cmd {
        let (account, local) = crate::server::person::scan_owner_ids().map_err(|err| ScanExit {
            code: 1,
            line: Some(format!("ling mem scan cannot resolve owner identity: {err}")),
        })?;
        return ling_mem::cli::scan::run(args, account.as_deref(), local, crate::paths::linggen_home());
    }
    let data_dir = cli
        .data_dir
        .get_or_insert_with(|| crate::paths::linggen_home().clone())
        .clone();
    match StoreLock::acquire(&data_dir, None) {
        // No engine holds the store: open it here, under the lock.
        Ok(_lock) => ling_mem::cli::run_data(cli, None).await,
        Err(busy) => {
            let base = engine_base(&data_dir).await.ok_or(busy)?;
            let base = (!needs_the_store(&cli)).then_some(base);
            ling_mem::cli::run_data(cli, base).await
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

/// Verbs with no HTTP form: they run on the store directly, engine or not.
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
