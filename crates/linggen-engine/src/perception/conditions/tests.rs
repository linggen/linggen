use super::*;
use crate::perception::told::Told;

const NOW: i64 = 1_800_000_000;

fn finding(level: u32) -> Finding {
    Finding {
        level,
        situation: "this Mac has 4.0 GB free".into(),
        verbs: "offer to look",
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
    Some(Told {
        at: NOW - days_ago * 86_400,
        level,
        by: "phone".into(),
    })
}

#[test]
fn every_notice_carries_a_verb() {
    // Rule 3, enforced rather than remembered: a condition with nothing to
    // offer must not exist.
    let n = Notice {
        topic: "t".into(),
        level: 1,
        situation: "s".into(),
        verbs: "v",
        owner: None,
    };
    assert!(!n.verbs.is_empty());
    let mut l = Ledger::default();
    let peer_notice = choose(&mut l, vec![], peer(vec![problem(1, None)]), NOW).unwrap();
    assert!(!peer_notice.verbs.is_empty(), "the other device's too");
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
