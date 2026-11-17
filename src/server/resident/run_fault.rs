//! A run that failed, as a fact she words — never a sentence of the engine's.
//!
//! Another agent's run failing is handed to her at once, as a fault
//! ([`super::triggers::wake_fault`]). Her own run failing cannot be: waking
//! her to say her mind is unreachable runs the very thing that failed, and
//! loops. So that fact is held, one at a time, and rides the next unprompted
//! turn on her own thread, beside whatever woke it. While her model stays
//! down nothing is said at all — the failed run's own chat shows its error. Held is never dropped: a turn
//! that fails again holds it again. The hold lives in memory; a restart loses
//! it, and the failed run's own record stays in its chat.

use std::sync::Mutex;

static HELD: Mutex<Option<String>> = Mutex::new(None);

/// The facts of a failed run, for her. `auth_required`: it failed for want
/// of a sign-in — the fix is the person's, in Settings.
pub(super) fn fact(agent_id: &str, auth_required: bool, her_own: bool) -> String {
    let who = if her_own {
        "one of your own turns".to_string()
    } else {
        format!("a run by the agent \"{agent_id}\"")
    };
    let why = if auth_required {
        "it needs a sign-in; the person signs in at Settings › Account"
    } else {
        "the reason is in that chat; nothing here can retry it"
    };
    format!("Something went wrong behind the scenes: {who} failed — {why}.")
}

/// Hold the fact for her next woken turn. The newest wins: one failure to
/// tell, not a list of them.
pub(super) fn hold(fact: String) {
    *HELD.lock().unwrap_or_else(|e| e.into_inner()) = Some(fact);
}

/// The kickoff with the held fact before it, and the fact taken — handed
/// back with [`hold`] when the turn it rode produced nothing.
pub(super) fn ride(kickoff: String) -> (String, Option<String>) {
    let held = HELD.lock().unwrap_or_else(|e| e.into_inner()).take();
    match held {
        Some(f) => (format!("{f} Say it in your line, in your own words. {kickoff}"), Some(f)),
        None => (kickoff, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_run_is_facts_she_words_never_a_line_of_the_engines() {
        let f = fact("ling", false, false);
        assert!(f.contains("\"ling\""), "{f}");
        let auth = fact("yinyue", true, true);
        assert!(auth.contains("one of your own turns") && auth.contains("Settings › Account"));
        // The four rotated apologies and the auth line are gone from the
        // engine: no sentence a person reads is written here.
        let triggers = include_str!("triggers.rs");
        for gone in [
            "I'm sorry — something just went wrong",
            "Something stumbled just now",
            "my apologies. I'm keeping an eye on it",
            "Something's off just now",
            "I can't reach my mind right now",
        ] {
            assert!(!triggers.contains(gone), "{gone}");
        }
    }

    /// The quiet glance is told what the turn can do, as a fact of the turn:
    /// it speaks, never acts, so she never says something was handed to Ling
    /// (2026-11-17: "已经托 Ling 看看", twice, from a turn that cannot).
    #[test]
    fn a_quiet_glance_is_told_it_cannot_act_or_hand_off() {
        let ambient = include_str!("ambient.rs");
        assert!(!ambient.contains("if something needs doing, say so"));
        assert!(ambient.contains("not to Ling, not to the phone"));
    }

    #[test]
    fn a_held_fault_rides_the_next_turn_and_is_never_dropped() {
        hold("F1.".into());
        hold("F2.".into());
        let (k, taken) = ride("Kick.".into());
        assert!(k.starts_with("F2.") && k.ends_with("Kick."), "{k}");
        assert_eq!(ride("Next.".into()).1, None, "taken once");
        // The turn it rode said nothing: held again for the one after.
        hold(taken.unwrap());
        assert!(ride("Again.".into()).0.starts_with("F2."));
    }
}
