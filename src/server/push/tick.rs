//! Once a minute: every waiting line, every phone that registered. A line
//! goes to a phone when it is due, the phone has not told it, the phone's
//! herald has the line on, and the day's budget is not spent. Her words are
//! written once per language and kept on the line, so a restart or a second
//! phone never asks her again.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use crate::engine::skill::NoticeTier;
use crate::server::ServerState;

use super::relay::{self, Ring, Sent};
use super::store::{self, LastPush, Pending, Phone};
use super::{compose, seal, timing};

const TICK_SECS: u64 = 60;

#[derive(Debug, PartialEq)]
pub(crate) enum Step {
    /// Not yet: not due, or waiting out a refused send.
    Wait,
    /// Never for this phone: told already, the line is off, the day is spent.
    Skip,
    Go,
}

/// Whether [p] goes to [phone] now. Pure.
pub(crate) fn step(p: &Pending, phone: &Phone, device: &str, now: u64) -> Step {
    if phone.told.contains_key(&p.told_key()) {
        return Step::Skip;
    }
    // A line the phone does not list is one its herald does not know: it
    // would show, unasked for. Off is off.
    if phone.lines.get(&p.herald_id()) != Some(&true) {
        return Step::Skip;
    }
    if now < timing::due_at(p.tier, p.raised_at, &phone.morning) {
        return Step::Wait;
    }
    if p.retry_at.get(device).is_some_and(|t| now < *t) {
        return Step::Wait;
    }
    let spent = phone.spent.get(&timing::day_of(now)).copied().unwrap_or(0);
    if p.tier != NoticeTier::Warning && spent >= timing::DAY_BUDGET {
        return Step::Skip;
    }
    Step::Go
}

/// The note as the phone's extension reads it (doc/phone-push-spec.md).
pub(crate) fn plaintext(p: &Pending, title: &str, text: &str, now: u64) -> serde_json::Value {
    serde_json::json!({
        "v": 1,
        "app": p.app,
        "line": p.line,
        "key": p.key,
        "told": p.told_key(),
        "title": title,
        "text": text,
        "route": p.route,
        "tier": p.tier,
        "at": now,
    })
}

pub(crate) async fn run(state: Arc<ServerState>) {
    tracing::info!("[push] tick started");
    loop {
        tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
        tick(&state).await;
    }
}

async fn tick(state: &Arc<ServerState>) {
    let now = crate::util::now_ts_secs();
    let paired: BTreeSet<String> = crate::server::api::pair::load_devices()
        .into_iter()
        .map(|d| d.id)
        .collect();
    let snapshot = store::modify(|s| {
        s.queue.retain(|p| p.until > now);
        // An unpaired phone takes its registration with it.
        s.phones.retain(|id, _| paired.contains(id));
        s.clone()
    });
    for p in &snapshot.queue {
        for (device, phone) in &snapshot.phones {
            if step(p, phone, device, now) == Step::Go {
                ring(state, p, device, phone, now).await;
            }
        }
    }
}

async fn ring(state: &Arc<ServerState>, p: &Pending, device: &str, phone: &Phone, now: u64) {
    let told = p.told_key();
    let (title, text) = match her_words(state, p, &phone.language).await {
        Ok(w) => w,
        Err(e) => {
            tracing::info!(
                "[push] {told}: no words yet ({e}); next try in {} min",
                timing::RETRY_SECS / 60
            );
            return wait(&told, device, now, "no_words");
        }
    };
    let Some(key) = seal::key_of(&phone.key) else {
        return wait(&told, device, now, "bad_key");
    };
    let note = plaintext(p, &title, &text, now).to_string();
    let sealed = match seal::seal(&key, note.as_bytes()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[push] {told}: {e}");
            return wait(&told, device, now, "too_long");
        }
    };
    let sent = relay::send(&Ring {
        token: &phone.token,
        sealed: &sealed,
        collapse_id: &relay::collapse_id(&told),
        priority: timing::priority(p.tier),
        expires_at: p.until,
        fallback: &phone.fallback,
    })
    .await;
    tracing::info!("[push] {told} → {device}: {sent:?}");
    match sent {
        Sent::Delivered => delivered(&told, p.tier, device, now),
        Sent::Gone => {
            store::modify(|s| s.phones.remove(device));
        }
        Sent::Later(code) => wait(&told, device, now, &code),
    }
}

/// Her words in [language], written once and kept on the line.
async fn her_words(
    state: &Arc<ServerState>,
    p: &Pending,
    language: &str,
) -> Result<(String, String), String> {
    if let Some(w) = p.words.get(language) {
        return Ok(w.clone());
    }
    let skill = state.skills.get_skill(&p.app).await.ok_or("skill gone")?;
    let decl = skill
        .notices
        .iter()
        .find(|n| n.line == p.line)
        .ok_or("line no longer declared")?;
    let words = compose::write(state, &skill, decl, language, &p.facts).await?;
    let told = p.told_key();
    store::modify(|s| {
        if let Some(q) = s.queue.iter_mut().find(|q| q.told_key() == told) {
            q.words.insert(language.to_string(), words.clone());
        }
    });
    Ok(words)
}

