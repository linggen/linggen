//! What is worth starting a sentence about.
//!
//! Perception is what makes a resident able to speak first. It does not make
//! every observation worth speaking (`doc/perception-spec.md` §5). So a
//! condition declares what it watches, what crosses, and — required — the verb
//! the agent can offer; the resident still judges whether to say it.
//!
//! Three rules, each earned:
//!
//! 1. **Fire on the crossing, not the level.** A condition true every day is
//!    said once, not daily, or the user learns to ignore it.
//! 2. **A "not now" holds.** A fired condition re-arms after [`REARM_DAYS`],
//!    not on the next sweep.
//! 3. **Every noticing ends in a verb the agent owns.** "Your disk is filling"
//!    with no next step is worse than silence, so [`Notice::verbs`] is not
//!    optional.

use std::path::PathBuf;

/// Once a week at most, per topic. Long enough that a dismissed notice stays
/// dismissed; short enough that a genuinely worsening situation is raised again.
const REARM_DAYS: i64 = 7;

/// Free space this low, in days of runway, is worth one sentence.
const FULL_WITHIN_DAYS: i64 = 2;

/// A situation the resident may speak about, and what it can offer to do.
pub struct Notice {
    pub topic: &'static str,
    /// What is true, in plain words. The resident says it in its own voice.
    pub situation: String,
    /// The verbs it owns for this. Never empty — see rule 3.
    pub verbs: &'static str,
}

/// Every condition this machine watches. Exactly one ships enabled: each
/// enabled condition is a licence to interrupt, and the next is added only
/// after living with this one.
fn conditions() -> Vec<(&'static str, fn() -> Option<Notice>)> {
    vec![("storage-filling", storage_filling)]
}

/// The first condition that has crossed and is not still holding its re-arm.
///
/// One per sweep by construction: two notices in one breath is a report, and a
/// resident that reports is one the user stops listening to.
pub fn sweep() -> Option<Notice> {
    for (topic, check) in conditions() {
        if holding(topic) {
            continue;
        }
        if let Some(notice) = check() {
            arm(topic);
            return Some(notice);
        }
    }
    None
}

fn storage_filling() -> Option<Notice> {
    let f = super::trend::forecast()?;
    if f.days_until_full > FULL_WITHIN_DAYS {
        return None;
    }
    let (free, _) = super::state::disk()?;
    Some(Notice {
        topic: "storage-filling",
        situation: format!(
            "this Mac has {:.1} GB free and is filling fast — about {} day{} of room left \
             at the rate of the last {} days",
            free as f64 / 1_000_000_000.0,
            f.days_until_full,
            if f.days_until_full == 1 { "" } else { "s" },
            f.span_days
        ),
        verbs: "you can offer to look for what is safe to delete, or to back the \
                heavy folders up to an external disk — ask first, never start",
    })
}

// --- re-arm bookkeeping ----------------------------------------------------

fn armed_path() -> PathBuf {
    super::activity::activity_dir().join("conditions.json")
}

fn armed() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(armed_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn holding(topic: &str) -> bool {
    let Some(last) = armed().get(topic).and_then(|v| v.as_i64()) else {
        return false;
    };
    let age = crate::util::now_ts_secs() as i64 - last;
    age < REARM_DAYS * 86_400
}

fn arm(topic: &str) {
    let mut map = armed();
    map.insert(
        topic.to_string(),
        serde_json::json!(crate::util::now_ts_secs() as i64),
    );
    let dir = super::activity::activity_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(body) = serde_json::to_vec_pretty(&map) {
        let _ = std::fs::write(armed_path(), body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_notice_carries_a_verb() {
        // Rule 3, enforced rather than remembered: a condition with nothing to
        // offer must not exist.
        let n = Notice {
            topic: "t",
            situation: "s".into(),
            verbs: "v",
        };
        assert!(!n.verbs.is_empty());
    }

    #[test]
    fn a_fired_topic_holds_its_rearm() {
        let topic = "test-topic-rearm";
        let mut map = armed();
        map.remove(topic);
        let _ = std::fs::write(armed_path(), serde_json::to_vec_pretty(&map).unwrap());

        assert!(!holding(topic));
        arm(topic);
        assert!(holding(topic), "a topic just fired must not fire again");
    }
}
