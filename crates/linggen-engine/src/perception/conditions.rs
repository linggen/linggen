//! What is worth starting a sentence about.
//!
//! Perception is what makes a resident able to speak first. It does not make
//! every observation worth speaking (`doc/perception-spec.md` §5). So a
//! condition declares what it watches, what crosses, how bad it is (a level),
//! and — required — its [`Remedy`]: the offers the person may tap, or why
//! there are none. The resident still judges when to say it.
//!
//! Three rules, each earned:
//!
//! 1. **Said once.** A problem is raised once, and again only when it is
//!    materially worse or went away and came back — never because time
//!    passed. The record is [`super::told`], kept by the device the problem is
//!    about; the other device's problems are raised from what it published,
//!    against its record and this Mac's report of having said them.
//! 2. **Never dropped.** A problem nobody has told is due at every glance until
//!    it is said. The record starts when the resident **speaks**, never when it
//!    is asked: a SILENT reply defers, it does not dismiss.
//! 3. **Every noticing carries what can be done, or why nothing can.** "Your
//!    disk is filling" with no next step is worse than silence, so a
//!    [`Notice`] holds a [`Remedy`], which cannot be built empty. The offers
//!    are facts; the words — her line and each button's label — are hers.

use super::offer::Remedy;
use super::told::{Ledger, PeerProblem, Standing};

/// Free space this low, in days of runway, is worth one sentence.
const FULL_WITHIN_DAYS: i64 = 2;

/// A situation the resident may speak about, and what can be done about it.
pub struct Notice {
    /// The problem's name in the ledger: this Mac's own, or `<owner>/<name>`
    /// for one the other device reported.
    pub topic: String,
    /// How bad it is, as its owner measures it. Higher is worse.
    pub level: u32,
    /// What is true, in plain words — and whether it was told before. The
    /// resident says it in its own voice.
    pub situation: String,
    /// The offers, or why there are none — see rule 3.
    pub remedy: Remedy,
    /// The device whose problem this is, when it is not this Mac.
    owner: Option<(String, String)>,
}

/// A condition's finding: its level and the facts behind it.
struct Finding {
    level: u32,
    situation: String,
    remedy: Remedy,
}

/// Every condition this machine watches. Exactly one ships enabled: each
/// enabled condition is a licence to interrupt, and the next is added only
/// after living with this one.
fn conditions() -> Vec<(&'static str, fn() -> Option<Finding>)> {
    vec![(STORAGE_FILLING, storage_filling)]
}

/// The Mac's disk running out of room — the key skills declare offers under.
pub const STORAGE_FILLING: &str = "storage-filling";

/// The first problem that is due — this Mac's own first, then the other
/// device's — or none.
///
/// One per sweep by construction: two notices in one breath is a report, and a
/// resident that reports is one the user stops listening to.
///
/// Finding a notice does **not** record it as told — [`spoken`] does. The
/// resident may answer SILENT, or its run may fail, and a problem recorded as
/// told that nobody heard is how a machine two days from full says nothing at
/// all. The retry costs nothing: the glance it rides was going to run a turn
/// either way.
pub fn sweep() -> Option<Notice> {
    let now = crate::util::now_ts_secs() as i64;
    let mut ledger = super::told::load();
    let own: Vec<(&'static str, Option<Finding>)> = conditions()
        .into_iter()
        .map(|(t, check)| (t, check()))
        .collect();
    let peer = super::state::peer_problems();
    let notice = choose(&mut ledger, own, peer, now);
    super::told::store(&ledger);
    notice
}

/// The decision, without the disk: readings in, the due notice out, the
/// ledger brought up to date with what was read.
fn choose(
    ledger: &mut Ledger,
    own: Vec<(&'static str, Option<Finding>)>,
    peer: Option<(String, Option<Vec<PeerProblem>>)>,
    now: i64,
) -> Option<Notice> {
    let mut due = None;
    for (topic, found) in own {
        ledger.reading(topic, found.is_some(), now);
        let Some(f) = found else { continue };
        let standing = super::told::standing(f.level, ledger.get(topic));
        if due.is_none() && standing.due() {
            due = Some(own_notice(topic, f, &standing, now));
        }
    }
    let owner = super::publish::FROM_PHONE;
    let Some((host, problems)) = peer else {
        return due;
    };
    ledger.prune_reports(owner, problems.as_deref());
    if due.is_some() {
        return due;
    }
    let problems = problems.unwrap_or_default();
    let (p, standing) = ledger.peer_due(owner, &problems)?;
    Some(peer_notice(&host, owner, p, &standing, now))
}

/// What she is told when the other device's problem is raised here: nothing
/// on this Mac can change it, and its offers run on that device.
pub const ON_THE_OTHER_DEVICE: &str =
    "it is on that device, not this Mac — what can be done is offered there, where it runs";

/// What she is told when no installed skill offers anything for a problem of
/// this Mac's.
pub const NOTHING_INSTALLED_OFFERS: &str =
    "no app installed on this Mac offers a fix for it — System Settings › General › Storage shows the person what holds the space";

/// The other device's problem, as this Mac may raise it. It is that device's
/// to fix, so the remedy is the honest one: none here, and why.
fn peer_notice(host: &str, owner: &str, p: &PeerProblem, standing: &Standing, now: i64) -> Notice {
    Notice {
        topic: format!("{owner}/{}", p.key),
        level: p.level,
        situation: format!("on {host}: {} ({})", p.line, standing.words(now)),
        remedy: Remedy::of(Vec::new(), ON_THE_OTHER_DEVICE),
        owner: Some((owner.to_string(), p.key.clone())),
    }
}

fn own_notice(topic: &str, f: Finding, standing: &Standing, now: i64) -> Notice {
    Notice {
        topic: topic.to_string(),
        level: f.level,
        situation: format!("{} ({})", f.situation, standing.words(now)),
        remedy: f.remedy,
        owner: None,
    }
}

/// The resident said it out loud. Now it is told, at the level it was said
/// at — rule 1 is about what the user heard, not about what was noticed.
pub fn spoken(notice: &Notice) {
    let now = crate::util::now_ts_secs() as i64;
    let mut ledger = super::told::load();
    record(&mut ledger, notice, now);
    super::told::store(&ledger);
}

fn record(ledger: &mut Ledger, notice: &Notice, now: i64) {
    match &notice.owner {
        Some((owner, key)) => ledger.reported(owner, key, notice.level, now),
        None => ledger.said(&notice.topic, notice.level, "mac", now),
    }
}

/// Room left, in days, as a level: two days is the first, and each halving
/// after it is worse — one day, then none.
fn runway_level(days: i64) -> u32 {
    match days {
        d if d > FULL_WITHIN_DAYS => 0,
        d if d > 1 => 1,
        1 => 2,
        _ => 3,
    }
}

fn storage_filling() -> Option<Finding> {
    let f = super::trend::forecast()?;
    let level = runway_level(f.days_until_full);
    if level == 0 {
        return None;
    }
    let (free, _) = super::state::disk()?;
    Some(Finding {
        level,
        situation: format!(
            "this Mac has {:.1} GB free and is filling fast — about {} day{} of room left \
             at the rate of the last {} days",
            free as f64 / 1_000_000_000.0,
            f.days_until_full,
            if f.days_until_full == 1 { "" } else { "s" },
            f.span_days
        ),
        remedy: Remedy::of(
            super::offer::declared_for(STORAGE_FILLING),
            NOTHING_INSTALLED_OFFERS,
        ),
    })
}

#[cfg(test)]
mod tests;
