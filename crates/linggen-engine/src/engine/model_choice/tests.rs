//! The rule, one row per case.

use super::*;

fn models() -> ModelManager {
    // The built-ins only: gpt-6.2-sol, gpt-6.1-terra, gpt-6-luna, cloud.
    ModelManager::new_with_credentials(vec![], &crate::credentials::Credentials::default())
}

const STANDING: &str = "gpt-6-luna";

fn owner(may_switch: bool) -> Rule<'static> {
    Rule {
        asker: Asker::Owner,
        may_switch,
    }
}

fn guest(shared: &[String], may_switch: bool) -> Rule<'_> {
    Rule {
        asker: Asker::Guest(shared),
        may_switch,
    }
}

#[test]
fn the_rule_one_row_per_case() {
    let m = models();
    let shared = vec!["gpt-6-sol".to_string()]; // a retired spelling of sol
    let none: Vec<String> = vec![];
    let rows: Vec<(&str, Option<&str>, Rule<'_>, &str, Outcome)> = vec![
        (
            "owner, asked, here",
            Some("gpt-6.1-terra"),
            owner(true),
            "gpt-6.1-terra",
            Outcome::Asked,
        ),
        (
            "owner, asked a retired id",
            Some("gpt-6-sol"),
            owner(true),
            "gpt-6.2-sol",
            Outcome::Asked,
        ),
        (
            "owner, asked, not here",
            Some("ghost"),
            owner(true),
            STANDING,
            Outcome::Replaced(Why::NotHere),
        ),
        (
            "owner, asked, not here, switching off",
            Some("ghost"),
            owner(false),
            "ghost",
            Outcome::Kept(Why::NotHere),
        ),
        (
            "owner, nothing asked",
            None,
            owner(true),
            STANDING,
            Outcome::Standing,
        ),
        (
            "owner, nothing asked, switching off",
            None,
            owner(false),
            STANDING,
            Outcome::Standing,
        ),
        (
            "guest, asked, shared",
            Some("gpt-6.2-sol"),
            guest(&shared, true),
            "gpt-6.2-sol",
            Outcome::Asked,
        ),
        (
            "guest, asked, not shared",
            Some("gpt-6.1-terra"),
            guest(&shared, true),
            "gpt-6.2-sol",
            Outcome::Replaced(Why::NotShared),
        ),
        (
            "guest, asked, not shared, switching off",
            Some("gpt-6.1-terra"),
            guest(&shared, false),
            "gpt-6.2-sol",
            Outcome::Replaced(Why::NotShared),
        ),
        (
            "guest, asked, not here",
            Some("ghost"),
            guest(&shared, true),
            "gpt-6.2-sol",
            Outcome::Replaced(Why::NotHere),
        ),
        (
            "guest, nothing asked",
            None,
            guest(&shared, true),
            "gpt-6.2-sol",
            Outcome::Standing,
        ),
        (
            "guest, room shares nothing, asked",
            Some("gpt-6.1-terra"),
            guest(&none, true),
            STANDING,
            Outcome::Replaced(Why::NothingShared),
        ),
        (
            "guest, room shares nothing, nothing asked",
            None,
            guest(&none, true),
            STANDING,
            Outcome::Replaced(Why::NothingShared),
        ),
    ];
    for (case, asked, rule, model, outcome) in rows {
        let choice = resolve(&m, asked, STANDING, rule);
        assert_eq!(
            (choice.model(), choice.outcome()),
            (model, outcome),
            "{case}"
        );
        assert_eq!(choice.is_asked(), outcome == Outcome::Asked, "{case}");
        assert_eq!(
            choice.clears_pin(),
            matches!(outcome, Outcome::Replaced(_)),
            "{case}"
        );
    }
}

/// Moved from the chat handler (`allowed_model`): a retired id runs, and is
/// pinned, as its successor; a guest runs only what the room shares.
#[test]
fn a_retired_model_runs_and_pins_as_its_successor() {
    let m = models();
    let shared = vec!["gpt-6-sol".to_string()];
    let live_sol = Ok("gpt-6.2-sol".to_string());
    assert_eq!(live(&m, Asker::Owner, "gpt-6-sol"), live_sol);
    assert_eq!(live(&m, Asker::Owner, "gpt-6.1-sol"), live_sol);
    assert_eq!(live(&m, Asker::Guest(&shared), "gpt-6.2-sol"), live_sol);
    assert_eq!(
        live(&m, Asker::Guest(&[]), "gpt-6.2-sol"),
        Err(Why::NotShared)
    );
    assert_eq!(live(&m, Asker::Owner, "no-such-model"), Err(Why::NotHere));
}
