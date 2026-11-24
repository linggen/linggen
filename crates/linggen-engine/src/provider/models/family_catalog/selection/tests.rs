use super::*;
use crate::provider::models::curated::{self, Bundle};

const ANTHROPIC: &str = "https://api.anthropic.com";
const TODAY: &str = "2026-11-23";

fn row(url: &str) -> ModelConfig {
    serde_json::from_value(serde_json::json!({"id":"m", "provider":"anthropic", "url":url, "model":""})).unwrap()
}

fn today() -> NaiveDate { TODAY.parse().unwrap() }

/// The shipped list, with the anthropic entry of `family` approving `model`.
fn approving(family: &str, model: &str) -> Bundle {
    Bundle::variant(|v| {
        let providers = v["international"].as_array_mut().unwrap();
        let anthropic = providers.iter_mut().find(|p| p["key"] == "anthropic").unwrap();
        let entry = anthropic["entries"].as_array_mut().unwrap().iter_mut().find(|e| e["family"]["name"] == family).unwrap();
        entry["model"] = model.into();
    })
}

/// Ask about `family`, the provider listing exactly `usable`.
fn ask<'a>(list: &'a Bundle, family: &str, current: Option<&'a str>, usable: &[&str], revision: Option<u64>) -> FamilyCurrent<'a> {
    let endpoint = row(ANTHROPIC);
    let usable: BTreeSet<String> = usable.iter().map(|s| s.to_string()).collect();
    let evidence = revision.map(|credential_revision| AccessEvidence {
        connection: Connection { endpoint: &endpoint, credential_revision }, usable: &usable,
    });
    family_current(list, Connection { endpoint: &endpoint, credential_revision: 1 }, family, current, evidence.as_ref().map_or(Access::Unchecked, Access::Listed), today())
}

fn every_id(list: &Bundle) -> Vec<&str> {
    list.facts_at("anthropic").map(ModelFacts::id).collect()
}

#[test]
fn the_approved_id_is_the_target_in_every_order_of_the_facts() {
    let list = curated::shipped();
    let ids = every_id(list);
    for (family, id) in [
        ("claude-sonnet", "claude-sonnet-5-5"), ("claude-opus", "claude-opus-5-5"),
        ("claude-haiku", "claude-haiku-5-5"), ("claude-fable", "claude-fable-5-2"),
    ] {
        assert!(matches!(ask(list, family, None, &ids, Some(1)), FamilyCurrent::Current { next, .. } if next.id == id), "{family}");
    }
}

#[test]
fn an_unapproved_newer_version_never_wins() {
    // The list approves an OLDER Opus; the provider lists a newer, dated, stable one too.
    let list = approving("claude-opus", "claude-opus-5");
    let ids = every_id(&list);
    assert!(ids.contains(&"claude-opus-5-5"));
    let FamilyCurrent::Current { next, .. } = ask(&list, "claude-opus", None, &ids, Some(1)) else { panic!("approved id is usable") };
    assert_eq!(next.id, "claude-opus-5");
    // A row already on the newer one is moved to the approved one: a deliberate rollback is a new list revision.
    let FamilyCurrent::Current { next, current } = ask(&list, "claude-opus", Some("claude-opus-5-5"), &ids, Some(1)) else { panic!() };
    assert_eq!((next.id, current), ("claude-opus-5", Some("claude-opus-5-5")));
}

