//! Each row verb's happy path, with the fields its callers read.

use crate::support::*;
use serde_json::{json, Value};

const NOTES: &str = "/opt/alex/notes";

#[tokio::test]
async fn add_answers_the_row_it_wrote() {
    let m = Mem::start().await;
    let data = m
        .ok(
            "add",
            json!({
                "content": "Alex prefers green tea in the morning",
                "type": "preference",
                "from": "user",
                "tier": "semantic",
                "outcome": "positive",
                "cwd": NOTES,
                "summary": "  tea   in the\nmorning ",
                "indexed": true,
                "occurred_at": "2026-11-10T12:00:00Z",
                "source_session": "s-1",
                "host": "claude-code",
            }),
        )
        .await;
    assert_eq!(
        scrub(&data),
        json!({
            "action": "added",
            "fact": row("Alex prefers green tea in the morning", "preference", "semantic", "user", json!({
                "outcome": "positive",
                "scope": NOTES,
                "summary": "tea in the morning",
                "indexed": true,
                "occurred_at": "2026-11-10T12:00:00Z",
                "source_session": "s-1",
                "host": "claude-code",
            })),
        })
    );
    assert!(data["fact"]["vector"].is_null(), "no vector on the wire");
    assert_eq!(data["fact"]["id"].as_str().unwrap().len(), 10);
}

/// Today an omitted tier is episodic, an omitted `from` is `derived`, an
/// omitted type is `fact`; `episodic: true` and `tier: "episodic"` agree.
#[tokio::test]
async fn add_defaults_to_an_episodic_derived_fact() {
    let m = Mem::start().await;
    for body in [
        json!({"content": "Alex walked the dog at noon"}),
        json!({"content": "Alex fixed the bike chain", "episodic": true, "tier": "core"}),
        json!({"content": "Alex read about tides", "tier": "episodic"}),
    ] {
        let fact = m.add(body.clone()).await;
        let content = body["content"].as_str().unwrap();
        assert_eq!(
            scrub(&fact),
            row(content, "fact", "episodic", "derived", json!({}))
        );
    }
    assert_eq!(m.ok("count", json!({"episodic": true})).await["count"], 3);
    assert_eq!(m.ok("count", json!({"episodic": false})).await["count"], 0);
}

#[tokio::test]
async fn add_batch_writes_every_non_blank_row_as_given() {
    let m = Mem::start().await;
    let data = m
        .ok(
            "add_batch",
            json!({"facts": [
                {"content": "Alex keeps bees", "tier": "semantic", "scope": "/opt/alex/bees"},
                {"content": "   "},
                {"content": "Alex checked the hive today"},
                {"content": "Alex is a beekeeper", "tier": "core", "scope": "/opt/alex/bees", "indexed": true},
                {"content": "Alex sells honey", "tier": "semantic", "cwd": "/opt/alex/shop", "global": true},
            ]}),
        )
        .await;
    assert_eq!(
        scrub(&data),
        json!({
            "action": "added",
            "count": 4,
            "facts": [
                row("Alex keeps bees", "fact", "semantic", "derived", json!({"scope": "/opt/alex/bees"})),
                row("Alex is a beekeeper", "fact", "core", "derived", json!({})),
                row("Alex sells honey", "fact", "semantic", "derived", json!({})),
                row("Alex checked the hive today", "fact", "episodic", "derived", json!({})),
            ],
        })
    );
}

/// Today `add_batch` does not dedup: the same row twice is two rows.
#[tokio::test]
async fn add_batch_does_not_dedup() {
    let m = Mem::start().await;
    let one = json!({"content": "Alex keeps bees", "tier": "semantic"});
    let data = m.ok("add_batch", json!({"facts": [one, one]})).await;
    assert_eq!(data["count"], 2);
    assert_eq!(m.ok("count", json!({})).await["count"], 2);
}

