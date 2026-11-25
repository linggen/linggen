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
//!
//! `migrate-v2` is the one verb of this program, not of the memory crate: it
//! moves the v1 rows into Linggen's own notes through the engine's writer
//! (`own_memory::migrate`), which the memory crate cannot reach.

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
    if verb_of(&args) == Some(MIGRATE_VERB) {
        if let Err(e) = migrate_v2(&args).await {
            print_error(&e);
            std::process::exit(1);
        }
        return Ok(());
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

/// The verb that moves the v1 rows into the notes.
const MIGRATE_VERB: &str = "migrate-v2";

/// `ling mem migrate-v2 [--dry-run] [--from FILE|-] [--data-dir DIR]`
#[derive(Parser, Debug)]
#[command(name = "ling mem migrate-v2", about = "Move the v1 memory rows into Linggen's own notes (v2); the v1 store is left as it is. Safe to run again.")]
struct MigrateArgs {
    /// Count what would be kept and dropped; write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Read the rows from an export file (`ling mem export`; `-` is stdin)
    /// instead of the store.
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,
    #[arg(long, hide = true)]
    format: Option<String>,
}

async fn migrate_v2(args: &[String]) -> Result<()> {
    use crate::own_memory::{migrate, Store};
    let rest = args.iter().filter(|a| a.as_str() != MIGRATE_VERB).cloned();
    let args = MigrateArgs::try_parse_from(std::iter::once("ling mem migrate-v2".to_string()).chain(rest))
        .unwrap_or_else(|e| e.exit());
    let data_dir = args.data_dir.clone().unwrap_or_else(|| crate::paths::linggen_home().clone());
    let notes = match &args.data_dir {
        Some(dir) => Store::new(
            dir.join("memory"),
            crate::outside_memory::Env::this_machine().ok_or_else(|| anyhow!("this machine has no home folder"))?,
        ),
        None => Store::this_machine().ok_or_else(|| anyhow!("this machine has no home folder"))?,
    };
    let (rows, unreadable) = match args.from.as_deref() {
        Some(file) => migrate::V1Row::from_ndjson(&read_export(file)?),
        None => migrate::V1Row::from_ndjson(&list_rows(&data_dir).await?),
    };
    let mut report = tokio::task::spawn_blocking(move || migrate::migrate(&notes, &rows, !args.dry_run)).await?;
    report.unreadable = unreadable;
    for line in report.lines(args.dry_run) {
        println!("{line}");
    }
    Ok(())
}

fn read_export(file: &str) -> Result<String> {
    use std::io::Read;
    match file {
        "-" => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            Ok(text)
        }
        path => std::fs::read_to_string(path).map_err(|e| anyhow!("cannot read {path}: {e}")),
    }
}

/// Every row of the store, both tables, as `ling mem export` gives them (it
/// opens the store itself beside a running engine, and only reads). The v1
/// store is not changed.
async fn list_rows(data_dir: &std::path::Path) -> Result<String> {
    let dir = data_dir.display().to_string();
    let scratch = tempfile::tempdir()?;
    let mut text = String::new();
    for (name, episodic) in [("semantic.ndjson", false), ("episodic.ndjson", true)] {
        let file = scratch.path().join(name);
        let mut argv = vec!["ling mem".to_string(), "--data-dir".to_string(), dir.clone()];
        argv.extend(episodic.then(|| "--episodic".to_string()));
        argv.extend(["export".to_string(), file.display().to_string()]);
        let cli = Cli::try_parse_from(argv).map_err(|e| anyhow!("{e}"))?;
        dispatch(cli).await?;
        text.push_str(&std::fs::read_to_string(&file)?);
    }
    Ok(text)
}
