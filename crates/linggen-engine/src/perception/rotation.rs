//! Rotation and lifecycle (`doc/perception-spec.md` §3).
//!
//! One moment, two jobs:
//!
//! 1. **A state sample** for the trend series, so a condition can talk about
//!    slope rather than level.
//! 2. **The sweep.** A day file older than
//!    [`RETAIN_DAYS`](super::activity::RETAIN_DAYS) is deleted. Activities
//!    never become memory: the log is read, never copied anywhere else.

use std::time::Duration;

/// Hourly. Rotation is cheap and idempotent — one sample a day wins, and a
/// swept day is simply no longer there to sweep.
const TICK: Duration = Duration::from_secs(3600);

/// Settle after boot: the daemon has better things to do in its first minute,
/// and a machine that is up for thirty seconds has no day to rotate.
const STARTUP: Duration = Duration::from_secs(120);

pub async fn rotation_loop() {
    tokio::time::sleep(STARTUP).await;
    loop {
        rotate();
        tokio::time::sleep(TICK).await;
    }
}

fn rotate() {
    if let Some((free, total)) = super::state::disk() {
        super::trend::sample(free, total);
    }

    // This loop is the only sweeper. Reading the log does not rotate it — the
    // window a reader sees must not depend on who happened to read first.
    let log = super::activity::log();
    for day in log.sweep() {
        tracing::info!("[perception] {day} passed the retention and was removed");
    }
    // The doorbell's window may have moved since the last pass — a daemon up
    // across midnight still holds a day that left it until the days are read
    // again.
    log.restore();
}