fn delivered(told: &str, tier: NoticeTier, device: &str, now: u64) {
    store::modify(|s| {
        let Some(phone) = s.phones.get_mut(device) else {
            return;
        };
        phone.told.insert(told.to_string(), now);
        if tier != NoticeTier::Warning {
            *phone.spent.entry(timing::day_of(now)).or_default() += 1;
        }
        phone.last = Some(LastPush {
            at: now,
            told: told.to_string(),
            outcome: "sent".into(),
        });
        store::prune(phone, now);
    });
}

fn wait(told: &str, device: &str, now: u64, why: &str) {
    store::modify(|s| {
        if let Some(q) = s.queue.iter_mut().find(|q| q.told_key() == told) {
            q.retry_at
                .insert(device.to_string(), now + timing::RETRY_SECS);
        }
        if let Some(phone) = s.phones.get_mut(device) {
            phone.last = Some(LastPush {
                at: now,
                told: told.to_string(),
                outcome: why.to_string(),
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(tier: NoticeTier, raised: u64) -> Pending {
        Pending {
            app: "cfo".into(),
            line: "watch".into(),
            key: "watch|2026-10-27".into(),
            tier,
            facts: serde_json::json!([]),
            route: Some("cfo/invest".into()),
            raised_at: raised,
            until: raised + 86400,
            words: Default::default(),
            retry_at: Default::default(),
        }
    }

    fn phone() -> Phone {
        let mut p = Phone {
            morning: "07:00".into(),
            ..Default::default()
        };
        p.lines.insert("cfo.watch".into(), true);
        p
    }

    const RAISED: u64 = 1_800_000_000;
    const LATER: u64 = RAISED + 2 * 86400;

    #[test]
    fn a_due_untold_line_that_is_on_goes() {
        assert_eq!(
            step(
                &pending(NoticeTier::Hand, RAISED),
                &phone(),
                "d",
                RAISED + 121
            ),
            Step::Go
        );
    }

    #[test]
    fn not_before_it_is_due() {
        assert_eq!(
            step(
                &pending(NoticeTier::Hand, RAISED),
                &phone(),
                "d",
                RAISED + 60
            ),
            Step::Wait
        );
    }

    #[test]
    fn a_line_the_phone_told_never_goes() {
        let mut ph = phone();
        ph.told.insert("cfo.watch:watch|2026-10-27".into(), RAISED);
        assert_eq!(
            step(&pending(NoticeTier::Record, RAISED), &ph, "d", LATER),
            Step::Skip
        );
    }

    #[test]
    fn a_line_switched_off_or_unknown_to_the_phone_never_goes() {
        let mut ph = phone();
        ph.lines.insert("cfo.watch".into(), false);
        assert_eq!(
            step(&pending(NoticeTier::Record, RAISED), &ph, "d", LATER),
            Step::Skip
        );
        ph.lines.clear();
        assert_eq!(
            step(&pending(NoticeTier::Record, RAISED), &ph, "d", LATER),
            Step::Skip
        );
    }

    #[test]
    fn a_refused_send_waits_its_turn() {
        let mut p = pending(NoticeTier::Hand, RAISED);
        p.retry_at.insert("d".into(), RAISED + 900);
        assert_eq!(step(&p, &phone(), "d", RAISED + 300), Step::Wait);
        assert_eq!(step(&p, &phone(), "other", RAISED + 300), Step::Go);
        assert_eq!(step(&p, &phone(), "d", RAISED + 901), Step::Go);
    }

    #[test]
    fn two_a_day_then_only_a_warning() {
        let mut ph = phone();
        ph.lines.insert("cfo.alarm".into(), true);
        ph.spent.insert(timing::day_of(LATER), 2);
        assert_eq!(
            step(&pending(NoticeTier::Hand, RAISED), &ph, "d", LATER),
            Step::Skip
        );
        let mut warn = pending(NoticeTier::Warning, RAISED);
        warn.line = "alarm".into();
        assert_eq!(step(&warn, &ph, "d", LATER), Step::Go);
    }

    #[test]
    fn the_note_carries_the_told_key_and_her_words() {
        let n = plaintext(&pending(NoticeTier::Record, RAISED), "早", "AAPL 涨了", 5);
        assert_eq!(n["told"], "cfo.watch:watch|2026-10-27");
        assert_eq!(n["tier"], "record");
        assert_eq!(n["title"], "早");
        assert_eq!(n["route"], "cfo/invest");
        assert_eq!(n["v"], 1);
    }
}
