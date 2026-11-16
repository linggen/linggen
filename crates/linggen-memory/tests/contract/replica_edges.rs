//! What a replica answers at the edges, through the real router: a write the
//! primary refuses, a write carrying a field this store does not know, a
//! body nobody can read — on a primary, on a replica whose primary answers,
//! and on one whose primary does not — and that a read never leaves the
//! machine.
//!
//! B is made A's replica by writing its `.replica.json` before it opens (the
//! hosted router has no gate of its own, so A takes any token).

use crate::support::*;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;

/// A store that follows the primary at `primary` (`host:port`).
async fn replica_of(primary: &str) -> Mem {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("memory")).unwrap();
    let file = json!({"primary": primary, "token": "tok-b", "device_id": "dev-b-0001"});
    std::fs::write(dir.path().join("memory/.replica.json"), file.to_string()).unwrap();
    Mem::open(dir, "").await
}

/// A, and B its replica reaching A through `front`.
async fn pair() -> (Mem, Mem, Front) {
    let a = Mem::start_under("/mem").await;
    let front = Front::to(a.addr()).await;
    let b = replica_of(&front.addr.to_string()).await;
    (a, b, front)
}

fn outbox(m: &Mem) -> Value {
    std::fs::read_to_string(m.data_dir().join("memory/.outbox.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(json!([]))
}

/// The primary refuses a write a replica forwarded: the replica answers
/// with the primary's status and body, applies nothing and queues nothing.
#[tokio::test]
async fn a_refusal_from_the_primary_comes_back_as_it_came() {
    let (a, b, _front) = pair().await;
    let voice = a
        .add(json!({"content": "Alex prefers tea", "from": "user", "tier": "semantic"}))
        .await;
    let replace = json!({"content": "Alex now prefers coffee", "replace_ids": [voice["id"]]});
    let on_a = a.call("add", replace.clone()).await;
    assert_eq!(on_a.0, 400, "{}", on_a.1);
    assert_eq!(on_a.1["code"], "BAD_REQUEST");

    let (status, headers, env) = b.post_with_headers("/api/memory/add", replace).await;
    assert_eq!((status, env), on_a);
    assert!(headers.get("x-ling-mem-queued").is_none());
    assert_eq!(outbox(&b), json!([]));
    assert_eq!(b.ok("count", json!({})).await["count"], 0);
    assert_eq!(a.ok("count", json!({})).await["count"], 1);
}

/// Offline, a write is queued with its body exactly as sent — a field the
/// request type does not name included.
#[tokio::test]
async fn a_queued_write_keeps_fields_this_store_does_not_know() {
    let (_a, b, front) = pair().await;
    front.set_up(false);
    let body = json!({
        "content": "Alex keeps bees",
        "tier": "semantic",
        "mood": "sunny",
        "extra": {"nested": [1, 2.5, null]},
    });
    let (status, headers, env) = b.post_with_headers("/api/memory/add", body.clone()).await;
    assert_eq!(status, 200, "{env}");
    assert_eq!(headers.get("x-ling-mem-queued").unwrap(), "1");
    let q = outbox(&b);
    assert_eq!(q.as_array().unwrap().len(), 1, "{q}");
    assert_eq!(q[0]["args"], body);
    assert_eq!(q[0]["verb"], "/api/memory/add");
}

const MISSING: &str = "Failed to deserialize the JSON body into the target type: missing field `content` at line 1 column 16";
const NOT_JSON: &str =
    "Failed to parse the request body as JSON: key must be a string at line 1 column 2";
const NO_TYPE: &str = "Expected request with `Content-Type: application/json`";

/// Three bodies the add handler cannot read, sent to `m`.
async fn malformed(m: &Mem) -> [(u16, Value); 3] {
    [
        m.post_raw("/api/memory/add", r#"{"contents":"x"}"#, "application/json")
            .await,
        m.post_raw("/api/memory/add", "{not json", "application/json")
            .await,
        m.post_raw(
            "/api/memory/add",
            r#"{"content":"Alex hums"}"#,
            "text/plain",
        )
        .await,
    ]
}

/// On a primary, and on a replica whose primary is down: axum's own plain
/// text refusals, nothing written, nothing queued.
#[tokio::test]
async fn a_malformed_write_is_refused_here_when_the_primary_is_not_asked() {
    let expected = [
        (422, json!(MISSING)),
        (400, json!(NOT_JSON)),
        (415, json!(NO_TYPE)),
    ];
    let primary = Mem::start().await;
    assert_eq!(malformed(&primary).await, expected);
    assert_eq!(primary.ok("count", json!({})).await["count"], 0);

    let (_a, b, front) = pair().await;
    front.set_up(false);
    assert_eq!(malformed(&b).await, expected);
    assert_eq!(outbox(&b), json!([]));
    assert_eq!(b.ok("count", json!({})).await["count"], 0);
}

/// On a replica whose primary answers, the body goes to the primary as it
/// came (a body that is not JSON goes as `null`, and always as JSON): the
/// primary's refusal comes back with its status and a `null` body, and a
/// readable body sent as text/plain is the primary's write.
#[tokio::test]
async fn a_malformed_write_on_a_replica_is_the_primarys_to_judge() {
    let (a, b, _front) = pair().await;
    let [missing, not_json, no_type] = malformed(&b).await;
    assert_eq!(missing, (422, Value::Null));
    assert_eq!(not_json, (422, Value::Null));
    assert_eq!(no_type.0, 200, "{}", no_type.1);
    assert_eq!(no_type.1["data"]["action"], "added");
    assert_eq!(outbox(&b), json!([]));
    assert_eq!(a.ok("count", json!({})).await["count"], 1);
    assert_eq!(b.ok("count", json!({})).await["count"], 1, "B pulled it");
}

/// The verbs that take no arguments read no body: any body, any type.
#[tokio::test]
async fn a_verb_without_arguments_ignores_its_body() {
    let primary = Mem::start().await;
    let (_a, b, front) = pair().await;
    front.set_up(false);
    for m in [&primary, &b] {
        for verb in ["accounts", "stats", "role"] {
            let path = format!("/api/memory/{verb}");
            let (status, env) = m.post_raw(&path, "{not json", "text/plain").await;
            assert_eq!(status, 200, "{verb}: {env}");
            assert_eq!(env["ok"], true, "{verb}");
        }
    }
    assert_eq!(outbox(&b), json!([]));
}

/// A replica answers every read from its own copy: the primary it names
/// never sees a connection.
#[tokio::test]
async fn a_read_on_a_replica_never_leaves_the_machine() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let primary = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = accepted.clone();
    let watcher = tokio::spawn(async move {
        while let Ok((conn, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            drop(conn);
        }
    });
    let b = replica_of(&primary.to_string()).await;
    let reads = [
        ("search", json!({"query": "bees"})),
        ("list", json!({})),
        ("count", json!({})),
        ("get", json!({"id": "no-such-row"})),
        ("accounts", json!({})),
        ("days", json!({})),
        ("session_start", json!({})),
        ("chains", json!({})),
        ("issues", json!({})),
        ("stats", json!({})),
        ("role", json!({})),
    ];
    for (verb, body) in reads {
        let (status, env) = b.call(verb, body).await;
        assert!(status == 200 || status == 404, "{verb}: {status} {env}");
    }
    assert_eq!(b.get("/api/sync/role").await.0, 200);
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    watcher.abort();
}
