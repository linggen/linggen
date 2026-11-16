use super::*;

fn mei() -> Who {
    Who::Person(Person {
        id: "u-2".into(),
        name: Some("Mei".into()),
    })
}

fn refusal(e: MemError) -> (u16, &'static str, String) {
    (e.code.http_status(), e.code.code(), e.message)
}

/// The verbs a person may run, written out — the table must say the same.
const ONE_PERSON: [&str; 10] = [
    "search",
    "list",
    "count",
    "get",
    "add",
    "add_batch",
    "update",
    "delete",
    "forget",
    "session_start",
];

#[test]
fn the_table_keeps_exactly_these_verbs_per_person() {
    for v in verbs::VERBS {
        assert_eq!(
            v.reach == Reach::Person,
            ONE_PERSON.contains(&v.name),
            "{}",
            v.name
        );
    }
    let machine: Vec<&str> = verbs::VERBS
        .iter()
        .filter(|v| machine_only(v.name))
        .map(|v| v.name)
        .collect();
    assert_eq!(machine, ["restamp", "accounts"]);
    assert!(!machine_only("wat"));
}

#[test]
fn the_owners_args_go_as_given() {
    for v in verbs::VERBS.iter().map(|v| v.name).chain(["wat"]) {
        let body = json!({"query": "x", "account": "u-9", "all_accounts": true});
        let mut args = body.clone();
        assert!(Who::Owner.scope(v, &mut args).is_ok(), "{v}");
        assert_eq!(args, body, "{v}");
    }
}

#[test]
fn a_person_runs_only_the_verbs_kept_per_person() {
    for v in verbs::VERBS.iter().map(|v| v.name).chain(["wat"]) {
        let mut args = json!({"query": "x", "facts": [{"content": "a"}]});
        let got = mei().scope(v, &mut args);
        if ONE_PERSON.contains(&v) {
            assert!(got.is_ok(), "{v}");
            continue;
        }
        assert_eq!(
            refusal(got.unwrap_err()),
            (
                403,
                "FORBIDDEN",
                format!("{v} is not kept per person, so only the Mac's owner may run it")
            ),
            "{v}"
        );
    }
}

#[test]
fn a_body_the_stamp_cannot_mark_is_refused() {
    for (verb, mut args) in [
        ("session_start", json!([])),
        ("add", json!(["likes tea"])),
        ("add_batch", json!({"facts": [{"content": "a"}, "b"]})),
        ("add_batch", json!({"facts": "a"})),
    ] {
        let e = mei().scope(verb, &mut args).unwrap_err();
        assert_eq!(e.code.code(), "BAD_REQUEST", "{verb}");
        assert_eq!(
            e.message,
            format!("{verb} takes a JSON object (and add_batch a list of them in facts)")
        );
    }
    let mut ok = json!({"facts": [{"content": "a"}]});
    assert!(mei().scope("add_batch", &mut ok).is_ok());
    assert_eq!(ok["facts"][0]["account_id"], "u-2");
}

#[test]
fn the_stamp_marks_writes_and_scopes_reads() {
    let mut add =
        json!({"content": "likes 90s HK songs", "account_id": "u-owner", "all_accounts": true});
    mei().scope("add", &mut add).unwrap();
    assert_eq!(
        add,
        json!({"content": "likes 90s HK songs", "account_id": "u-2", "account_name": "Mei"})
    );

    let mut list = json!({"tier": "core", "account": "u-owner", "all_accounts": true});
    mei().scope("list", &mut list).unwrap();
    assert_eq!(list, json!({"tier": "core", "account": "u-2"}));

    let mut batch = json!({"facts": [{"content": "a", "account": "u-owner"}, {"content": "b"}], "account": "x"});
    mei().scope("add_batch", &mut batch).unwrap();
    for f in batch["facts"].as_array().unwrap() {
        assert_eq!(
            (f["account_id"].clone(), f["account_name"].clone()),
            (json!("u-2"), json!("Mei"))
        );
        assert!(f.get("account").is_none());
    }
    assert!(batch.get("account").is_none());

    // No person: every claim off, nothing on — the owner's own phone.
    let mut owners = json!({"query": "x", "account": "u-2", "account_id": "u-2"});
    stamp("search", &mut owners, None);
    assert_eq!(owners, json!({"query": "x"}));
}