#[test]
fn an_approved_same_day_or_undated_version_works() {
    // DeepSeek's approved Flash shares its day with two siblings: no tie.
    let list = curated::shipped();
    let deepseek = ModelConfig { url: "https://api.deepseek.com/v1".into(), provider: "deepseek".into(), ..row(ANTHROPIC) };
    let usable: BTreeSet<String> = list.facts_at("deepseek").map(|m| m.id().to_string()).collect();
    let evidence = AccessEvidence { connection: Connection { endpoint: &deepseek, credential_revision: 1 }, usable: &usable };
    let answer = family_current(list, Connection { endpoint: &deepseek, credential_revision: 1 }, "deepseek-flash", None, Access::Listed(&evidence), today());
    assert!(matches!(answer, FamilyCurrent::Current { next, .. } if next.id == "deepseek-flash"));
    // An approved id the facts do not know at all: it runs, with no facts.
    let list = approving("claude-opus", "claude-opus-6-brand-new");
    let FamilyCurrent::Current { next, .. } = ask(&list, "claude-opus", None, &["claude-opus-6-brand-new"], Some(1)) else { panic!("undated approved id works") };
    assert!(next.id == "claude-opus-6-brand-new" && next.facts.is_none());
}

#[test]
fn a_missing_or_refused_target_holds_and_never_chooses_another() {
    let list = curated::shipped();
    // The provider lists the siblings of Sonnet but not the approved one.
    let others: Vec<&str> = every_id(list).into_iter().filter(|id| *id != "claude-sonnet-5-5").collect();
    assert!(others.contains(&"claude-sonnet-5") && others.contains(&"claude-sonnet-4-6"));
    for current in [None, Some("claude-sonnet-5")] {
        let FamilyCurrent::Choose { reason, choices, .. } = ask(list, "claude-sonnet", current, &others, Some(1)) else { panic!("held") };
        assert_eq!(reason, Hold::NoAccess);
        // The approved one is first and held; every other version is only "not approved".
        assert_eq!(choices[0].facts.id(), "claude-sonnet-5-5");
        assert!(choices.iter().skip(1).all(|c| c.held == Some(Hold::NotApproved)));
    }
    // Unproven access never moves anything: asking, no key, failed list.
    assert!(matches!(ask(list, "claude-sonnet", None, &[], None), FamilyCurrent::Checking { .. }));
    let endpoint = row(ANTHROPIC);
    let held = |access| family_current(list, Connection { endpoint: &endpoint, credential_revision: 1 }, "claude-sonnet", None, access, today());
    assert!(matches!(held(Access::NoKey), FamilyCurrent::NoKey));
    assert!(matches!(held(Access::Failed), FamilyCurrent::ListFailed));
    // A family the list does not carry, and an address it does not know.
    assert!(matches!(ask(list, "claude-unknown", None, &[], Some(1)), FamilyCurrent::Unavailable { .. }));
    let elsewhere = row("https://gateway.example.com");
    assert!(matches!(family_current(list, Connection { endpoint: &elsewhere, credential_revision: 1 }, "claude-sonnet", None, Access::Unchecked, today()), FamilyCurrent::Unavailable { .. }));
}

#[test]
fn a_stale_list_shows_but_does_not_move_a_row_and_a_key_change_is_not_evidence() {
    let list = curated::shipped();
    let ids = every_id(list);
    let later = |days| today() + chrono::Days::new(days);
    let endpoint = row(ANTHROPIC);
    let evidence = AccessEvidence { connection: Connection { endpoint: &endpoint, credential_revision: 1 }, usable: &BTreeSet::from_iter(ids.iter().map(|s| s.to_string())) };
    let answer = |current, day| family_current(list, Connection { endpoint: &endpoint, credential_revision: 1 }, "claude-opus", current, Access::Listed(&evidence), day);
    assert!(matches!(answer(Some("claude-opus-5"), later(21)), FamilyCurrent::Current { .. }));
    assert!(matches!(answer(Some("claude-opus-5"), later(22)), FamilyCurrent::Choose { reason: Hold::Stale, .. }));
    // Already on the approved id: staleness holds nothing.
    assert!(matches!(answer(Some("claude-opus-5-5"), later(60)), FamilyCurrent::Current { .. }));
    // A list held for another key is not evidence for this one.
    let other_key = family_current(list, Connection { endpoint: &endpoint, credential_revision: 2 }, "claude-opus", None, Access::Listed(&evidence), today());
    assert!(matches!(other_key, FamilyCurrent::Checking { .. }));
}
