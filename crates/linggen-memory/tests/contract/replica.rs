//! Two stores on two loopback ports, B made a replica of A through the
//! real routes: B asks (`PUT /api/sync/primary`), A allows
//! (`/api/connect/{id}/allow`), B picks the token up on its own 2 s poll.
//! Then a write on B is forwarded to A, B pulls A's rows, and B's write
//! while A is unreachable waits in B's outbox and replays when A returns;
//! a queued edit of a row A changed meanwhile ends as a conflict issue.
//!
//! What this does not cover: B's 30 s background loop (`replica::spawn`
//! is not started by `open_quiet`; this test calls `replica::sync_once`
//! where that loop would), and the engine's own `/mem` admission (machine
//! tokens, `machine_may`) — A here is memory's router alone.

use crate::support::*;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

async fn role(m: &Mem) -> Value {
    m.ok("role", json!({})).await
}

/// A and B, B approved as A's replica; A is reached through `front`.
async fn paired() -> (Mem, Mem, Front) {
    let a = Mem::start_under("/mem").await;
    let b = Mem::start().await;
    let front = Front::to(a.addr()).await;

    let (status, env) = b
        .put(
            "/api/sync/primary",
            json!({"primary": front.addr.to_string()}),
        )
        .await;
    assert_eq!(status, 200, "{env}");
    assert_eq!(env["data"]["role"], "replica");
    assert_eq!(env["data"]["upstream"], json!(front.addr.to_string()));
    assert_eq!(env["data"]["waiting_for_approval"], true);
    assert_eq!(env["data"]["approved"], false);

    let (_, env) = a.get("/api/connect/pending").await;
    let pending = &env["data"]["pending"];
    assert_eq!(pending["ip"], "127.0.0.1", "{env}");
    assert_eq!(pending["known"], false);
    let rid = pending["id"]
        .as_str()
        .expect("a pending request")
        .to_string();
    let (status, env) = a
        .post(&format!("/api/connect/{rid}/allow"), json!({}))
        .await;
    assert_eq!(status, 200, "{env}");

    let deadline = Instant::now() + Duration::from_secs(15);
    while role(&b).await["approved"] != true {
        assert!(
            Instant::now() < deadline,
            "B never saw the approval: {}",
            role(&b).await
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(role(&b).await["waiting_for_approval"], false);
    (a, b, front)
}

fn outbox_len(m: &Mem) -> usize {
    std::fs::read_to_string(m.data_dir().join("memory/.outbox.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok())
        .map_or(0, |q| q.len())
}

async fn contents(m: &Mem) -> Vec<String> {
    let rows = m.ok("list", json!({"sort": "oldest"})).await;
    rows.as_array()
        .unwrap()
        .iter()
        .map(|r| r["content"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_replica_forwards_pulls_and_replays_its_outbox() {
    let (a, b, front) = paired().await;

    // A write on B is A's write: forwarded, answered by A, then pulled.
    let (status, headers, env) = b
        .post_with_headers(
            "/api/memory/add",
            json!({"content": "Alex keeps bees", "tier": "semantic"}),
        )
        .await;
    assert_eq!(status, 200, "{env}");
    assert!(headers.get("x-ling-mem-queued").is_none());
    let id = env["data"]["fact"]["id"].clone();
    assert_eq!(
        a.ok("get", json!({"id": id})).await["content"],
        "Alex keeps bees"
    );
    assert_eq!(
        b.ok("get", json!({"id": id})).await["content"],
        "Alex keeps bees"
    );
    assert_eq!(outbox_len(&b), 0);
    let r = role(&b).await;
    assert_eq!(
        (r["online"].clone(), r["error"].clone(), r["queued"].clone()),
        (json!(true), Value::Null, json!(0))
    );

    // A row written on A reaches B on B's next sync.
    let on_a = a
        .add(json!({"content": "Alex sells honey", "tier": "semantic"}))
        .await;
    ling_mem::replica::sync_once(&b.state).await;
    assert_eq!(b.ok("get", json!({"id": on_a["id"]})).await, on_a);
    assert_eq!(contents(&b).await, contents(&a).await);

    // A unreachable: B's write lands locally and waits in the outbox.
    front.set_up(false);
    let (status, headers, env) = b
        .post_with_headers(
            "/api/memory/add",
            json!({"content": "Alex bought a smoker", "tier": "semantic"}),
        )
        .await;
    assert_eq!(status, 200, "{env}");
    assert_eq!(headers.get("x-ling-mem-queued").unwrap(), "1");
    let local = env["data"]["fact"]["id"].clone();
    assert_eq!(outbox_len(&b), 1);
    assert_eq!(contents(&a).await, ["Alex keeps bees", "Alex sells honey"]);
    let r = role(&b).await;
    assert_eq!(
        (r["online"].clone(), r["error"].clone(), r["queued"].clone()),
        (json!(false), json!("primary offline"), json!(1))
    );

    // A back: the replay lands on A, B's local copy gives way to A's row.
    front.set_up(true);
    ling_mem::replica::sync_once(&b.state).await;
    assert_eq!(outbox_len(&b), 0);
    assert_eq!(
        contents(&a).await,
        [
            "Alex keeps bees",
            "Alex sells honey",
            "Alex bought a smoker"
        ]
    );
    assert_eq!(contents(&b).await, contents(&a).await);
    let rows_a = a.ok("list", json!({"sort": "oldest"})).await;
    let rows_b = b.ok("list", json!({"sort": "oldest"})).await;
    assert_eq!(rows_a, rows_b, "B holds A's rows, ids and all");
    assert_ne!(
        rows_a[2]["id"], local,
        "A wrote its own row for the replayed write"
    );
    assert_refused(
        &b.call("get", json!({"id": local})).await,
        404,
        "NOT_FOUND",
        &format!("no fact with id {}", local.as_str().unwrap()),
    );
    let r = role(&b).await;
    assert_eq!(
        (r["online"].clone(), r["error"].clone(), r["queued"].clone()),
        (json!(true), Value::Null, json!(0))
    );
}

/// An edit B queued while A was unreachable, of a row A changed in the
/// meantime: A's version stands, B gets it back, and A's review queue
/// holds a `replica-conflict` issue naming the row.
#[tokio::test]
async fn a_queued_edit_of_a_row_the_primary_changed_is_a_conflict() {
    let (a, b, front) = paired().await;
    let row = b
        .post(
            "/api/memory/add",
            json!({"content": "Alex keeps two hives", "tier": "semantic"}),
        )
        .await
        .1["data"]["fact"]
        .clone();
    let id = row["id"].as_str().unwrap().to_string();

    front.set_up(false);
    let (_, headers, _) = b
        .post_with_headers(
            "/api/memory/update",
            json!({"id": id, "content": "Alex keeps three hives"}),
        )
        .await;
    assert_eq!(headers.get("x-ling-mem-queued").unwrap(), "1");
    // The primary's own edit, a moment later than the queued one.
    tokio::time::sleep(Duration::from_millis(5)).await;
    a.ok(
        "update",
        json!({"id": id, "content": "Alex keeps four hives"}),
    )
    .await;

    front.set_up(true);
    ling_mem::replica::sync_once(&b.state).await;
    assert_eq!(outbox_len(&b), 0);
    assert_eq!(
        a.ok("get", json!({"id": id})).await["content"],
        "Alex keeps four hives"
    );
    assert_eq!(
        b.ok("get", json!({"id": id})).await["content"],
        "Alex keeps four hives"
    );
    let issues = a.ok("issues", json!({})).await;
    assert_eq!(issues["open_count"], 1, "{issues}");
    let issue = &issues["issues"][0];
    assert_eq!(issue["kind"], "replica-conflict");
    assert_eq!(issue["row_ids"], json!([id]));
    let device = b.state.replica.snapshot().device_id.unwrap();
    assert_eq!(
        issue["note"],
        json!(format!(
            "changed on the primary while this machine was offline; the primary's version stands (/api/memory/update from replica {device})"
        ))
    );
}
