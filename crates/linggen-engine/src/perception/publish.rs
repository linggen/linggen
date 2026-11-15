//! Two machines, one view (`doc/perception-spec.md` §6).
//!
//! Each host writes its own log and computes its own state; neither file is
//! synced. What crosses is a summary of what a host measured about *itself*,
//! and it crosses the way every other reading does — published on a retained
//! topic, because reads are published and actions are queued. The reader
//! renders it with its age, so a stale account can never pass for a fresh one,
//! and a host that cannot be reached simply contributes nothing.

use std::time::Duration;

/// The topic both hosts publish their own state on.
pub const TOPIC: &str = "perception";
/// Op the Mac publishes under — what the phone reads.
pub const FROM_MAC: &str = "mac";
/// Op a phone publishes under — what the Mac reads.
pub const FROM_PHONE: &str = "phone";

/// How often the Mac reconsiders what it would say about itself. Cheap: one
/// `statvfs` and an in-memory list, and nothing goes on the wire unless the
/// answer changed.
const TICK: Duration = Duration::from_secs(60);

/// Tell the user's devices what is true here, whenever that changes.
///
/// Silent when no device is connected — there is nobody to tell, and a
/// retained payload nobody reads is just a file aging on disk.
/// `publish` hands one retained payload (`topic`, `op`, payload) to the
/// daemon's topic bus — the only thing this loop needs from it.
pub async fn publish_loop(publish: impl Fn(&str, &str, serde_json::Value) + Send + 'static) {
    tracing::info!("[perception] publishing this Mac's state to connected devices");
    let mut last: Option<String> = None;
    let mut known: Vec<String> = Vec::new();
    loop {
        tokio::time::sleep(TICK).await;
        // A device that just arrived has heard nothing, whatever we said to the
        // one before it. Without this, "unchanged" is measured against a reader
        // that is no longer there, and a phone can stay connected for an hour
        // with no account of the Mac at all.
        let present = super::devices::present_ids();
        if present != known {
            known = present.clone();
            last = None;
        }
        if present.is_empty() {
            continue;
        }
        let lines = super::state::share();
        if lines.is_empty() {
            continue;
        }
        // History rides along but never rides a turn: the other host shows it
        // only when its `recent_activity` is called. Without it, "what has been
        // happening" is answered from one machine's log while the user is
        // thinking of both — which reads as the agent having missed something
        // they just did on the other one.
        let recent = super::state::share_history();
        // Both halves decide freshness. A song deleted here moves the history
        // and not one state line, and a payload we declined to resend over that
        // is a delete the other machine never hears about.
        let digest = format!("{}\n--\n{}", lines.join("\n"), recent.join("\n"));
        if last.as_deref() == Some(digest.as_str()) {
            continue;
        }
        last = Some(digest);
        let payload = serde_json::json!({
            "host": super::host_name(),
            "at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "lines": lines,
            "recent": recent,
        });
        publish(TOPIC, FROM_MAC, payload);
    }
}
