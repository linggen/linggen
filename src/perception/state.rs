//! What is true on this machine right now, as lines for the resident's prompt.
//!
//! See `doc/perception-spec.md` §2 and §4. Two rules earn their keep:
//!
//! Every line is a **reading**, taken now from the same source the screen
//! shows — so what the agent says can never drift from what the user is looking
//! at. And nothing accumulates: this is rebuilt each turn and never stored,
//! which is what lets it ride every turn at all. History does not; it costs a
//! tool call ([`super::activity`]) and is announced here by two lines rather
//! than carried.

use std::collections::HashMap;
use std::sync::Mutex;

/// Free and total bytes on the volume the user's home lives on.
///
/// One `statvfs`, taken now. Shifu's disk card is a scan and can be days old;
/// a reading that stale is exactly what §2 forbids, so perception takes its
/// own — the same number the Finder shows, at the moment it is asked.
pub fn disk() -> Option<(u64, u64)> {
    let home = dirs::home_dir()?;
    let c = std::ffi::CString::new(home.as_os_str().as_encoded_bytes()).ok()?;
    // SAFETY: `buf` is fully written by statvfs before we read it, and the
    // path is a NUL-terminated C string that outlives the call.
    unsafe {
        let mut buf: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut buf) != 0 {
            return None;
        }
        let unit = if buf.f_frsize > 0 {
            buf.f_frsize
        } else {
            buf.f_bsize
        } as u64;
        Some((buf.f_bavail as u64 * unit, buf.f_blocks as u64 * unit))
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}