#[tokio::test]
async fn get_finds_a_row_in_either_table_unless_told_which() {
    let m = Mem::start().await;
    let sem = m
        .add(json!({"content": "Alex plays chess", "tier": "semantic"}))
        .await;
    let epi = m.add(json!({"content": "Alex lost a chess game"})).await;
    for row in [&sem, &epi] {
        assert_eq!(m.ok("get", json!({"id": row["id"]})).await, *row);
    }
    let missing = |id: &Value| format!("no fact with id {}", id.as_str().unwrap());
    assert_refused(
        &m.call("get", json!({"id": sem["id"], "episodic": true}))
            .await,
        404,
        "NOT_FOUND",
        &missing(&sem["id"]),
    );
    assert_refused(
        &m.call("get", json!({"id": epi["id"], "episodic": false}))
            .await,
        404,
        "NOT_FOUND",
        &missing(&epi["id"]),
    );
}

#[tokio::test]
async fn search_finds_the_row_and_never_returns_its_vector() {
    let m = Mem::start().await;
    let tea = m
        .add(json!({"content": "Alex prefers green tea in the morning", "tier": "semantic"}))
        .await;
    m.add(json!({"content": "the deploy script needs port 4100", "tier": "semantic"}))
        .await;

    // The row's own words: cosine 1.0, keyword boost 0.3, shown clamped.
    let hits = m
        .ok(
            "search",
            json!({"query": "Alex prefers green tea in the morning"}),
        )
        .await;
    let mut want = tea.clone();
    want["score"] = json!(1.0);
    want["hybrid_score"] = json!(1.0);
    want["raw_hybrid"] = json!(1.2999999523162842);
    want["scope_weight"] = json!(1.0);
    assert_eq!(hits, json!([want]));

    // Two of its seven words: cosine ≈ 0.53, over the 0.7 floor only with
    // the keyword boost.
    let hits = m.ok("search", json!({"query": "green tea"})).await;
    assert_eq!(hits.as_array().unwrap().len(), 1);
    assert_eq!(hits[0]["id"], tea["id"]);
    assert_eq!(hits[0]["score"], json!(0.534522533416748));
    assert_eq!(hits[0]["hybrid_score"], json!(0.834522545337677));
    assert!(hits[0].get("vector").is_none());

    // Above that, or with no shared word: nothing.
    let none = m
        .ok("search", json!({"query": "green tea", "min_score": 0.9}))
        .await;
    assert_eq!(none, json!([]));
    assert_eq!(
        m.ok("search", json!({"query": "violin lessons"})).await,
        json!([])
    );
    // A table that does not hold it: nothing.
    let other = m
        .ok("search", json!({"query": "green tea", "episodic": true}))
        .await;
    assert_eq!(other, json!([]));
}

#[tokio::test]
async fn search_refuses_a_blank_query() {
    let m = Mem::start().await;
    assert_refused(
        &m.call("search", json!({"query": " "})).await,
        400,
        "BAD_REQUEST",
        "query must not be empty",
    );
}

