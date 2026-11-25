//! `Memory::call` for a [`Who`]: the owner's args as given; a person's call
//! refused unless the verb is kept per person, and otherwise stamped — every
//! place inside the verbs that decides whose rows (by-id lookup, filters,
//! dedup, `replace_ids`, batches) then keeps to that person, whatever the
//! args claimed.

use crate::support::*;
use ling_mem::{MemError, Person, Reply, Who};
use serde_json::{json, Value};

fn guest() -> Who {
    Who::Person(Person {
        id: "u-guest".into(),
        name: Some("Guest".into()),
    })
}

async fn data(m: &Mem, who: &Who, verb: &str, args: Value) -> Value {
    match m.state.call(who, verb, args).await {
        Ok(Reply::Done(v)) => v,
        other => panic!("{verb}: {other:?}"),
    }
}

async fn refused(m: &Mem, who: &Who, verb: &str, args: Value) -> (&'static str, String) {
    let e: MemError = m.state.call(who, verb, args).await.unwrap_err();
    (e.code.code(), e.message)
}

fn id(row: &Value) -> String {
    row["fact"]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_person_writes_their_own_rows_whatever_the_args_claim() {
    let m = Mem::start().await;
    let claim = json!({"content": "likes tea", "tier": "semantic", "account_id": "u-other", "account_name": "Other", "account": "u-other", "all_accounts": true});
    let row = data(&m, &guest(), "add", claim).await;
    assert_eq!(
        (
            row["fact"]["account_id"].clone(),
            row["fact"]["account_name"].clone()
        ),
        (json!("u-guest"), json!("Guest"))
    );
    let batch = json!({"facts": [{"content": "likes jazz", "account_id": "u-other"}, {"content": "went to a gig", "all_accounts": true}]});
    let rows = data(&m, &guest(), "add_batch", batch).await;
    for f in rows["facts"].as_array().unwrap() {
        assert_eq!(f["account_id"], "u-guest", "{f}");
    }
    // The owner may name an account: that is how the owner's tools work.
    let named = data(
        &m,
        &Who::Owner,
        "add",
        json!({"content": "Alex likes chess", "tier": "semantic", "account_id": "u-alex"}),
    )
    .await;
    assert_eq!(named["fact"]["account_id"], "u-alex");

    // `all_accounts` from a person is overridden by the stamp.
    let mine = data(&m, &guest(), "count", json!({"all_accounts": true})).await;
    assert_eq!(mine["count"], 3);
    let all = data(&m, &Who::Owner, "count", json!({"all_accounts": true})).await;
    assert_eq!(all["count"], 4);
}

#[tokio::test]
async fn another_persons_row_answers_like_a_missing_one() {
    let m = Mem::start().await;
    let owners = data(
        &m,
        &Who::Owner,
        "add",
        json!({"content": "the owner likes folk", "tier": "semantic"}),
    )
    .await;
    let theirs = id(&owners);
    for (verb, extra) in [
        ("get", json!({})),
        ("update", json!({"content": "likes rock"})),
    ] {
        let mut body = json!({"id": theirs});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (code, message) = refused(&m, &guest(), verb, body).await;
        assert_eq!(
            (code, message),
            ("NOT_FOUND", format!("no fact with id {theirs}")),
            "{verb}"
        );
    }
    assert_eq!(
        data(&m, &guest(), "delete", json!({"id": theirs})).await,
        json!({"id": theirs, "removed": false})
    );
    // `replace_ids` naming the owner's row refuses as a missing id does.
    let (code, message) = refused(
        &m,
        &guest(),
        "add",
        json!({"content": "likes rock", "replace_ids": [theirs]}),
    )
    .await;
    assert_eq!(
        (code, message),
        ("NOT_FOUND", format!("no fact with id {theirs}"))
    );
    let still = data(&m, &Who::Owner, "get", json!({"id": theirs})).await;
    assert_eq!(still["content"], "the owner likes folk");
}

#[tokio::test]
async fn a_persons_dedup_search_and_forget_stay_inside_the_person() {
    let m = Mem::start().await;
    let owner = &Who::Owner;
    data(
        &m,
        owner,
        "add",
        json!({"content": "tried oat milk", "type": "learned", "tier": "semantic"}),
    )
    .await;
    let first = data(
        &m,
        &guest(),
        "add",
        json!({"content": "tried oat milk", "type": "learned", "tier": "semantic"}),
    )
    .await;
    assert_eq!(first["action"], "added", "no merge into the owner's row");
    let hits = data(&m, &guest(), "search", json!({"query": "tried oat milk"})).await;
    let owners: Vec<&Value> = hits
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["account_id"] != "u-guest")
        .collect();
    assert!(owners.is_empty(), "{hits}");
    assert_eq!(
        data(&m, &guest(), "forget", json!({"type": "learned"})).await,
        json!({"removed": 1})
    );
    assert_eq!(
        data(&m, owner, "count", json!({"type": "learned"})).await["count"],
        1
    );
}

