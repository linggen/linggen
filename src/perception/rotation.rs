//! Rotation and lifecycle (`doc/perception-spec.md` §7).
//!
//! One moment, two outputs:
//!
//! 1. **A state sample** for the trend series, so a condition can talk about
//!    slope rather than level.
//! 2. **The day's activities handed to the dream pass.** Activity is to
//!    long-term memory what episodic staging is to semantic: the dream judges
//!    the day and promotes what is durable about the person, and the raw rows
//!    are swept afterwards. Activities do not get a second lifecycle of their
//!    own, and nothing is uploaded (§8) — the digest goes to `ling-mem`, which
//!    is this machine.

use std::time::Duration;

/// Hourly. Rotation is cheap and idempotent — one sample a day wins, and a day
/// is handed over exactly once because handing it over is what deletes it.
const TICK: Duration = Duration::from_secs(3600);

/// Settle after boot: the daemon has better things to do in its first minute,
/// and a machine that is up for thirty seconds has no day to rotate.
const STARTUP: Duration = Duration::from_secs(120);

pub async fn rotation_loop(state: std::sync::Arc<crate::server::ServerState>) {
    tokio::time::sleep(STARTUP).await;
    loop {
        let url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
        rotate(&url).await;
        tokio::time::sleep(TICK).await;
    }
}

async fn rotate(ling_mem_url: &str) {
    if let Some((free, total)) = super::state::disk() {
        super::trend::sample(free, total);
    }

    // Loading is what sweeps: any day outside the keep window comes back here
    // once, on its way out.
    for (day, rows) in super::activity::log().load() {
        let Some(digest) = digest(&day, &rows) else {
            continue;
        };
        match hand_to_dream(ling_mem_url, &digest).await {
            Ok(()) => tracing::info!("[perception] {day} handed to the dream pass"),
            // The rows are already gone — saying so is all that is left, and a
            // failed handoff must not cost the daemon anything else.
            Err(e) => tracing::warn!("[perception] {day} could not be handed over: {e}"),
        }
    }
}

/// One day, in the words a reader would use. `None` for a day with nothing in
/// it — an empty day is not a memory.
fn digest(day: &str, rows: &[super::activity::Activity]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = rows.iter().map(|a| a.line()).collect();
    // A day of one app repeating itself is one fact, not forty; keep the shape
    // of the day rather than every beat of it.
    lines.dedup();
    let shown: Vec<String> = lines.iter().take(40).cloned().collect();
    let more = lines.len().saturating_sub(shown.len());
    let tail = if more > 0 {
        format!("\n…and {more} more")
    } else {
        String::new()
    };
    Some(format!(
        "What happened on {} on {day} ({} thing{}):\n{}{tail}",
        super::host_name(),
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
        shown.join("\n")
    ))
}

/// Stage the day as one episodic row. The dream judges it like any other day's
/// staging: what is durable about the person is promoted, the rest ages out on
/// the ordinary TTL. One row per day per host, not one per activity — a
/// memory store filling with "you deleted a song" is the failure this design
/// exists to avoid.
async fn hand_to_dream(ling_mem_url: &str, digest: &str) -> anyhow::Result<()> {
    crate::engine::tools::memory_http::call_memory_http(
        ling_mem_url,
        "perception rotation",
        serde_json::json!({
            "verb": "add",
            "tier": "episodic",
            "type": "fact",
            "content": digest,
            "contexts": ["perception"],
            "host": "linggen",
        }),
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perception::activity::Activity;

    fn row(verb: &str, object: &str) -> Activity {
        Activity {
            at: "2026-08-06T11:00:00Z".into(),
            by: "user".into(),
            device: None,
            app: "dj".into(),
            verb: verb.into(),
            object: Some(object.into()),
            detail: None,
        }
    }

    #[test]
    fn an_empty_day_is_not_a_memory() {
        assert!(digest("2026-08-06", &[]).is_none());
    }

    #[test]
    fn a_day_reads_as_lines_a_person_wrote() {
        let d = digest("2026-08-06", &[row("delete", "三天三夜"), row("sync", "12 songs")]).unwrap();
        assert!(d.contains("on 2026-08-06 (2 things)"), "{d}");
        assert!(d.contains("you delete 三天三夜"), "{d}");
        assert!(d.contains("you sync 12 songs"), "{d}");
    }

    #[test]
    fn a_long_day_keeps_its_shape_not_its_every_beat() {
        let rows: Vec<Activity> = (0..60).map(|i| row("delete", &format!("song {i}"))).collect();
        let d = digest("2026-08-06", &rows).unwrap();
        assert!(d.contains("(60 things)"), "the count stays honest: {d}");
        assert!(d.contains("…and 20 more"), "{d}");
    }
}
