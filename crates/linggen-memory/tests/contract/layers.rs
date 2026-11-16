//! The two layers on the hosted router that a direct function call would
//! skip: replica write-forwarding (`replica::forward::replica_writes`) and
//! the activity stamp background upkeep waits on (`mark_activity_layer`).

use crate::support::*;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// A store that is a replica of a primary nobody answers for.
async fn replica_of_nobody() -> Mem {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("memory")).unwrap();
    std::fs::write(
        dir.path().join("memory/.replica.json"),
        r#"{"primary": "127.0.0.1:1", "token": "tok-b", "device_id": "dev-b-0001"}"#,
    )
    .unwrap();
    Mem::open(dir, "").await
}

fn outbox(m: &Mem) -> Value {
    let text = std::fs::read_to_string(m.data_dir().join("memory/.outbox.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Every write route lands locally and is queued (the route path as its
/// `verb`, the body as sent), the answer marked `x-ling-mem-queued: 1`.
#[tokio::test]
async fn a_write_with_the_primary_unreachable_is_queued_in_the_outbox() {
    let m = replica_of_nobody().await;
    let body = json!({"content": "Alex keeps bees", "tier": "semantic"});
    let (status, headers, env) = m.post_with_headers("/api/memory/add", body.clone()).await;
    assert_eq!(status, 200, "{env}");
    assert_eq!(env["data"]["action"], "added");
    assert_eq!(headers.get("x-ling-mem-queued").unwrap(), "1");
    let local = env["data"]["fact"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        m.ok("get", json!({"id": local})).await["content"],
        "Alex keeps bees"
    );

    let issue = json!({"kind": "chain", "note": "n"});
    let (_, headers, _) = m
        .post_with_headers("/api/memory/issue_add", issue.clone())
        .await;
    assert_eq!(headers.get("x-ling-mem-queued").unwrap(), "1");
    // A read is answered here and queues nothing.
    let (_, headers, _) = m.post_with_headers("/api/memory/list", json!({})).await;
    assert!(headers.get("x-ling-mem-queued").is_none());
    // A refused write is not queued.
    let (status, headers, _) = m
        .post_with_headers("/api/memory/add", json!({"content": ""}))
        .await;
    assert_eq!(status, 400);
    assert!(headers.get("x-ling-mem-queued").is_none());

    let mut q = outbox(&m);
    for e in q.as_array_mut().unwrap() {
        assert!(e.as_object_mut().unwrap().remove("created_at").is_some());
    }
    assert_eq!(
        q,
        json!([
            {"seq": 1, "verb": "/api/memory/add", "args": body, "replica_id": "dev-b-0001", "local_ids": [local]},
            {"seq": 2, "verb": "/api/memory/issue_add", "args": issue, "replica_id": "dev-b-0001", "local_ids": []},
        ])
    );
    let role = m.ok("role", json!({})).await;
    assert_eq!(
        role,
        json!({
            "role": "replica", "upstream": "127.0.0.1:1", "approved": true, "waiting_for_approval": false,
            "online": false, "last_sync": null, "queued": 2, "error": "primary offline",
        })
    );
}

/// The dream pass's routes run on the primary only: a replica refuses them
/// with 409 `REPLICA`, naming the primary.
#[tokio::test]
async fn a_primary_only_route_is_refused_on_a_replica() {
    let m = replica_of_nobody().await;
    for (path, body) in [
        ("/api/memory/remember_day", json!({"date": days_ago(1)})),
        ("/api/memory/sweep", json!({})),
        (
            "/api/memory/dream_lease",
            json!({"day": days_ago(1), "holder": "x"}),
        ),
        ("/api/schema/apply", json!({"confirm": true})),
    ] {
        assert_refused(
            &m.post(path, body).await,
            409,
            "REPLICA",
            "Memory is dreamed on the primary (127.0.0.1:1)",
        );
    }
    // harvest_day is a write: queued, not refused.
    let (status, headers, _) = m
        .post_with_headers("/api/memory/harvest_day", json!({"date": days_ago(1)}))
        .await;
    assert_eq!(status, 200);
    assert_eq!(headers.get("x-ling-mem-queued").unwrap(), "1");
}

/// Every request stamps the clock the quiet-window upkeep reads.
#[tokio::test]
async fn a_request_stamps_the_activity_clock() {
    let m = Mem::start().await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(m.state.idle_for() >= Duration::from_millis(60));
    for (path, get) in [
        ("/api/health", true),
        ("/api/memory/count", false),
        ("/api/memory/wat", false),
    ] {
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(m.state.idle_for() >= Duration::from_millis(60), "{path}");
        let sent = Instant::now();
        if get {
            m.get(path).await;
        } else {
            m.post(path, json!({})).await;
        }
        // The clock keeps whole milliseconds: allow one.
        let idle = m.state.idle_for();
        assert!(
            idle <= sent.elapsed() + Duration::from_millis(1),
            "{path}: {idle:?}"
        );
    }
}
