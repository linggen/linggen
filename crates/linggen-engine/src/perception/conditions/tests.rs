use super::*;
use crate::perception::told::Told;

const NOW: i64 = 1_800_000_000;

fn finding(level: u32) -> Finding {
    Finding {
        level,
        situation: "this Mac has 4.0 GB free".into(),
        remedy: Remedy::of(Vec::new(), NOTHING_INSTALLED_OFFERS),
    }
}

fn problem(level: u32, told: Option<Told>) -> PeerProblem {
    PeerProblem {
        key: "storage".into(),
        level,
        line: "storage: 8.3 GB free of 110.0 GB · 8% free".into(),
        told,
    }
}

fn peer(ps: Vec<PeerProblem>) -> Option<(String, Option<Vec<PeerProblem>>)> {
    Some(("this phone".into(), Some(ps)))
}

fn told_by_phone(days_ago: i64, level: u32) -> Option<Told> {
    Some(Told::new(NOW - days_ago * 86_400, level, "phone"))
}

#[test]
fn every_notice_carries_offers_or_why_there_are_none() {
    // Rule 3, enforced by the type: a Remedy cannot be built empty, so a
    // condition with neither offers nor a reason cannot register.
    let mut l = Ledger::default();
    let peer_notice = choose(&mut l, vec![], peer(vec![problem(1, None)]), NOW).unwrap();
    assert_eq!(
        peer_notice.remedy.none(),
        Some(ON_THE_OTHER_DEVICE),
        "the other device's problem: none here, and why"
    );
}

#[test]
fn the_macs_disk_carries_what_the_skills_declare_for_it() {
    use crate::perception::offer::{declare_for_this_thread, Declared, Offer};
    let open = Offer::Open { skill: "shifu".into(), view: Some("files".into()) };
    declare_for_this_thread(vec![Declared { problem: STORAGE_FILLING.into(), offer: open.clone() }]);
    let r = Remedy::of(crate::perception::offer::declared_for(STORAGE_FILLING), NOTHING_INSTALLED_OFFERS);
    assert_eq!(r.offers(), &[open]);
    declare_for_this_thread(vec![]);
    let r = Remedy::of(crate::perception::offer::declared_for(STORAGE_FILLING), NOTHING_INSTALLED_OFFERS);
    assert_eq!(r.none(), Some(NOTHING_INSTALLED_OFFERS));
}

#[test]
fn no_notice_tells_her_which_tools_to_use() {
    // The prose that named tools and verbs is gone: what she is handed is
    // facts and the remedy's facts.
    let src = include_str!("../conditions.rs");
    for gone in ["verbs", "never start", "you can offer"] {
        assert!(!src.contains(gone), "{gone}");
    }
}

#[test]
fn a_fired_topic_holds_its_rearm() {
    let mut l = Ledger::default();
    let n = choose(&mut l, vec![("disk", Some(finding(1)))], None, NOW).unwrap();
    record(&mut l, &n, NOW);
    assert!(
        choose(
            &mut l,
            vec![("disk", Some(finding(1)))],
            None,
            NOW + 30 * 86_400
        )
        .is_none(),
        "a topic just said must not be said again — a month later included"
    );
    let worse = choose(&mut l, vec![("disk", Some(finding(2)))], None, NOW + 86_400);
    assert!(worse
        .unwrap()
        .situation
        .contains("worse than when told 1d ago"));
}

#[test]
fn a_glance_does_not_raise_a_peer_problem_already_told_and_no_worse() {
    let mut l = Ledger::default();
    let n = choose(
        &mut l,
        vec![],
        peer(vec![problem(1, told_by_phone(2, 1))]),
        NOW,
    );
    assert!(n.is_none(), "told two days ago and no worse is not news");
}

