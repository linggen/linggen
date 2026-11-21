use super::*;

fn tools() -> Vec<(String, ActionClass)> {
    vec![("ScanDisk".into(), ActionClass::Read), ("Clear".into(), ActionClass::Edit)]
}

#[test]
fn a_remedy_is_offers_or_the_reason_there_are_none_never_neither() {
    let none = Remedy::of(vec![], "it is on the other device");
    assert!(none.offers().is_empty());
    assert_eq!(none.none(), Some("it is on the other device"));
    assert!(none.facts().contains("nothing can be offered from here: it is on the other device"));

    let open = Offer::Open { skill: "shifu".into(), view: Some("files".into()) };
    let some = Remedy::of(vec![open], "unused");
    assert_eq!(some.none(), None, "a reason is only for a remedy with no offers");
    assert!(some.facts().contains("1. open the shifu page at files"), "{}", some.facts());
}

#[test]
fn a_skill_declares_offers_by_problem_and_a_bad_one_is_left_out() {
    let raw = serde_json::json!({
        "storage-filling": [
            {"open": "files"},
            {"tool": "ScanDisk"},
            {"tool": "NoSuchTool"},
            {"tool": "ScanDisk", "open": "files"},
            {"tool": "Clear", "args": [1]}
        ],
        "bad": "not a list"
    });
    let got = usable(Some(raw), "shifu", &tools(), true);
    let offers: Vec<&Offer> = got.iter().map(|d| &d.offer).collect();
    assert_eq!(
        offers,
        vec![
            &Offer::Open { skill: "shifu".into(), view: Some("files".into()) },
            &Offer::Tool {
                skill: "shifu".into(),
                tool: "ScanDisk".into(),
                args: serde_json::Value::Null,
                tier: ActionClass::Read
            },
        ],
        "at most three are read, and of those only the good ones stay"
    );
    assert!(got.iter().all(|d| d.problem == "storage-filling"));
}

#[test]
fn a_skill_without_a_page_cannot_offer_to_open_it() {
    let raw = serde_json::json!({"storage-filling": [{"open": ""}]});
    assert!(usable(Some(raw.clone()), "s", &tools(), false).is_empty());
    let got = usable(Some(raw), "s", &tools(), true);
    assert_eq!(got[0].offer, Offer::Open { skill: "s".into(), view: None });
}

#[test]
fn declared_offers_are_read_by_problem() {
    declare_for_this_thread(vec![
        Declared {
            problem: "storage-filling".into(),
            offer: Offer::Open { skill: "a".into(), view: None },
        },
        Declared {
            problem: "other".into(),
            offer: Offer::Open { skill: "b".into(), view: None },
        },
    ]);
    assert_eq!(declared_for("storage-filling").len(), 1);
    assert!(declared_for("nothing").is_empty());
}
