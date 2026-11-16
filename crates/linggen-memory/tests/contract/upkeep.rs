//! The day calendar, the dream pass's verbs, the review queue, the session
//! block, stats, config, the sync protocol, the schema step, health.

use crate::support::*;
use serde_json::{json, Value};

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn days_reports_each_day_and_the_first_undone_ones() {
    let m = Mem::start().await;
    let (d3, d2) = (days_ago(3), days_ago(2));
    m.add(json!({"content": "Alex hiked", "occurred_at": local_noon(&d3)}))
        .await;
    m.add(json!({"content": "Alex swam", "occurred_at": local_noon(&d2)}))
        .await;
    m.add(json!({"content": "Alex rested", "tier": "semantic", "occurred_at": local_noon(&d2)}))
        .await;
    let harvested = m.ok("harvest_day", json!({"date": d3})).await;
    assert_eq!(
        scrub(&harvested),
        json!({"date": d3, "record": {"harvested_at": "<time>", "judged": 0, "promoted": 0, "forgotten": 0}})
    );
    let remembered = m
        .ok(
            "remember_day",
            json!({"date": d3, "judged": 1, "promoted": 1}),
        )
        .await;
    assert_eq!(
        scrub(&remembered),
        json!({"date": d3, "record": {"harvested_at": "<time>", "remembered_at": "<time>", "judged": 1, "promoted": 1, "forgotten": 0}})
    );
    let data = m.ok("days", json!({})).await;
    let day = |date: &str, scanned: bool, dreamed: bool, unjudged: u32| {
        json!({
            "date": date, "scanned": scanned, "dreamed": dreamed, "rows": 1,
            "unjudged": unjudged, "past_ttl": 0,
            "harvested_at": if scanned { json!("<time>") } else { Value::Null },
            "remembered_at": if dreamed { json!("<time>") } else { Value::Null },
            "judged": if dreamed { 1 } else { 0 },
            "promoted": if dreamed { 1 } else { 0 },
            "forgotten": 0,
        })
    };
    assert_eq!(
        scrub(&data),
        json!({
            "today": today(),
            "ttl_days": 7,
            "open_issues": 0,
            "first_unscanned": d2,
            "first_undreamed": d2,
            "total_days": 2,
            "scanned_days": 1,
            "dreamed_days": 1,
            "row_days": {d3.clone(): 1, d2.clone(): 2},
            "days": [day(&d3, true, true, 0), day(&d2, false, false, 1)],
        })
    );
    let only = m.ok("days", json!({"undreamed_only": true})).await;
    assert_eq!(only["days"].as_array().unwrap().len(), 1);
    assert_eq!(only["days"][0]["date"], json!(d2));
    let ranged = m.ok("days", json!({"from": d2, "to": d2})).await;
    assert_eq!(ranged["days"][0]["date"], json!(d2));
    assert_refused(
        &m.call("days", json!({"from": "2026/11/01"})).await,
        400,
        "BAD_REQUEST",
        "invalid day \"2026/11/01\": expected YYYY-MM-DD",
    );
}

/// Sweep deletes staged rows past the TTL whose day was remembered after
/// they were written — and nothing else.
#[tokio::test]
async fn sweep_removes_judged_rows_past_the_ttl() {
    let m = Mem::start().await;
    let old = days_ago(20);
    let judged = m
        .add(json!({"content": "Alex hiked", "occurred_at": local_noon(&old)}))
        .await;
    m.ok("remember_day", json!({"date": old})).await;
    m.add(json!({"content": "Alex saw a lynx", "occurred_at": local_noon(&old)}))
        .await;
    m.add(json!({"content": "Alex swam", "occurred_at": local_noon(&days_ago(21))}))
        .await;
    m.add(json!({"content": "Alex kept a diary", "tier": "semantic", "occurred_at": local_noon(&old)})).await;
    assert_eq!(
        m.ok("sweep", json!({"dry_run": true})).await,
        json!({"removed": 1, "dry_run": true, "days": {old.clone(): 1}})
    );
    assert_eq!(m.ok("count", json!({})).await["count"], 4);
    assert_eq!(
        m.ok("sweep", json!({})).await,
        json!({"removed": 1, "dry_run": false, "days": {old.clone(): 1}})
    );
    assert_refused(
        &m.call("get", json!({"id": id(&judged)})).await,
        404,
        "NOT_FOUND",
        &format!("no fact with id {}", id(&judged)),
    );
    let days = m.ok("days", json!({"from": old, "to": old})).await;
    assert_eq!(days["days"][0]["forgotten"], 1);
    assert_eq!(m.ok("sweep", json!({})).await["removed"], 0);
}