/// The block that rides every turn: what is true here, then whether history is
/// worth opening. `None` when there is nothing to say.
///
/// Total by construction. Perception is additive — an unreadable source is one
/// missing line, never a lost turn, because a status line is a bad reason to
/// cost the user their reply.
///
/// `mark` stamps the doorbell as read. True when this is going into a turn,
/// false when something is merely looking (the `sense` tool) — a glance must
/// not consume "since you last looked" for the turn that follows it.
pub fn block(session_id: Option<&str>, mark: bool) -> Option<String> {
    let lines = read_lines(session_id, mark);
    if lines.is_empty() {
        return None;
    }
    Some(
        lines
            .iter()
            .map(|l| format!("- {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The same readings, unformatted — what this host publishes to the other one
/// (§6) and what `sense` reports, so the block, the tool and the peer's view
/// can never disagree about the machine.
pub fn read_lines(session_id: Option<&str>, mark: bool) -> Vec<String> {
    let mut lines = own_lines();
    lines.extend(peer_lines());
    lines.extend(doorbell(session_id, mark));
    lines
}

/// This machine's own readings — what it measured, without the other host's
/// account of itself folded in.
fn own_lines() -> Vec<String> {
    let mut lines = disk_lines();
    lines.extend(device_lines());
    lines
}

/// What this machine tells the other one about itself (§6).
///
/// Its own readings plus the last thing that happened here — never the peer's
/// lines, which would echo a host's own state back at it, and never a doorbell
/// count, which means nothing to a reader that is not the one who last looked.
pub fn share() -> Vec<String> {
    let mut lines = own_lines();
    if let Some(newest) = super::activity::log().recent().first() {
        let when = newest
            .age_secs()
            .map(super::activity::ago)
            .unwrap_or_default();
        lines.push(format!("latest here: {} · {when}", newest.line()));
    }
    lines
}

fn disk_lines() -> Vec<String> {
    let Some((free, total)) = disk() else {
        return Vec::new();
    };
    let pct = if total > 0 { free * 100 / total } else { 0 };
    let mut line = format!("disk: {} free of {} · {pct}%", gb(free), gb(total));
    if let Some(f) = super::trend::forecast() {
        // Slope, not level — and only when there is a slope worth naming.
        line.push_str(&format!(
            " · filling at {} a day, full in about {} days",
            gb(f.bytes_per_day),
            f.days_until_full
        ));
    }
    vec![line]
}

fn device_lines() -> Vec<String> {
    let here = super::devices::present();
    if !here.is_empty() {
        return vec![format!("connected now: {}", here.join(", "))];
    }
    let paired = crate::server::api::pair::paired_device_count();
    if paired == 0 {
        return vec!["no device paired to this Mac yet".to_string()];
    }
    vec![format!(
        "{paired} paired device{} — none connected right now, so anything that needs one will fail",
        if paired == 1 { "" } else { "s" }
    )]
}

/// The other machine's own perception, as it last published it (§6). Never a
/// synced file: each host writes its own and this is read at the moment it is
/// needed, with its age, so a stale reading can never pass for a fresh one.
fn peer_lines() -> Vec<String> {
    let Some(peer) = read_peer() else {
        return Vec::new();
    };
    let mut out = vec![format!("on {} ({}):", peer.host, peer.age)];
    out.extend(peer.lines.into_iter().map(|l| format!("  {l}")));
    out
}

struct Peer {
    host: String,
    age: String,
    lines: Vec<String>,
}

/// The peer's retained perception payload. Anything malformed reads as no
/// peer at all — a host that cannot be understood contributes nothing, which
/// is the same answer as a host that cannot be reached.
fn read_peer() -> Option<Peer> {
    let path = crate::paths::topics_dir()
        .join(super::publish::TOPIC)
        .join(format!("{}.json", super::publish::FROM_PHONE));
    let text = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let host = v.get("host")?.as_str()?.to_string();
    let lines: Vec<String> = v
        .get("lines")?
        .as_array()?
        .iter()
        .filter_map(|l| l.as_str().map(|s| s.to_string()))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let age = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| super::activity::ago(d.as_secs() as i64))
        .unwrap_or_else(|| "age unknown".to_string());
    Some(Peer { host, age, lines })
}

/// Two lines: whether anything happened, and the newest thing that did.
///
/// That is the whole always-on cost of history. It gives the agent the hook to
/// remark and, most of the time, the answer as well — and when it needs more it
/// calls `recent_activity`, which costs nothing until wanted.
fn doorbell(session_id: Option<&str>, mark: bool) -> Vec<String> {
    let log = super::activity::log();
    let recent = log.recent();
    let Some(newest) = recent.first() else {
        return Vec::new();
    };
    let revision = log.revision();
    let unseen = session_id.map_or(0, |s| revision.saturating_sub(seen_revision(s)));
    if let (Some(s), true) = (session_id, mark) {
        mark_seen(s, revision);
    }
    ring(newest, unseen)
}

fn ring(newest: &super::activity::Activity, unseen: u64) -> Vec<String> {
    let when = newest
        .age_secs()
        .map(super::activity::ago)
        .unwrap_or_default();
    vec![
        if unseen > 0 {
            format!(
                "{unseen} thing{} happened since you last looked",
                if unseen == 1 { "" } else { "s" }
            )
        } else {
            "nothing new since you last looked".to_string()
        },
        format!("most recent: {} · {when}", newest.line()),
        "call recent_activity to see more".to_string(),
    ]
}

/// Per-agent, because "since you last looked" is a fact about a reader, not
/// about the log. Keyed by session so two agents on one Mac each get their own
/// answer without the log storing a cursor per agent.
static SEEN: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

fn seen_revision(session_id: &str) -> u64 {
    let g = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref()
        .and_then(|m| m.get(session_id).copied())
        .unwrap_or(0)
}

/// The resident has now been told. Stamped after a block is built into a turn,
/// so "since you last looked" means since it last had a chance to.
pub fn mark_seen(session_id: &str, revision: u64) {
    let mut g = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new)
        .insert(session_id.to_string(), revision);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perception::activity::Activity;

    fn thing() -> Activity {
        Activity {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            by: "user".into(),
            device: None,
            app: "dj".into(),
            verb: "delete".into(),
            object: Some("三天三夜".into()),
            detail: None,
        }
    }

    #[test]
    fn the_doorbell_is_three_lines_and_never_the_log_itself() {
        let lines = ring(&thing(), 6);
        assert_eq!(lines.len(), 3, "the always-on cost stays fixed: {lines:?}");
        assert_eq!(lines[0], "6 things happened since you last looked");
        assert!(lines[1].starts_with("most recent: you delete 三天三夜 · "));
        assert!(lines[2].contains("recent_activity"), "and names the door");
    }

    #[test]
    fn one_thing_is_singular_and_none_is_said_plainly() {
        assert_eq!(ring(&thing(), 1)[0], "1 thing happened since you last looked");
        assert_eq!(ring(&thing(), 0)[0], "nothing new since you last looked");
    }

    #[test]
    fn since_you_last_looked_is_per_reader() {
        // Two agents on one Mac each get their own answer, without the log
        // carrying a cursor for either.
        mark_seen("sess-a", 7);
        assert_eq!(seen_revision("sess-a"), 7);
        assert_eq!(seen_revision("sess-b"), 0);
    }

    #[test]
    fn the_machine_says_what_it_measured() {
        // A reading, not a claim: whatever this test host is, the line is
        // built from a live statvfs or it is not built at all.
        let lines = disk_lines();
        match disk() {
            Some(_) => {
                assert_eq!(lines.len(), 1);
                assert!(lines[0].starts_with("disk: "), "{lines:?}");
                assert!(lines[0].contains("free of"), "{lines:?}");
            }
            None => assert!(lines.is_empty()),
        }
    }
}
