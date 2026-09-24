//! Ambient life-signs — she glances at the day on a jittered cadence and,
//! now and then, says one small unprompted thing. Mostly she stays quiet.

use super::*;

/// Base wake interval — she wakes *occasionally*, not every few minutes.
pub(super) const AMBIENT_BASE_SECS: u64 = 1800; // ~30 min
/// Added jitter (0..=this) so a remark never lands on the dot.
pub(super) const AMBIENT_JITTER_SECS: u64 = 1200; // up to ~20 min
/// Settle after boot before the first glance.
pub(super) const AMBIENT_STARTUP_SECS: u64 = 240;

/// Yinyue's ambient loop — a sibling to the event-reactive watch. On a jittered
/// ~30–50 min cadence she glances at the day; most glances she stays silent, now
/// and then she says one small thing in her own voice. Not a mission (no run-
/// history clutter); the Pet master switch disables it.
pub async fn yinyue_ambient_loop(state: Arc<ServerState>) {
    tracing::info!("[yinyue-ambient] started");
    tokio::time::sleep(Duration::from_secs(AMBIENT_STARTUP_SECS)).await;
    loop {
        let jitter = crate::util::now_ts_secs() % (AMBIENT_JITTER_SECS + 1);
        tokio::time::sleep(Duration::from_secs(AMBIENT_BASE_SECS + jitter)).await;
        ambient_glance(&state).await;
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
    if !state.manager.get_config_snapshot().await.pet.enabled {
        return; // pet off → no ambient life
    }
    if let Some(notice) = crate::perception::conditions::sweep() {
        tracing::info!("[yinyue-ambient] condition crossed: {}", notice.topic);
        let kickoff = format!(
            "Something about this machine crossed a line worth one sentence: {}. \
             You noticed it — say so in your own voice, one short line, spoken aloud, \
             plain prose, and offer the next step: {}. Never start anything yourself. \
             If this truly isn't the moment, reply with exactly SILENT.",
            notice.situation, notice.verbs
        );
        // The re-arm starts only if she actually said it. A SILENT reply is
        // her judging the moment, not the user being told — and this condition
        // fires two days before the disk is full.
        if wake_herald(state.clone(), kickoff, "neutral").await {
            crate::perception::conditions::spoken(notice.topic);
        }
        return;
    }
    let kickoff = "A quiet moment — no event, just you. Right now tells you how things are; \
        read it and decide. Most of the time there's nothing worth saying — reply with exactly SILENT. \
        Only now and then, if a small natural remark genuinely fits — the hour, how the day's \
        gone, the quiet, your own mood — say ONE short line in your voice, spoken aloud, plain \
        prose. This is just you being present, never a report: never narrate their work or what \
        an agent did. If they're heads-down working, let them be. Don't repeat what you said last \
        time. When in doubt, SILENT."
        .to_string();
    wake_herald(state.clone(), kickoff, "neutral").await;
}
