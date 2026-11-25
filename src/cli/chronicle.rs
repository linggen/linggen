//! `ling chronicle …` — the chronicle's two verbs (memory v2, `doc/memory-spec.md`).
//!
//! - `prepare` is the `chronicle` mission's prepare step: it writes the
//!   input the mission reads (`own_memory::chronicle`) and exits with
//!   `missions::NOTHING_TO_DO` when memory v2 is off or no month is waiting,
//!   so the run ends quietly with no model call.
//! - `backfill` starts that mission now, by hand: the same run the schedule
//!   makes, over every month the person's notes, sessions and other agents'
//!   notes still cover (up to a run's share; run it again for more).
//!
//! Neither writes a note: the mission's agent does, through the one writer.

use crate::config::Config;
use crate::extensions::missions::NOTHING_TO_DO;
use crate::own_memory::{chronicle, Mode, Store};
use crate::state_fs::sessions::SessionStore;
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::time::Duration;

/// The built-in mission's id.
const MISSION: &str = "chronicle";

pub enum ChronicleAction {
    Prepare,
    Backfill,
}

pub async fn run(action: ChronicleAction, config: &Config) -> Result<()> {
    match action {
        ChronicleAction::Prepare => prepare(config),
        ChronicleAction::Backfill => backfill(config).await,
    }
}

fn prepare(config: &Config) -> Result<()> {
    if config.memory.linggen() != Mode::V2 {
        println!("The chronicle is kept only when [memory] linggen = \"v2\".");
        std::process::exit(NOTHING_TO_DO);
    }
    let store = Store::this_machine().context("this machine has no home folder")?;
    let now = crate::util::now_ts_secs() as i64;
    let Some(ready) = chronicle::prepare(&store, SessionStore::global(), now) else {
        println!("No month of the chronicle is waiting to be written.");
        std::process::exit(NOTHING_TO_DO);
    };
    std::fs::create_dir_all(store.root())?;
    std::fs::write(store.root().join(chronicle::INPUT_FILE), &ready.text)?;
    println!("chronicle: {} month(s), {} year(s) to write", ready.months.len(), ready.years.len());
    Ok(())
}

async fn backfill(config: &Config) -> Result<()> {
    if config.memory.linggen() != Mode::V2 {
        bail!("The chronicle is kept only when [memory] linggen = \"v2\".");
    }
    let port = config.server.port();
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/missions/{MISSION}/trigger"))
        .json(&json!({}))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("The engine is not answering")?;
    let ok = response.status().is_success();
    let body = response.text().await.unwrap_or_default();
    if !ok {
        bail!("The engine did not start the chronicle: {body}");
    }
    println!("The chronicle run has started. Run `ling chronicle backfill` again when it ends if more months are waiting.");
    Ok(())
}