#[test]
fn a_glance_raises_the_same_peer_problem_a_level_worse_as_worse() {
    let mut l = Ledger::default();
    let n = choose(
        &mut l,
        vec![],
        peer(vec![problem(2, told_by_phone(2, 1))]),
        NOW,
    )
    .expect("worse is news");
    assert!(
        n.situation.contains("worse than when told 2d ago"),
        "{}",
        n.situation
    );
    assert!(n.situation.contains("this phone"), "{}", n.situation);
    assert!(n.situation.contains("8% free"), "{}", n.situation);
}

#[test]
fn a_glance_raises_a_peer_problem_never_told() {
    let mut l = Ledger::default();
    let n = choose(&mut l, vec![], peer(vec![problem(1, None)]), NOW).expect("never dropped");
    assert!(n.situation.contains("not told yet"), "{}", n.situation);
    // Said once: after she says it, the next glance does not raise it, though
    // the phone has not heard yet.
    record(&mut l, &n, NOW);
    assert!(choose(&mut l, vec![], peer(vec![problem(1, None)]), NOW + 3600).is_none());
}

#[test]
fn a_peer_problem_that_cleared_and_returned_is_raised_again() {
    let mut l = Ledger::default();
    let n = choose(&mut l, vec![], peer(vec![problem(1, None)]), NOW).unwrap();
    record(&mut l, &n, NOW);
    // The phone recovered: it publishes no such problem, and its own record
    // of telling it is gone with it.
    assert!(choose(&mut l, vec![], peer(vec![]), NOW + 86_400).is_none());
    // And it came back.
    let back = choose(
        &mut l,
        vec![],
        peer(vec![problem(1, None)]),
        NOW + 3 * 86_400,
    );
    assert!(back.is_some(), "a return is news");
}

#[test]
fn a_phone_that_says_nothing_about_problems_settles_nothing() {
    // A build from before problems crossed: unknown, not cleared.
    let mut l = Ledger::default();
    let n = choose(&mut l, vec![], peer(vec![problem(1, None)]), NOW).unwrap();
    record(&mut l, &n, NOW);
    let _ = choose(&mut l, vec![], Some(("this phone".into(), None)), NOW + 60);
    assert!(choose(&mut l, vec![], peer(vec![problem(1, None)]), NOW + 120).is_none());
}

#[test]
fn this_macs_own_come_first_one_per_glance() {
    let mut l = Ledger::default();
    let n = choose(
        &mut l,
        vec![("disk", Some(finding(1)))],
        peer(vec![problem(1, None)]),
        NOW,
    )
    .unwrap();
    assert_eq!(n.topic, "disk");
}

#[test]
fn runway_halving_is_worse() {
    assert_eq!(runway_level(13), 0);
    assert_eq!(runway_level(2), 1);
    assert_eq!(runway_level(1), 2);
    assert_eq!(runway_level(0), 3);
}

#[test]
fn a_phone_not_now_that_ran_out_is_said_once_on_the_mac_not_at_every_glance() {
    let mut told = Told::new(NOW - 10 * 86_400, 1, "phone");
    told.answer = Some(crate::perception::told::Answer::NotNow);
    told.answered_at = Some(NOW - 8 * 86_400);
    let mut l = Ledger::default();
    let n = choose(&mut l, vec![], peer(vec![problem(1, Some(told.clone()))]), NOW)
        .expect("a not-now that ran out is due again");
    record(&mut l, &n, NOW);
    // The phone has not filed the report yet: it still publishes its old record.
    assert!(
        choose(&mut l, vec![], peer(vec![problem(1, Some(told))]), NOW + 3600).is_none(),
        "said once — the Mac's newer report holds until the phone files it"
    );
    // Filed: the phone's record is the report's telling; the report goes, and
    // it is still not due.
    let filed = Told::new(NOW, 1, "mac");
    assert!(choose(&mut l, vec![], peer(vec![problem(1, Some(filed))]), NOW + 7200).is_none());
    assert!(l.reports("phone").is_empty(), "filed, so dropped");
}
