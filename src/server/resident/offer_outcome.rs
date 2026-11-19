//! What a tapped offer came to, said by her — the result turn.
//!
//! The person tapped one of her offers; it ran as their button, and the
//! route's answer comes here as facts. She says what was done, or what
//! stopped it, from that result alone. It is an answer to the person, so it
//! runs as an asked turn: outside the budget of unprompted lines, and quiet
//! (it speaks, it never acts — the tap already did the doing).

use std::sync::Arc;

use super::{Origin, Reach, ServerState};

/// The most of a tool's result she is shown.
const RESULT_MAX: usize = 600;

/// The kickoff: the facts of the tap and its result, and the honest-outcome
/// rule as a fact of the turn.
pub(super) fn kickoff(label: &str, answer: &str, result: &str) -> String {
    let result: String = result.chars().take(RESULT_MAX).collect();
    format!(
        "The person tapped your offer \"{label}\". It came to: {answer}. What it returned: {result}. \
         Say the outcome to them in one short line, from this result only: what was done, or what \
         stopped it. Never say more was done than the result says. This turn cannot act."
    )
}

/// Say it. Spawned: the tap's answer does not wait on her voice.
pub(crate) fn say(state: &Arc<ServerState>, label: String, answer: String, result: String) {
    let state = state.clone();
    tokio::spawn(async move {
        let kickoff = kickoff(&label, &answer, &result);
        super::triggers::wake_and_speak(state, (kickoff, None), "neutral", Origin::Asked, Reach::SpeakOnly, None)
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_turn_is_an_answer_outside_the_budget_and_cannot_act() {
        assert_eq!(super::super::triggers::unprompted_source(Reach::SpeakOnly, Origin::Asked), None);
        let k = kickoff("Look what's big", "done", "{\"exit_code\":0,\"stdout\":\"DISK 12 GB free\"}");
        assert!(k.contains("Look what's big") && k.contains("DISK 12 GB free"));
        assert!(k.contains("cannot act"));
        let long = "x".repeat(5000);
        assert!(kickoff("l", "done", &long).len() < 1200, "a result is bounded");
    }
}
