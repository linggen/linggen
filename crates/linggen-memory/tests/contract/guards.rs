//! The refusals and the merge rules a caller (often a model) reads.

use crate::support::*;
use serde_json::{json, Value};

/// The user-voice guard's refusal, word for word: a model reads it and
/// acts on its recovery steps.
fn voice_refusal(rows: &str) -> String {
    format!(
        "BLOCKED by the user-voice merge guard: this write replaces/rewrites rows in the USER'S VOICE (from=user): {rows}. The user's voice changes only with the user (merge law). Recovery — pick ONE: (1) if the user's CURRENT message states this change as SETTLED — a command (\"update X to Y\", \"forget X\"), a declaration (\"my X is now Y\"), or a commitment (\"from now on, X\") — retry the exact same call with \"user_directed\": true; a hedged reflection (\"X feels about right to me\") does NOT qualify; (2) otherwise ask the user to choose (the host's ask-user primitive, or a plain question in chat), then retry with \"user_directed\": true after their answer. Do NOT work around it by writing a new row without replace_ids — that creates the drift this guard exists to prevent."
    )
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn blank_content_is_refused() {
    let m = Mem::start().await;
    for content in ["", "  \n\t"] {
        assert_refused(
            &m.call("add", json!({"content": content})).await,
            400,
            "BAD_REQUEST",
            "content must not be empty",
        );
    }
    assert_eq!(m.ok("count", json!({})).await["count"], 0);
}

#[tokio::test]
async fn replacing_or_rewriting_the_users_voice_needs_user_directed() {
    let m = Mem::start().await;
    let long = "Alex prefers tea over coffee, always loose leaf, never bagged, and never after six";
    let voice = m
        .add(json!({"content": long, "from": "user", "tier": "semantic"}))
        .await;
    let gist: String = long.chars().take(60).collect();
    let listing = format!("{} \"{gist}\"", id(&voice));

    let replace = json!({"content": "Alex now prefers coffee", "replace_ids": [id(&voice)]});
    assert_refused(
        &m.call("add", replace.clone()).await,
        400,
        "BAD_REQUEST",
        &voice_refusal(&listing),
    );
    let rewrite = json!({"id": id(&voice), "content": "Alex prefers coffee"});
    assert_refused(
        &m.call("update", rewrite).await,
        400,
        "BAD_REQUEST",
        &voice_refusal(&listing),
    );
    // Nothing was written; metadata-only patches pass unguarded.
    assert_eq!(m.ok("count", json!({})).await["count"], 1);
    assert_eq!(
        m.ok("update", json!({"id": id(&voice), "summary": "tea"}))
            .await["summary"],
        "tea"
    );

    let mut directed = replace;
    directed["user_directed"] = json!(true);
    let data = m.ok("add", directed).await;
    assert_eq!(data["action"], "added");
    assert_eq!(data["replaced"], json!([id(&voice)]));
    // A replacement keeps its loser's tier.
    assert_eq!(data["fact"]["tier"], "semantic");
}

/// A semantic loser is expired (kept, findable by `superseded_by`); an
/// episodic loser is deleted; an id that is no row is `replaced_failed`.
#[tokio::test]
async fn replace_ids_expires_semantic_rows_and_deletes_staged_ones() {
    let m = Mem::start().await;
    let kept = m
        .add(json!({"content": "Alex uses vim", "tier": "semantic"}))
        .await;
    let staged = m.add(json!({"content": "Alex tried helix once"})).await;
    let data = m
        .ok(
            "add",
            json!({"content": "Alex edits code in helix", "replace_ids": [id(&kept), id(&staged), "nope"]}),
        )
        .await;
    let winner = id(&data["fact"]);
    assert_eq!(data["replaced"], json!([id(&kept), id(&staged)]));
    assert_eq!(data["replaced_failed"], json!(["nope"]));
    assert_eq!(data["fact"]["tier"], "semantic", "the losers' highest tier");

    // Expired: out of default reads, still there for the unpack query.
    let live: Vec<String> = m
        .ok("list", json!({}))
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(id)
        .collect();
    assert_eq!(live, vec![winner.clone()]);
    let unpacked = m.ok("list", json!({"superseded_by": winner})).await;
    assert_eq!(unpacked.as_array().unwrap().len(), 1);
    assert_eq!(
        scrub(&unpacked[0]),
        row(
            "Alex uses vim",
            "fact",
            "semantic",
            "derived",
            json!({
                "expired_at": "<time>",
                "superseded_by": winner,
            })
        )
    );
    // Today `get` still answers an expired row by id.
    assert_eq!(
        m.ok("get", json!({"id": id(&kept)})).await["superseded_by"],
        json!(winner)
    );
    assert_refused(
        &m.call("get", json!({"id": id(&staged)})).await,
        404,
        "NOT_FOUND",
        &format!("no fact with id {}", id(&staged)),
    );
}

/// Exact content in the other table: a lower-tier copy is promoted away,
/// a same-or-higher-tier copy absorbs the write (`merge_with_existing`).
#[tokio::test]
async fn cross_tier_dedup_promotes_or_merges() {
    let m = Mem::start().await;
    let staged = m.add(json!({"content": "Alex bakes sourdough"})).await;
    let promoted = m
        .ok(
            "add",
            json!({"content": "Alex bakes sourdough", "tier": "semantic", "host": "codex"}),
        )
        .await;
    assert_eq!(promoted["action"], "added");
    assert_ne!(id(&promoted["fact"]), id(&staged));
    assert_eq!(
        m.ok("count", json!({"episodic": true})).await["count"],
        0,
        "the staged copy is gone"
    );

    let again = m
        .ok(
            "add",
            json!({"content": "Alex bakes sourdough", "outcome": "positive", "summary": "bakes", "source_session": "s-9"}),
        )
        .await;
    assert_eq!(
        scrub(&again),
        json!({
            "action": "merged",
            "similarity": 1.0,
            "previous_id": id(&promoted["fact"]),
            "fact": row("Alex bakes sourdough", "fact", "semantic", "derived", json!({
                "outcome": "positive",
                "summary": "bakes",
                "source_session": "s-9",
                "host": "codex",
            })),
        })
    );
    assert_eq!(id(&again["fact"]), id(&promoted["fact"]));
    assert_eq!(m.ok("count", json!({})).await["count"], 1);
    // In one table, the same text merges too.
    let same = m
        .ok(
            "add",
            json!({"content": "Alex bakes sourdough", "tier": "semantic"}),
        )
        .await;
    assert_eq!(same["action"], "merged");
    assert_eq!(same["previous_id"], json!(id(&promoted["fact"])));
    // skip_dedup (alias force) writes a second row.
    let forced = m
        .ok(
            "add",
            json!({"content": "Alex bakes sourdough", "tier": "semantic", "force": true}),
        )
        .await;
    assert_eq!(forced["action"], "added");
    assert_eq!(m.ok("count", json!({})).await["count"], 2);
}

#[tokio::test]
async fn a_core_row_has_no_scope_and_no_index() {
    let m = Mem::start().await;
    let core = m
        .add(json!({"content": "Alex lives in Lisbon", "tier": "core", "cwd": "/opt/alex/notes", "scope": "/opt/alex", "indexed": true, "summary": "Lisbon"}))
        .await;
    assert_eq!(
        scrub(&core),
        row(
            "Alex lives in Lisbon",
            "fact",
            "core",
            "derived",
            json!({"summary": "Lisbon"})
        )
    );
    let scoped = m
        .add(json!({"content": "Alex uses tabs in Go", "cwd": "/opt/alex/go", "indexed": true}))
        .await;
    assert_eq!(scoped["scope"], "/opt/alex/go");
    assert_eq!(scoped["indexed"], true);
    assert_eq!(scoped["tier"], "semantic", "an indexed row is long-term");
    let raised = m
        .ok(
            "update",
            json!({"id": id(&scoped), "tier": "core", "scope": "/opt/alex/go", "indexed": true}),
        )
        .await;
    assert!(raised.get("scope").is_none(), "{raised}");
    assert!(raised.get("indexed").is_none(), "{raised}");
}

#[tokio::test]
async fn forget_refuses_an_empty_filter() {
    let m = Mem::start().await;
    m.add(json!({"content": "Alex keeps bees", "tier": "semantic"}))
        .await;
    let why = "forget refuses an empty filter — supply at least one of apps, type, tier, from, outcome, since, until, source_session";
    for body in [
        json!({}),
        json!({"scope_root": "/opt/alex"}),
        json!({"type": ""}),
        json!({"indexed": true}),
    ] {
        assert_refused(&m.call("forget", body).await, 400, "BAD_REQUEST", why);
    }
    assert_eq!(m.ok("count", json!({})).await["count"], 1);
}

/// An unknown verb is axum's bare 404: no envelope, no body.
#[tokio::test]
async fn an_unknown_verb_is_a_bare_404() {
    let m = Mem::start().await;
    assert_eq!(m.call("wat", json!({})).await, (404, json!("")));
    assert_eq!(m.get("/api/memory/list").await, (405, json!("")));
}

/// A body the handler cannot read is refused by axum before it runs, in
/// plain text, not the envelope.
#[tokio::test]
async fn a_malformed_body_is_axums_plain_text_refusal() {
    let m = Mem::start().await;
    assert_eq!(
        m.call("add", json!({"contents": "x"})).await,
        (
            422,
            json!("Failed to deserialize the JSON body into the target type: missing field `content` at line 1 column 16")
        )
    );
    assert_eq!(
        m.call("add", json!({"content": "x", "indexed": "yes"})).await,
        (
            422,
            json!("Failed to deserialize the JSON body into the target type: indexed: invalid type: string \"yes\", expected a boolean at line 1 column 30")
        )
    );
    assert_eq!(
        m.post_raw("/api/memory/add", "{not json", "application/json")
            .await,
        (
            400,
            json!(
                "Failed to parse the request body as JSON: key must be a string at line 1 column 2"
            )
        )
    );
    assert_eq!(
        m.post_raw("/api/memory/add", "{\"content\":\"x\"}", "text/plain")
            .await,
        (
            415,
            json!("Expected request with `Content-Type: application/json`")
        )
    );
    assert_eq!(m.ok("count", json!({})).await["count"], 0);
}

/// `past_ttl`: episodic rows older than the configured TTL, and only the
/// episodic table unless the caller names one.
#[tokio::test]
async fn list_past_ttl_is_staged_rows_older_than_the_ttl() {
    let m = Mem::start().await;
    let old = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
    let stale = m
        .add(json!({"content": "Alex fed the cat", "occurred_at": old}))
        .await;
    m.add(json!({"content": "Alex fed the dog"})).await;
    m.add(json!({"content": "Alex adopted a cat", "tier": "semantic", "occurred_at": old}))
        .await;
    let past = m.ok("list", json!({"past_ttl": true})).await;
    assert_eq!(
        past.as_array().unwrap().iter().map(id).collect::<Vec<_>>(),
        vec![id(&stale)]
    );
    assert_eq!(
        m.ok("list", json!({"past_ttl": true, "episodic": false}))
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // A longer TTL: nothing is past it.
    m.put(
        "/api/config",
        json!({"episodic_ttl_days": 60, "recall_min_score": 0.7}),
    )
    .await;
    assert_eq!(m.ok("list", json!({"past_ttl": true})).await, json!([]));
}

/// `unjudged` needs a day, and lists only that day's rows created after
/// the day was remembered.
#[tokio::test]
async fn list_unjudged_is_a_days_rows_after_its_remember_pass() {
    let m = Mem::start().await;
    assert_refused(
        &m.call("list", json!({"unjudged": true})).await,
        400,
        "BAD_REQUEST",
        "unjudged needs a day",
    );
    let day = days_ago(2);
    let at = local_noon(&day);
    let judged = m
        .add(json!({"content": "Alex hiked a ridge", "occurred_at": at}))
        .await;
    m.ok("remember_day", json!({"date": day})).await;
    let late = m
        .add(json!({"content": "Alex saw an eagle", "occurred_at": at}))
        .await;
    let list = m.ok("list", json!({"day": day, "unjudged": true})).await;
    assert_eq!(
        list.as_array().unwrap().iter().map(id).collect::<Vec<_>>(),
        vec![id(&late)]
    );
    let all = m.ok("list", json!({"day": day, "sort": "oldest"})).await;
    assert_eq!(
        all.as_array().unwrap().iter().map(id).collect::<Vec<_>>(),
        vec![id(&judged), id(&late)]
    );
    // A day never remembered: all of its rows are unjudged.
    let other = days_ago(3);
    m.add(json!({"content": "Alex swam", "occurred_at": local_noon(&other)}))
        .await;
    assert_eq!(
        m.ok("list", json!({"day": other, "unjudged": true}))
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// A day is one key however it is spelled: `2026-1-5` names the same
/// remembered day and the same lease as `2026-01-05`.
#[tokio::test]
async fn an_unpadded_day_names_the_same_day() {
    let m = Mem::start().await;
    let (padded, unpadded) = ("2026-01-05", "2026-1-5");
    m.add(json!({"content": "Alex hiked a ridge", "occurred_at": local_noon(padded)}))
        .await;
    m.ok("remember_day", json!({"date": padded})).await;
    assert_eq!(
        m.ok("list", json!({"day": unpadded, "unjudged": true})).await,
        json!([])
    );
    let lease = m
        .ok("dream_lease", json!({"day": unpadded, "holder": "mac-a"}))
        .await;
    assert_eq!(lease["day"], json!(padded));
    assert_eq!(
        m.call("remember_day", json!({"date": padded, "holder": "mac-b"}))
            .await
            .0,
        409
    );
    assert_refused(
        &m.call("list", json!({"day": "2026-02-30", "unjudged": true})).await,
        400,
        "BAD_REQUEST",
        "invalid day \"2026-02-30\": expected YYYY-MM-DD",
    );
}

#[tokio::test]
async fn today_and_later_cannot_be_remembered_or_harvested() {
    let m = Mem::start().await;
    let tomorrow = (chrono::Local::now() + chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    for day in [today(), tomorrow] {
        assert_refused(
            &m.call("remember_day", json!({"date": day})).await,
            400,
            "BAD_REQUEST",
            &format!("day {day} is not over yet — only past local days can be remembered"),
        );
        assert_refused(
            &m.call("harvest_day", json!({"date": day})).await,
            400,
            "BAD_REQUEST",
            &format!("day {day} is not over yet — only past local days can be scanned"),
        );
    }
    assert_refused(
        &m.call("harvest_day", json!({"date": "yesterday"})).await,
        400,
        "BAD_REQUEST",
        "invalid day \"yesterday\": expected YYYY-MM-DD",
    );
}

/// One holder per day: a second holder gets the 409 naming the first;
/// remember_day under someone else's lease gets the same.
#[tokio::test]
async fn a_dream_lease_refuses_a_second_holder() {
    let m = Mem::start().await;
    let day = days_ago(1);
    let first = m
        .ok("dream_lease", json!({"day": day, "holder": "mac-a"}))
        .await;
    let until = first["lease"]["expires_at"].as_str().unwrap();
    let until = chrono::DateTime::parse_from_rfc3339(until)
        .unwrap()
        .to_rfc3339();
    let busy = format!("day {day} is being dreamed by mac-a (until {until})");
    assert_refused(
        &m.call("dream_lease", json!({"day": day, "holder": "mac-b"}))
            .await,
        409,
        "CONFLICT",
        &busy,
    );
    assert_refused(
        &m.call("remember_day", json!({"date": day, "holder": "mac-b"}))
            .await,
        409,
        "CONFLICT",
        &busy,
    );
    assert_eq!(
        m.ok("remember_day", json!({"date": day, "holder": "mac-a"}))
            .await["date"],
        json!(day)
    );
    assert_eq!(
        m.ok(
            "dream_lease",
            json!({"day": day, "holder": "mac-a", "action": "release"})
        )
        .await,
        json!({"day": day, "released": true})
    );
    assert_eq!(
        m.ok("dream_lease", json!({"day": day, "holder": "mac-b"}))
            .await["lease"]["holder"],
        "mac-b"
    );
    assert_refused(
        &m.call(
            "dream_lease",
            json!({"day": day, "holder": "x", "action": "steal"}),
        )
        .await,
        400,
        "BAD_REQUEST",
        "action is acquire, renew or release",
    );
    assert_refused(
        &m.call("dream_lease", json!({"day": "soon", "holder": "x"}))
            .await,
        400,
        "BAD_REQUEST",
        "day is YYYY-MM-DD and holder is required",
    );
}