/// A person's refusals, whole: the verbs not kept per person (those that
/// cross account lines included), and a body the stamp cannot mark.
#[tokio::test]
async fn a_persons_refusals_are_exact() {
    let m = Mem::start().await;
    for verb in ["restamp", "accounts", "stats", "days", "sweep", "wat"] {
        assert_eq!(
            refused(&m, &guest(), verb, json!({})).await,
            (
                "FORBIDDEN",
                format!("{verb} is not kept per person, so only the Mac's owner may run it")
            ),
            "{verb}"
        );
    }
    assert_eq!(
        refused(&m, &guest(), "add_batch", json!({"facts": ["likes tea"]})).await,
        (
            "BAD_REQUEST",
            "add_batch takes a JSON object (and add_batch a list of them in facts)".into()
        )
    );
    // Through the route form: the same refusal in memory's envelope.
    let res = ling_mem::http::verb::answer(m.state.clone(), guest(), "accounts", json!({})).await;
    assert_eq!(res.status().as_u16(), 403);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"ok": false, "code": "FORBIDDEN", "error": "accounts is not kept per person, so only the Mac's owner may run it"})
    );
    // Nothing was written by any of them.
    assert_eq!(
        data(&m, &Who::Owner, "count", json!({"all_accounts": true})).await["count"],
        0
    );
}

/// The user-voice guard: another person's `from=user` row is not found
/// before the guard reads it; a person's own gets the guard's refusal,
/// exactly as the owner's own does.
#[tokio::test]
async fn the_user_voice_guard_meets_only_the_persons_own_rows() {
    let m = Mem::start().await;
    let owners = data(
        &m,
        &Who::Owner,
        "add",
        json!({"content": "the owner prefers tea", "type": "preference", "from": "user", "tier": "semantic"}),
    )
    .await;
    let guests = data(
        &m,
        &guest(),
        "add",
        json!({"content": "the guest prefers cocoa", "type": "preference", "from": "user", "tier": "semantic"}),
    )
    .await;
    let rewrite = |row: &Value| json!({"id": id(row), "content": "prefers water"});
    let (code, message) = refused(&m, &guest(), "update", rewrite(&owners)).await;
    assert_eq!(
        (code, message),
        ("NOT_FOUND", format!("no fact with id {}", id(&owners)))
    );
    let theirs = refused(&m, &guest(), "update", rewrite(&guests)).await;
    let own = refused(&m, &Who::Owner, "update", rewrite(&owners)).await;
    let row = |words: &str, row: &Value| words.replace(&id(row), "<ID>");
    assert_eq!(
        (
            theirs.0,
            row(&theirs.1, &guests).replace("the guest prefers cocoa", "<ROW>")
        ),
        (
            own.0,
            row(&own.1, &owners).replace("the owner prefers tea", "<ROW>")
        )
    );
}

/// `list` and `session_start` show a person their own rows only.
#[tokio::test]
async fn a_persons_list_and_session_start_are_their_own() {
    let m = Mem::start().await;
    data(
        &m,
        &Who::Owner,
        "add",
        json!({"content": "Alex lives in Lisbon", "tier": "core"}),
    )
    .await;
    data(
        &m,
        &guest(),
        "add",
        json!({"content": "the guest lives in Porto", "tier": "core"}),
    )
    .await;
    let contents = |rows: &Value| -> Vec<Value> {
        rows.as_array()
            .unwrap()
            .iter()
            .map(|r| r["content"].clone())
            .collect()
    };
    let listed = data(&m, &guest(), "list", json!({"all_accounts": true})).await;
    assert_eq!(contents(&listed), [json!("the guest lives in Porto")]);
    let start = data(&m, &guest(), "session_start", json!({})).await;
    assert_eq!(
        contents(&start["core"]),
        [json!("the guest lives in Porto")]
    );
}

/// Dedup across the two tables keeps to the person: the guest's promote
/// of a staged row never touches the owner's staged copy of the same words.
#[tokio::test]
async fn cross_table_dedup_keeps_to_the_person() {
    let m = Mem::start().await;
    let words = "walked to the harbour at dawn";
    data(
        &m,
        &Who::Owner,
        "add",
        json!({"content": words, "episodic": true}),
    )
    .await;
    data(
        &m,
        &guest(),
        "add",
        json!({"content": words, "episodic": true}),
    )
    .await;
    data(
        &m,
        &guest(),
        "add",
        json!({"content": words, "tier": "semantic"}),
    )
    .await;
    let owners = data(&m, &Who::Owner, "count", json!({"episodic": true})).await;
    assert_eq!(owners["count"], 1, "the owner's staged row is still there");
    // `count` without a table spans both: the staged row is all the owner has.
    let all = data(&m, &Who::Owner, "count", json!({})).await;
    assert_eq!(all["count"], 1, "and no promoted copy of it");
    let theirs = data(&m, &guest(), "list", json!({"tier": "semantic"})).await;
    assert_eq!(theirs.as_array().unwrap().len(), 1, "{theirs}");
}