#[tokio::test]
async fn chains_finds_rows_that_cite_each_other() {
    let m = Mem::start().await;
    assert_eq!(
        m.ok("chains", json!({})).await,
        json!({"chains": [], "kind": "cited", "offset": 0, "scanned": 0, "total": 0})
    );
    let first = m
        .add(json!({"content": "OPEN: the backup job fails on Sundays", "tier": "semantic"}))
        .await;
    let second = m
        .add(json!({"content": format!("fixed the backup job (closes {})", id(&first)), "tier": "semantic"}))
        .await;
    let data = m.ok("chains", json!({"kind": "cited"})).await;
    assert_eq!(data["total"], 1, "{data}");
    assert_eq!(data["scanned"], 2);
    let members: Vec<String> = data["chains"][0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(id)
        .collect();
    assert_eq!(members, vec![id(&first), id(&second)], "{data}");
    assert_eq!(data["chains"][0]["derived_only"], true, "{data}");
    let marker = m.ok("chains", json!({"kind": "marker"})).await;
    assert_eq!(marker["kind"], "marker", "{marker}");
    assert_refused(
        &m.call("chains", json!({"kind": "loops"})).await,
        400,
        "BAD_REQUEST",
        "invalid kind \"loops\": expected \"cited\", \"marker\", or \"subject\"",
    );
}

#[tokio::test]
async fn issues_open_dedup_and_resolve() {
    let m = Mem::start().await;
    assert_eq!(
        m.ok("issues", json!({})).await,
        json!({"open_count": 0, "issues": []})
    );
    let added = m
        .ok(
            "issue_add",
            json!({"kind": "contradiction", "row_ids": ["b", "a"], "note": "  two cities  "}),
        )
        .await;
    let issue = json!({"id": "<id>", "kind": "contradiction", "row_ids": ["b", "a"], "note": "two cities", "created_at": "<time>", "status": "open"});
    assert_eq!(scrub(&added), json!({"issue": issue, "deduped": false}));
    let again = m
        .ok(
            "issue_add",
            json!({"kind": "contradiction", "row_ids": ["a", "b"], "note": "same"}),
        )
        .await;
    assert_eq!(scrub(&again), json!({"issue": issue, "deduped": true}));
    assert_eq!(
        scrub(&m.ok("issues", json!({})).await),
        json!({"open_count": 1, "issues": [issue]})
    );

    let iid = id(&added["issue"]);
    let resolved = m
        .ok(
            "issue_resolve",
            json!({"id": iid, "outcome": "resolved", "note": "Lisbon stands"}),
        )
        .await;
    let mut closed = issue.clone();
    closed["status"] = json!("resolved");
    closed["resolved_at"] = json!("<time>");
    closed["resolution"] = json!("Lisbon stands");
    assert_eq!(
        scrub(&resolved),
        json!({"issue": closed, "already_closed": false})
    );
    let twice = m
        .ok("issue_resolve", json!({"id": iid, "outcome": "dismissed"}))
        .await;
    assert_eq!(
        scrub(&twice),
        json!({"issue": closed, "already_closed": true})
    );
    assert_eq!(
        m.ok("issues", json!({})).await,
        json!({"open_count": 0, "issues": []})
    );
    assert_eq!(
        scrub(&m.ok("issues", json!({"status": "all"})).await)["issues"],
        json!([closed])
    );

    let refusals = [
        (json!({"kind": "rumour", "note": "x"}), "issue_add", "invalid kind \"rumour\": expected one of [\"chain\", \"stale-status\", \"contradiction\", \"subject\", \"replica-conflict\"]"),
        (json!({"kind": "chain", "note": " "}), "issue_add", "note is required"),
        (json!({"id": iid, "outcome": "fixed"}), "issue_resolve", "invalid outcome \"fixed\": expected resolved | dismissed"),
        (json!({"status": "closed"}), "issues", "invalid status \"closed\": expected open | resolved | dismissed | all"),
    ];
    for (body, verb, why) in refusals {
        assert_refused(&m.call(verb, body).await, 400, "BAD_REQUEST", why);
    }
    // Today a missing issue id is a 400, not a 404.
    assert_refused(
        &m.call(
            "issue_resolve",
            json!({"id": "nope", "outcome": "resolved"}),
        )
        .await,
        400,
        "BAD_REQUEST",
        "no issue with id \"nope\"",
    );
}

#[tokio::test]
async fn session_start_answers_core_memory_as_a_block() {
    let m = Mem::start().await;
    assert_eq!(
        m.ok("session_start", json!({})).await,
        json!({
            "block": "", "candidates": [], "candidates_line": "", "chars": 0,
            "core": [], "core_block": "", "index": [], "index_block": "", "index_skipped": 0,
        })
    );
    let core = m
        .add(json!({"content": "Alex lives in Lisbon", "tier": "core", "from": "user"}))
        .await;
    m.add(json!({"content": "Sam lives in Porto", "tier": "core", "account_id": "u-sam"}))
        .await;
    m.add(json!({"content": "Alex keeps bees", "tier": "semantic"}))
        .await;
    let block = format!(
        "## Core memory — who the user is\n\n- Alex lives in Lisbon (id={})",
        id(&core)
    );
    assert_eq!(
        m.ok("session_start", json!({})).await,
        json!({
            "block": block, "candidates": [], "candidates_line": "", "chars": block.chars().count(),
            "core": [core], "core_block": block, "index": [], "index_block": "", "index_skipped": 0,
        })
    );
    let sams = m.ok("session_start", json!({"account": "u-sam"})).await;
    assert_eq!(sams["core"][0]["content"], "Sam lives in Porto");
    assert_eq!(
        m.ok("session_start", json!({"core": false})).await["core"],
        json!([])
    );
}

#[tokio::test]
async fn stats_counts_each_tier() {
    let m = Mem::start().await;
    m.add(json!({"content": "Alex lives in Lisbon", "tier": "core"}))
        .await;
    let kept = m
        .add(json!({"content": "Alex uses vim", "tier": "semantic"}))
        .await;
    m.add(json!({"content": "Alex uses helix", "replace_ids": [id(&kept)]}))
        .await;
    m.add(json!({"content": "Alex tried emacs"})).await;
    let mut data = m.ok("stats", json!({})).await;
    let disk = data.as_object_mut().unwrap().remove("disk_bytes").unwrap();
    for k in ["semantic", "episodic", "total"] {
        assert!(disk[k].as_u64().unwrap() > 0, "{disk}");
    }
    assert_eq!(
        data,
        json!({
            "embedding_model": "Qwen/Qwen3-Embedding-0.6B",
            "expired": 1,
            "last_remembered_at": null,
            "open_issues": 0,
            "per_tier": {"core": 1, "semantic": 1, "episodic": 1},
            "remembered_days": 0,
            "schema_version": 2,
            "total": 3,
            "ttl_days": 7,
        })
    );
}

/// The two knobs. (`network` rides in the same file; another change owns
/// it, so it is left out here.)
#[tokio::test]
async fn config_reads_and_writes_the_two_knobs() {
    let m = Mem::start().await;
    let knobs = |mut v: Value| {
        v.as_object_mut().unwrap().remove("network");
        v
    };
    let (status, env) = m.get("/api/config").await;
    assert_eq!(status, 200);
    assert_eq!(env["ok"], true);
    assert_eq!(env.as_object().unwrap().len(), 2);
    assert_eq!(
        knobs(env["data"].clone()),
        json!({"episodic_ttl_days": 7, "recall_min_score": 0.699999988079071})
    );
    let (status, env) = m
        .put(
            "/api/config",
            json!({"episodic_ttl_days": 14, "recall_min_score": 0.5}),
        )
        .await;
    assert_eq!(status, 200);
    let mut want = json!({"episodic_ttl_days": 14, "recall_min_score": 0.5});
    assert_eq!(knobs(env["data"].clone()), want);
    assert_eq!(knobs(m.get("/api/config").await.1["data"].clone()), want);
    // A field left out takes its default.
    m.put("/api/config", json!({"episodic_ttl_days": 3})).await;
    want = json!({"episodic_ttl_days": 3, "recall_min_score": 0.699999988079071});
    assert_eq!(knobs(m.get("/api/config").await.1["data"].clone()), want);
    assert_refused(
        &m.put("/api/config", json!({"episodic_ttl_days": 4000}))
            .await,
        400,
        "BAD_REQUEST",
        "episodic_ttl_days must be <= 3650 (10 years)",
    );
    assert_refused(
        &m.put("/api/config", json!({"recall_min_score": 1.5})).await,
        400,
        "BAD_REQUEST",
        "recall_min_score must be between 0.0 and 1.0",
    );
}

/// What a replica pulls: rows with their vectors and table, tombstones,
/// the calendar and the queue, paged by sequence.
#[tokio::test]
async fn sync_serves_changes_rows_and_role() {
    let m = Mem::start().await;
    let (status, env) = m.get("/api/sync/role").await;
    assert_eq!(status, 200);
    let primary = json!({
        "role": "primary", "upstream": null, "approved": false, "waiting_for_approval": false,
        "online": false, "last_sync": null, "queued": 0, "error": null,
    });
    assert_eq!(env, json!({"ok": true, "data": primary}));
    assert_eq!(m.ok("role", json!({})).await, primary);

    let a = m
        .add(json!({"content": "Alex keeps bees", "tier": "semantic"}))
        .await;
    let b = m.add(json!({"content": "Alex checked the hive"})).await;
    m.ok("delete", json!({"id": id(&b)})).await;
    m.ok("issue_add", json!({"kind": "chain", "note": "n"}))
        .await;

    let (_, env) = m.get("/api/sync/changes?since=0&limit=1").await;
    let page = &env["data"];
    assert_eq!(page["model"], "Qwen/Qwen3-Embedding-0.6B");
    assert_eq!(
        (
            page["head"].clone(),
            page["next"].clone(),
            page["more"].clone()
        ),
        (json!(3), json!(1), json!(true))
    );
    assert_eq!(page["rows"][0]["table"], "semantic");
    assert_eq!(page["rows"][0]["row"]["id"], json!(id(&a)));
    assert_eq!(
        page["rows"][0]["row"]["vector"].as_array().unwrap().len(),
        1024
    );
    assert_eq!(page["days"], Value::Null);
    assert_eq!(page["issues"]["issues"][0]["kind"], "chain");

    let (_, env) = m.get("/api/sync/changes?since=1").await;
    let page = &env["data"];
    assert_eq!(
        (page["next"].clone(), page["more"].clone()),
        (json!(3), json!(false))
    );
    // The staged row was written and deleted: one tombstone for it.
    assert_eq!(page["rows"], json!([]));
    assert_eq!(page["tombstones"], json!([id(&b)]));

    let (_, env) = m
        .get(&format!("/api/sync/rows?ids={},{},zz", id(&a), id(&b)))
        .await;
    let data = &env["data"];
    assert_eq!(data["rows"].as_array().unwrap().len(), 1);
    assert_eq!(data["rows"][0]["row"]["content"], "Alex keeps bees");
    assert_eq!(data["tombstones"], json!([id(&b), "zz"]));
}

#[tokio::test]
async fn the_schema_step_runs_on_a_scratch_store_and_backs_it_up() {
    let m = Mem::start().await;
    m.add(json!({"content": "Alex keeps bees", "tier": "semantic"}))
        .await;
    assert_refused(
        &m.post("/api/schema/apply", json!({})).await,
        400,
        "BAD_REQUEST",
        "the v2 step drops the contexts and tags columns — pass confirm:true",
    );
    let (status, env) = m.post("/api/schema/apply", json!({"confirm": true})).await;
    assert_eq!(status, 200, "{env}");
    let data = &env["data"];
    assert_eq!(data["schema_version"], 2);
    let backup = std::path::PathBuf::from(data["backup_dir"].as_str().unwrap());
    assert!(
        backup.starts_with(m.data_dir().join("memory/backups")),
        "{backup:?}"
    );
    assert!(backup.join("memory.lancedb").is_dir());
    assert_eq!(m.ok("count", json!({})).await["count"], 1, "rows survive");
}

#[tokio::test]
async fn health_answers_the_version() {
    let m = Mem::start().await;
    assert_eq!(
        m.get("/api/health").await,
        (
            200,
            json!({"ok": true, "data": {"status": "healthy", "version": env!("CARGO_PKG_VERSION")}})
        )
    );
}
