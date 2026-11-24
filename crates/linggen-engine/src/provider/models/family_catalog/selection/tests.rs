use super::*;

const FIXTURE: &str = include_str!("../family-fixture.json");

fn row(preset: &str) -> ModelConfig {
    let p = crate::config::providers::PRESETS.iter().find(|p| p.key == preset).unwrap();
    serde_json::from_value(serde_json::json!({"id":"m", "provider":p.name, "url":p.url, "model":""})).unwrap()
}

fn resolve<'a>(s: &'a Snapshot, preset: &str, family: &str, current: Option<&'a str>, revision: Option<u64>, today: &str) -> FamilyCurrent<'a> {
    let endpoint = row(preset);
    let usable = s.providers.values().flat_map(|p| p.keys().cloned()).collect();
    let evidence = revision.map(|credential_revision| AccessEvidence {
        connection: Connection { endpoint: &endpoint, credential_revision }, usable: &usable,
    });
    family_current(s, Connection { endpoint: &endpoint, credential_revision: 1 }, family, current, evidence.as_ref().map_or(Access::Unchecked, Access::Listed), date(today).unwrap())
}

const TODAY: &str = "2026-11-23";

#[test]
fn real_frontiers_are_order_independent() {
    let mut shuffled: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    for p in shuffled["providers"].as_array_mut().unwrap() {
        p["models"].as_array_mut().unwrap().reverse();
    }
    for input in [FIXTURE.as_bytes().to_vec(), serde_json::to_vec(&shuffled).unwrap()] {
        let s = Snapshot::parse(&input).unwrap();
        for (p, f, id) in [
            ("anthropic", "claude-sonnet", "claude-sonnet-5-5"),
            ("anthropic", "claude-opus", "claude-opus-5-5"),
            ("anthropic", "claude-haiku", "claude-haiku-5-5"),
            ("anthropic", "claude-fable", "claude-fable-5-2"),
            ("openai", "gpt-sol", "gpt-6.2-sol"),
            ("openai", "gpt", "gpt-5.5"),
            ("gemini", "gemini-flash", "gemini-3.8-flash"),
        ] {
            assert!(matches!(resolve(&s, p, f, None, Some(1), TODAY), FamilyCurrent::Current { next, .. } if next.id() == id), "{f}");
        }
        let FamilyCurrent::Choose { reason: Hold::AmbiguousHead, choices, .. } = resolve(&s, "deepseek", "deepseek-flash", None, Some(1), TODAY) else { panic!("real DeepSeek tie"); };
        assert!(choices.iter().any(|c| c.facts.id() == "deepseek-v4-flash-vision-exp" && c.held == Some(Hold::NonStable(Stability::Experimental))));
    }
}

#[test]
fn follow_requires_fresh_evidence_for_this_credential_and_a_newer_day() {
    let s = Snapshot::parse(FIXTURE.as_bytes()).unwrap();
    let decide = |current, revision, day| resolve(&s, "anthropic", "claude-opus", current, revision, day);
    for revision in [None, Some(2)] {
        assert!(matches!(decide(Some("claude-opus-5"), revision, TODAY), FamilyCurrent::Checking { next } if next.id() == "claude-opus-5-5"));
    }
    // 21 days after the observation still follows; 22 does not.
    assert!(matches!(decide(Some("claude-opus-5"), Some(1), "2026-12-14"), FamilyCurrent::Current { .. }));
    assert!(matches!(decide(Some("claude-opus-5"), Some(1), "2026-12-15"), FamilyCurrent::Choose { reason: Hold::Stale, .. }));
    assert!(matches!(decide(None, Some(1), "2026-12-15"), FamilyCurrent::Current { current: None, .. }));
    assert!(matches!(decide(Some("claude-opus-5"), Some(1), TODAY), FamilyCurrent::Current { next, .. } if next.id() == "claude-opus-5-5"));
    assert!(matches!(resolve(&s, "anthropic", "never-seen", None, Some(1), TODAY), FamilyCurrent::Unavailable { current: None }));
}

#[test]
fn an_unreadable_list_is_not_an_empty_list() {
    let s = Snapshot::parse(FIXTURE.as_bytes()).unwrap();
    let endpoint = row("anthropic");
    let connection = || Connection { endpoint: &endpoint, credential_revision: 1 };
    let usable = BTreeSet::new();
    let evidence = AccessEvidence { connection: connection(), usable: &usable };
    let decide = |access| family_current(&s, connection(), "claude-opus", None, access, date(TODAY).unwrap());
    assert!(matches!(decide(Access::Failed), FamilyCurrent::ListFailed));
    assert!(matches!(decide(Access::Listed(&evidence)), FamilyCurrent::Choose { reason: Hold::NoAccess, .. }));
}

#[test]
fn same_day_and_missing_date_hold_without_trying_the_runner_up() {
    for (day, expected) in [(Some("2026-09-22"), Hold::AmbiguousHead), (None, Hold::MissingDate)] {
        let mut s = Snapshot::parse(FIXTURE.as_bytes()).unwrap();
        s.providers.get_mut("anthropic").unwrap().get_mut("claude-opus-5").unwrap().release_date = day.map(str::to_string);
        assert!(matches!(resolve(&s, "anthropic", "claude-opus", None, Some(1), TODAY), FamilyCurrent::Choose { reason, .. } if reason == expected));
    }
    let s = Snapshot::parse(FIXTURE.as_bytes()).unwrap();
    assert!(matches!(resolve(&s, "anthropic", "claude-opus", Some("claude-fable-5-2"), Some(1), TODAY), FamilyCurrent::Choose { reason: Hold::NotNewer, .. }));
}