/// The list filters callers use: tier, type, from, day, since, until,
/// limit, offset, sort; both tables by default.
#[tokio::test]
async fn list_filters_sorts_and_pages() {
    let m = Mem::start().await;
    let noon = |days: i64| {
        let d = chrono::Local::now().date_naive() - chrono::Days::new(days as u64);
        let local = d.and_hms_opt(12, 0, 0).unwrap();
        let utc = chrono::TimeZone::from_local_datetime(&chrono::Local, &local)
            .earliest()
            .unwrap()
            .with_timezone(&chrono::Utc);
        (d.format("%Y-%m-%d").to_string(), utc.to_rfc3339())
    };
    let (day3, at3) = noon(3);
    let (_, at1) = noon(1);
    let a = m
        .add(json!({"content": "Alex lives in Lisbon", "tier": "core", "from": "user"}))
        .await;
    let b = m
        .add(json!({"content": "Alex chose Rust for the tool", "type": "decision", "tier": "semantic", "occurred_at": at3}))
        .await;
    let c = m
        .add(json!({"content": "Alex tried a new bread recipe", "type": "tried", "occurred_at": at1}))
        .await;
    let ids = |v: Value| -> Vec<Value> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].clone())
            .collect()
    };

    assert_eq!(
        ids(m.ok("list", json!({})).await),
        vec![c["id"].clone(), b["id"].clone(), a["id"].clone()]
    );
    assert_eq!(
        ids(m.ok("list", json!({"sort": "oldest"})).await),
        vec![a["id"].clone(), b["id"].clone(), c["id"].clone()]
    );
    assert_eq!(
        ids(m.ok("list", json!({"limit": 1, "offset": 1})).await),
        vec![b["id"].clone()]
    );
    assert_eq!(
        ids(m.ok("list", json!({"tier": "core"})).await),
        vec![a["id"].clone()]
    );
    assert_eq!(
        ids(m.ok("list", json!({"type": "decision"})).await),
        vec![b["id"].clone()]
    );
    assert_eq!(
        ids(m.ok("list", json!({"from": "user"})).await),
        vec![a["id"].clone()]
    );
    assert_eq!(
        ids(m.ok("list", json!({"day": day3})).await),
        vec![b["id"].clone()]
    );
    assert_eq!(
        ids(m.ok("list", json!({"episodic": true})).await),
        vec![c["id"].clone()]
    );
    // since/until bound COALESCE(occurred_at, created_at).
    let (_, at2) = noon(2);
    assert_eq!(
        ids(m.ok("list", json!({"until": at2})).await),
        vec![b["id"].clone()]
    );
    assert_eq!(
        ids(m.ok("list", json!({"since": at2, "sort": "oldest"})).await),
        vec![a["id"].clone(), c["id"].clone()]
    );
    // Lenient shapes: "" is no filter; a date-only bound is midnight UTC.
    assert_eq!(
        m.ok("list", json!({"type": "", "from": "", "since": ""}))
            .await
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_refused(
        &m.call("list", json!({"day": "11/16"})).await,
        400,
        "BAD_REQUEST",
        "invalid day \"11/16\": expected YYYY-MM-DD",
    );
    // The rows carry no vector.
    assert!(m
        .ok("list", json!({}))
        .await
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r.get("vector").is_none()));
}

#[tokio::test]
async fn count_answers_rows_and_the_latest_activity() {
    let m = Mem::start().await;
    assert_eq!(
        m.ok("count", json!({})).await,
        json!({"count": 0, "latest_created_at": null})
    );
    m.add(json!({"content": "Alex plays chess", "tier": "semantic"}))
        .await;
    let last = m.add(json!({"content": "Alex lost a chess game"})).await;
    let data = m.ok("count", json!({})).await;
    assert_eq!(data["count"], 2);
    assert_eq!(data["latest_created_at"], last["created_at"]);
    assert_eq!(m.ok("count", json!({"tier": "semantic"})).await["count"], 1);
}

/// Update's set/clear twins; set wins over clear when both are given.
#[tokio::test]
async fn update_sets_and_clears_each_field() {
    let m = Mem::start().await;
    let id = m
        .add(json!({"content": "Alex runs on Sundays", "tier": "semantic"}))
        .await["id"]
        .clone();
    let set = m
        .ok(
            "update",
            json!({
                "id": id,
                "content": "Alex runs on Saturdays",
                "type": "preference",
                "outcome": "neutral",
                "scope": "/opt/alex/run",
                "host": "codex",
                "summary": "runs Saturdays",
                "indexed": true,
                "occurred_at": "2026-11-01",
            }),
        )
        .await;
    assert_eq!(
        scrub(&set),
        row(
            "Alex runs on Saturdays",
            "preference",
            "semantic",
            "derived",
            json!({
                "outcome": "neutral",
                "scope": "/opt/alex/run",
                "host": "codex",
                "summary": "runs Saturdays",
                "indexed": true,
                "occurred_at": "2026-11-01T00:00:00Z",
                "updated_at": "<time>",
            })
        )
    );
    let cleared = m
        .ok(
            "update",
            json!({
                "id": id,
                "clear_outcome": true,
                "clear_scope": true,
                "clear_host": true,
                "clear_summary": true,
                "clear_occurred_at": true,
                "indexed": false,
            }),
        )
        .await;
    assert_eq!(
        scrub(&cleared),
        row(
            "Alex runs on Saturdays",
            "preference",
            "semantic",
            "derived",
            json!({"updated_at": "<time>"})
        )
    );
    let both = m
        .ok(
            "update",
            json!({"id": id, "outcome": "positive", "clear_outcome": true, "global": true}),
        )
        .await;
    assert_eq!(both["outcome"], "positive");
    assert!(both.get("scope").is_none());
    assert_refused(
        &m.call("update", json!({"id": id, "scope": "notes/run"}))
            .await,
        400,
        "BAD_REQUEST",
        "scope \"notes/run\" is not an absolute directory (or ~/…)",
    );
}

