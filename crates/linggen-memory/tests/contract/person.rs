//! Rows kept per person inside the store: a by-id verb scoped to one
//! person answers another person's row exactly as it answers a missing one.

use crate::support::*;
use serde_json::{json, Value};

const ALEX: &str = "u-alex";
const SAM: &str = "u-sam";

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

/// The answer for `id` with every id in it replaced, so another person's
/// row and a missing row can be compared whole.
async fn answer(m: &Mem, verb: &str, body: Value, row_id: &str) -> (u16, String) {
    let (status, env) = m.call(verb, body).await;
    (status, env.to_string().replace(row_id, "<ID>"))
}

#[tokio::test]
async fn another_persons_row_answers_like_a_missing_one() {
    let m = Mem::start().await;
    let sams = m
        .add(json!({"content": "Sam collects stamps", "tier": "semantic", "account_id": SAM, "account_name": "Sam"}))
        .await;
    let owners = m
        .add(json!({"content": "the owner collects coins", "tier": "semantic"}))
        .await;
    let missing = "no-such-row";

    for (verb, extra) in [
        ("get", json!({})),
        ("update", json!({"content": "Alex collects stamps"})),
        ("delete", json!({})),
    ] {
        for theirs in [&sams, &owners] {
            let mut body = json!({"id": id(theirs), "account": ALEX});
            let mut gone = json!({"id": missing, "account": ALEX});
            for (k, v) in extra.as_object().unwrap() {
                body[k] = v.clone();
                gone[k] = v.clone();
            }
            assert_eq!(
                answer(&m, verb, body, &id(theirs)).await,
                answer(&m, verb, gone, missing).await,
                "{verb} of {}",
                theirs["content"]
            );
        }
    }
    // The answers themselves.
    assert_refused(
        &m.call("get", json!({"id": id(&sams), "account": ALEX}))
            .await,
        404,
        "NOT_FOUND",
        &format!("no fact with id {}", id(&sams)),
    );
    assert_eq!(
        m.ok("delete", json!({"id": id(&sams), "account": ALEX}))
            .await,
        json!({"id": id(&sams), "removed": false})
    );
    // The owner (no account) does not see a person's row either.
    assert_refused(
        &m.call("get", json!({"id": id(&sams)})).await,
        404,
        "NOT_FOUND",
        &format!("no fact with id {}", id(&sams)),
    );
    // ...unless asking for everyone's (maintenance).
    assert_eq!(
        m.ok("get", json!({"id": id(&sams), "all_accounts": true}))
            .await["id"],
        json!(id(&sams))
    );
    // Nothing changed.
    assert_eq!(
        m.ok("get", json!({"id": id(&sams), "account": SAM})).await,
        sams
    );
    assert_eq!(m.ok("get", json!({"id": id(&owners)})).await, owners);
}

/// `replace_ids` from a person may name only that person's rows: another
/// person's id refuses the whole call as a missing id does, before the
/// user-voice guard reads anything and before anything is written.
#[tokio::test]
async fn a_persons_replace_may_name_only_their_own_rows() {
    let m = Mem::start().await;
    let sams = m
        .add(json!({"content": "Sam likes tea", "from": "user", "tier": "semantic", "account_id": SAM}))
        .await;
    let alexs = m
        .add(json!({"content": "Alex likes tea", "tier": "semantic", "account_id": ALEX}))
        .await;
    let write = |ids: Value| json!({"content": "Alex likes coffee", "account_id": ALEX, "replace_ids": ids});
    let missing = "no-such-row";
    assert_eq!(
        answer(&m, "add", write(json!([id(&sams)])), &id(&sams)).await,
        answer(&m, "add", write(json!([missing])), missing).await
    );
    assert_refused(
        &m.call("add", write(json!([id(&alexs), id(&sams)]))).await,
        404,
        "NOT_FOUND",
        &format!("no fact with id {}", id(&sams)),
    );
    assert_eq!(
        m.ok("count", json!({"all_accounts": true})).await["count"],
        2
    );
    // Their own row: replaced, and the new row is theirs.
    let data = m.ok("add", write(json!([id(&alexs)]))).await;
    assert_eq!(data["replaced"], json!([id(&alexs)]));
    assert_eq!(data["fact"]["account_id"], ALEX);
    // The owner's write skips a missing id rather than refusing.
    let owner = m
        .ok(
            "add",
            json!({"content": "the owner likes cocoa", "replace_ids": [missing]}),
        )
        .await;
    assert_eq!(owner["replaced_failed"], json!([missing]));
}

#[tokio::test]
async fn add_batch_stamps_each_row_with_its_person() {
    let m = Mem::start().await;
    let data = m
        .ok(
            "add_batch",
            json!({"facts": [
                {"content": "Alex likes jazz", "tier": "semantic", "account_id": ALEX, "account_name": "Alex"},
                {"content": "Alex went to a gig", "account_id": ALEX, "account_name": " "},
                {"content": "the owner likes folk", "account_id": ""},
            ]}),
        )
        .await;
    // The semantic group first, then the staged rows in the order given.
    assert_eq!(
        scrub(&data["facts"]),
        json!([
            row(
                "Alex likes jazz",
                "fact",
                "semantic",
                "derived",
                json!({"account_id": ALEX, "account_name": "Alex"})
            ),
            row(
                "Alex went to a gig",
                "fact",
                "episodic",
                "derived",
                json!({"account_id": ALEX})
            ),
            row(
                "the owner likes folk",
                "fact",
                "episodic",
                "derived",
                json!({})
            ),
        ])
    );
    // Reads are scoped the same way.
    assert_eq!(m.ok("count", json!({"account": ALEX})).await["count"], 2);
    assert_eq!(m.ok("count", json!({})).await["count"], 1);
    let alexs: Vec<Value> = m
        .ok("list", json!({"account": ALEX, "sort": "oldest"}))
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["content"].clone())
        .collect();
    assert_eq!(
        alexs,
        vec![json!("Alex likes jazz"), json!("Alex went to a gig")]
    );
}

/// A person's write meets only that person's rows in dedup.
#[tokio::test]
async fn dedup_never_merges_across_people() {
    let m = Mem::start().await;
    m.add(json!({"content": "likes tea", "tier": "semantic"}))
        .await;
    let theirs = m
        .ok(
            "add",
            json!({"content": "likes tea", "tier": "semantic", "account_id": ALEX}),
        )
        .await;
    assert_eq!(theirs["action"], "added");
    let again = m
        .ok(
            "add",
            json!({"content": "likes tea", "tier": "semantic", "account_id": ALEX}),
        )
        .await;
    assert_eq!(again["action"], "merged");
    assert_eq!(again["previous_id"], theirs["fact"]["id"]);
}
