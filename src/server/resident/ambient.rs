//! Ambient life-signs — she glances at the day on a jittered cadence and,
//! now and then, says one small unprompted thing. Mostly she stays quiet.

use super::*;
use super::triggers::wake_and_speak;

/// Base wake interval — she wakes *occasionally*, not every few minutes.
pub(super) const AMBIENT_BASE_SECS: u64 = 1800; // ~30 min
/// Added jitter (0..=this) so a remark never lands on the dot.
pub(super) const AMBIENT_JITTER_SECS: u64 = 1200; // up to ~20 min
/// Settle after boot before the first glance.
pub(super) const AMBIENT_STARTUP_SECS: u64 = 240;

/// A condition came due while no Linggen page was in front: it waits, and
/// the first page to come to the front makes one glance of its own.
static WAITING_FOR_A_PAGE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How often the loop looks for a page coming back while a condition waits:
/// a read of the presence board, no model.
const PAGE_TICK_SECS: u64 = 5;

/// A test's knob: the glance cadence and its settle, in seconds, no jitter.
fn cadence_override() -> Option<u64> {
    std::env::var("LINGGEN_AMBIENT_SECS").ok()?.parse().ok()
}

/// Yinyue's ambient loop — a sibling to the event-reactive watch. On a jittered
/// ~30–50 min cadence she glances at the day; most glances she stays silent, now
/// and then she says one small thing in her own voice. Not a mission (no run-
/// history clutter); the Pet master switch disables it.
pub async fn yinyue_ambient_loop(state: Arc<ServerState>) {
    tracing::info!("[yinyue-ambient] started");
    let fixed = cadence_override();
    tokio::time::sleep(Duration::from_secs(fixed.unwrap_or(AMBIENT_STARTUP_SECS))).await;
    loop {
        let jitter = crate::util::now_ts_secs() % (AMBIENT_JITTER_SECS + 1);
        let wait = fixed.unwrap_or(AMBIENT_BASE_SECS + jitter);
        until_next_glance(&state, wait).await;
        ambient_glance(&state).await;
    }
}

/// The cadence runs out — or a condition is waiting and a page comes to the
/// front, which ends the wait early, once.
async fn until_next_glance(state: &Arc<ServerState>, secs: u64) {
    use std::sync::atomic::Ordering;
    let mut left = secs;
    while left > 0 {
        let step = left.min(PAGE_TICK_SECS);
        tokio::time::sleep(Duration::from_secs(step)).await;
        left -= step;
        if WAITING_FOR_A_PAGE.load(Ordering::Relaxed) && super::triggers::a_page_is_in_front(state) {
            return;
        }
    }
}

/// One ambient glance. She reads the room and decides whether a small remark
/// fits — most of the time, nothing. Never narrates the user's work.
///
/// This is also when the machine's conditions are swept
/// (`doc/perception-spec.md` §5). Deliberately here rather than on a timer of
/// its own: a notice is only worth raising at a moment she could speak anyway,
/// and a condition firing into a silent house is work nobody can hear.
pub(super) async fn ambient_glance(state: &Arc<ServerState>) {
    // This glance is the attempt a waiting condition was owed. Cleared first,
    // before any early return: a glance that is blocked (pet off, quiet hours,
    // the hour's ceiling) goes back to the normal cadence, not to a re-glance
    // every few seconds while a page stays in front.
    WAITING_FOR_A_PAGE.store(false, std::sync::atomic::Ordering::Relaxed);
    let pet = state.manager.get_config_snapshot().await.pet;
    if !pet.enabled || !here(state).await {
        return; // pet off, or she isn't met yet → no ambient life
    }
    // Quiet hours, or the hour's ceiling reached: no glance at all — and the
    // machine's conditions wait for the next one, unspent.
    if super::guard::held_now(&pet, super::guard::Source::Glance).is_some() {
        return;
    }
    if let Some(notice) = crate::perception::conditions::sweep() {
        tracing::info!("[yinyue-ambient] condition crossed: {}", notice.topic);
        // A condition is for a person who can see it: with no Linggen page in
        // front there is no model call, nothing told, and it stays due.
        if !super::triggers::a_page_is_in_front(state) {
            WAITING_FOR_A_PAGE.store(true, std::sync::atomic::Ordering::Relaxed);
            tracing::info!("[yinyue-ambient] no page in front — {} waits", notice.topic);
            return;
        }
        // Published only with its row kept: while a turn runs in her thread
        // the row cannot land, so no model call is spent on it this glance —
        // it stays due for the next one.
        let stage = super::spoken::Stage::of(state, None).await;
        if !stage.row_is_free(state) {
            tracing::info!("[yinyue-ambient] her thread is busy — {} waits", notice.topic);
            return;
        }
        let kickoff = format!(
            "Something crossed a line worth one sentence: {}. {}. \
             You noticed it — say so in your own voice, one short line, plain prose. \
             Nothing runs from this turn: an offer runs only when the person taps it.",
            notice.situation, notice.remedy.facts()
        );
        // It is told when its line is published with a page in front, and
        // there (`wake_and_offer`): a silent reply, a failed turn or a
        // departed page leaves it due for the next glance.
        let raised = super::triggers::Raised {
            problem: &notice.topic,
            offers: notice.remedy.offers(),
            notice: &notice,
        };
        let _ = super::triggers::wake_and_offer(
            state.clone(),
            (kickoff, None),
            ("neutral", Origin::Glance),
            Reach::SpeakOnly,
            None,
            Some(raised),
        )
        .await;
        if !super::triggers::a_page_is_in_front(state) {
            WAITING_FOR_A_PAGE.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        return;
    }
    let kickoff = "A quiet moment — no event, just you. Right now tells you how things are; \
        read it and decide. Most of the time there's nothing worth saying. \
        Only now and then, if a small natural remark genuinely fits — the hour, how the day's \
        gone, the quiet, your own mood — say ONE short line in your voice. \
        This is just you being present, never a report: never narrate their work or what \
        an agent did. If they're heads-down working, let them be. A line marked already told is \
        not news. This turn can speak, never act: nothing you say here is done, sent, or handed to anyone — \
        not to Ling, not to the phone. When in doubt, say nothing."
        .to_string();
    wake_and_speak(state.clone(), (kickoff, None), "neutral", Origin::Glance, Reach::SpeakOnly, None).await;
}