/// A tier change across the table line moves the row and keeps its id.
#[tokio::test]
async fn update_moves_a_row_across_tables_keeping_its_id() {
    let m = Mem::start().await;
    let staged = m
        .add(json!({"content": "Alex learned to splice rope"}))
        .await;
    let id = staged["id"].clone();
    let moved = m.ok("update", json!({"id": id, "tier": "semantic"})).await;
    assert_eq!(moved["id"], id);
    assert_eq!(moved["tier"], "semantic");
    assert_eq!(m.ok("count", json!({"episodic": true})).await["count"], 0);
    assert_eq!(
        m.ok("get", json!({"id": id, "episodic": false})).await["tier"],
        "semantic"
    );
    let back = m.ok("update", json!({"id": id, "tier": "episodic"})).await;
    assert_eq!(back["tier"], "episodic");
    assert_eq!(
        m.ok("get", json!({"id": id, "episodic": true})).await["id"],
        id
    );
    assert_refused(
        &m.call("update", json!({"id": "gone", "content": "x"}))
            .await,
        404,
        "NOT_FOUND",
        "no fact with id gone",
    );
}

#[tokio::test]
async fn delete_answers_removed_and_a_second_delete_answers_false() {
    let m = Mem::start().await;
    let id = m.add(json!({"content": "Alex lost a chess game"})).await["id"].clone();
    assert_eq!(
        m.ok("delete", json!({"id": id})).await,
        json!({"id": id, "removed": true})
    );
    assert_eq!(
        m.ok("delete", json!({"id": id})).await,
        json!({"id": id, "removed": false})
    );
}

#[tokio::test]
async fn forget_removes_what_the_filter_names_in_one_table() {
    let m = Mem::start().await;
    m.add(json!({"content": "Alex tried oat milk", "type": "learned", "tier": "semantic"}))
        .await;
    m.add(json!({"content": "Alex tried soy milk", "type": "learned"}))
        .await;
    m.add(json!({"content": "Alex likes rye bread", "tier": "semantic"}))
        .await;
    assert_eq!(
        m.ok("forget", json!({"type": "learned"})).await,
        json!({"removed": 1})
    );
    assert_eq!(
        m.ok("forget", json!({"type": "learned", "episodic": true}))
            .await,
        json!({"removed": 1})
    );
    assert_eq!(m.ok("count", json!({})).await["count"], 1);
}

#[tokio::test]
async fn restamp_moves_a_devices_rows_and_accounts_lists_people() {
    let m = Mem::start().await;
    let stamp =
        |c: &str| json!({"content": c, "account_id": "device:d1", "account_name": "Alex's phone"});
    m.add(stamp("Alex likes jazz")).await;
    m.ok("add_batch", json!({"facts": [stamp("Alex likes blues")]}))
        .await;
    m.add(json!({"content": "the owner likes folk"})).await;
    assert_eq!(
        m.ok("accounts", json!({})).await,
        json!([{"account_id": "device:d1", "account_name": "Alex's phone", "rows": 2}])
    );
    assert_eq!(
        m.ok(
            "restamp",
            json!({"from": "device:d1", "account_id": "u-alex", "account_name": "Alex"})
        )
        .await,
        json!({"from": "device:d1", "account_id": "u-alex", "moved": 2})
    );
    assert_eq!(
        m.ok("accounts", json!({})).await,
        json!([{"account_id": "u-alex", "account_name": "Alex", "rows": 2}])
    );
    // To the owner: the stamp comes off.
    assert_eq!(
        m.ok("restamp", json!({"from": "u-alex"})).await,
        json!({"from": "u-alex", "account_id": null, "moved": 2})
    );
    assert_eq!(m.ok("accounts", json!({})).await, json!([]));
    assert_refused(
        &m.call("restamp", json!({"from": " "})).await,
        400,
        "BAD_REQUEST",
        "from must not be empty",
    );
}
