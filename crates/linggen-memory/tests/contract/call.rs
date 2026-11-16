//! `Memory::call` on the handle, the way an in-process caller uses it: the
//! same verbs, the same replica behaviour as the routes, no HTTP on this
//! side.

use crate::support::*;
use ling_mem::{MemError, Reply, Who};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

async fn replica_of(primary: &str) -> Mem {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("memory")).unwrap();
    let file = json!({"primary": primary, "token": "tok-b", "device_id": "dev-b-0001"});
    std::fs::write(dir.path().join("memory/.replica.json"), file.to_string()).unwrap();
    Mem::open(dir, "").await
}

fn outbox(m: &Mem) -> Value {
    std::fs::read_to_string(m.data_dir().join("memory/.outbox.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(json!([]))
}

fn refused(e: MemError) -> (&'static str, String) {
    (e.code.code(), e.message)
}

async fn call(m: &Mem, verb: &str, args: Value) -> Result<Reply, MemError> {
    m.state.call(&Who::Owner, verb, args).await
}

#[tokio::test]
async fn a_write_on_a_primary_runs_here() {
    let m = Mem::start().await;
    let reply = call(
        &m,
        "add",
        json!({"content": "Alex keeps bees", "tier": "semantic"}),
    )
    .await
    .unwrap();
    let Reply::Done(data) = reply else {
        panic!("not done here: {reply:?}");
    };
    assert_eq!(data["action"], "added");
    let got = m.ok("get", json!({"id": data["fact"]["id"]})).await;
    assert_eq!(got["content"], "Alex keeps bees");

    let count = call(&m, "count", json!({})).await.unwrap();
    assert_eq!(count.into_result().unwrap()["count"], 1);
}

#[tokio::test]
async fn args_the_verb_cannot_read_are_a_bad_request() {
    let m = Mem::start().await;
    let e = call(&m, "add", json!({"contents": "x"})).await.unwrap_err();
    assert_eq!(
        refused(e),
        ("BAD_REQUEST", "add: missing field `content`".into())
    );
    assert_eq!(m.ok("count", json!({})).await["count"], 0);
}

/// Online: the primary's answer, as it came; then the replica holds the row.
#[tokio::test]
async fn a_replicas_write_is_the_primarys_and_is_pulled() {
    let a = Mem::start_under("/mem").await;
    let b = replica_of(&a.addr().to_string()).await;
    let reply = call(
        &b,
        "add",
        json!({"content": "Alex sells honey", "tier": "semantic"}),
    )
    .await
    .unwrap();
    let Reply::Primary { status, body } = reply.clone() else {
        panic!("not the primary's: {reply:?}");
    };
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    let data = reply.into_result().unwrap();
    assert_eq!(data, body["data"]);
    let id = data["fact"]["id"].clone();
    assert_eq!(
        a.ok("get", json!({"id": id})).await["content"],
        "Alex sells honey"
    );
    let here = call(&b, "get", json!({"id": id})).await.unwrap();
    assert_eq!(here.into_result().unwrap()["content"], "Alex sells honey");
    assert_eq!(outbox(&b), json!([]));
}

/// The primary's refusal reaches an in-process caller as a `MemError` with
/// the primary's code and words; nothing is applied or queued.
#[tokio::test]
async fn the_primarys_refusal_is_a_mem_error() {
    let a = Mem::start_under("/mem").await;
    let b = replica_of(&a.addr().to_string()).await;
    let voice = a
        .add(json!({"content": "Alex prefers tea", "from": "user", "tier": "semantic"}))
        .await;
    let replace = json!({"content": "Alex now prefers coffee", "replace_ids": [voice["id"]]});
    let (status, direct) = a.call("add", replace.clone()).await;
    assert_eq!(status, 400);

    let reply = call(&b, "add", replace).await.unwrap();
    assert_eq!(
        reply,
        Reply::Primary {
            status: 400,
            body: direct.clone()
        }
    );
    let e = reply.into_result().unwrap_err();
    assert_eq!(
        refused(e),
        ("BAD_REQUEST", direct["error"].as_str().unwrap().to_string())
    );
    assert_eq!(outbox(&b), json!([]));
    assert_eq!(b.ok("count", json!({})).await["count"], 0);
}

/// Offline: applied here and queued — the same outbox entry the route
/// writes for the same body.
#[tokio::test]
async fn an_offline_write_is_queued_as_the_route_queues_it() {
    let by_call = replica_of("127.0.0.1:1").await;
    let by_route = replica_of("127.0.0.1:1").await;
    let body = json!({"content": "Alex keeps bees", "tier": "semantic", "mood": "sunny"});

    let reply = call(&by_call, "add", body.clone()).await.unwrap();
    let Reply::Queued(data) = reply else {
        panic!("not queued: {reply:?}");
    };
    let (status, headers, env) = by_route
        .post_with_headers("/api/memory/add", body.clone())
        .await;
    assert_eq!(status, 200, "{env}");
    assert_eq!(headers.get("x-ling-mem-queued").unwrap(), "1");

    let entry = |m: &Mem, id: &Value| {
        let mut e = outbox(m)[0].clone();
        assert_eq!(outbox(m).as_array().unwrap().len(), 1);
        assert!(e.as_object_mut().unwrap().remove("created_at").is_some());
        assert_eq!(e["local_ids"], json!([id]));
        e["local_ids"] = json!(["<local>"]);
        e
    };
    let queued = entry(&by_call, &data["fact"]["id"]);
    assert_eq!(queued, entry(&by_route, &env["data"]["fact"]["id"]));
    assert_eq!(
        queued,
        json!({"seq": 1, "verb": "/api/memory/add", "args": body, "replica_id": "dev-b-0001", "local_ids": ["<local>"]})
    );
    let here = call(&by_call, "get", json!({"id": data["fact"]["id"]})).await;
    assert_eq!(
        here.unwrap().into_result().unwrap()["content"],
        "Alex keeps bees"
    );
}

#[tokio::test]
async fn a_primary_only_verb_is_refused_on_a_replica() {
    let b = replica_of("127.0.0.1:1").await;
    for (verb, args) in [
        ("sweep", json!({})),
        ("remember_day", json!({"date": days_ago(1)})),
        ("dream_lease", json!("not even an object")),
        ("apply_schema", json!({"confirm": true})),
    ] {
        let e = call(&b, verb, args).await.unwrap_err();
        assert_eq!(
            refused(e),
            (
                "REPLICA",
                "Memory is dreamed on the primary (127.0.0.1:1)".into()
            ),
            "{verb}"
        );
    }
    assert_eq!(outbox(&b), json!([]));
}

#[tokio::test]
async fn an_unknown_verb_is_not_found() {
    let m = Mem::start().await;
    let e = call(&m, "wat", json!({})).await.unwrap_err();
    assert_eq!(refused(e), ("NOT_FOUND", "no memory verb wat".into()));
}

#[tokio::test]
async fn a_call_stamps_the_activity_clock() {
    let m = Mem::start().await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(m.state.idle_for() >= Duration::from_millis(60));
    let sent = Instant::now();
    call(&m, "count", json!({})).await.unwrap();
    let idle = m.state.idle_for();
    assert!(
        idle <= sent.elapsed() + Duration::from_millis(1),
        "{idle:?}"
    );
    // An unknown verb stamps it too, as a request to an unknown route does.
    tokio::time::sleep(Duration::from_millis(60)).await;
    let sent = Instant::now();
    let _ = call(&m, "wat", json!({})).await;
    assert!(m.state.idle_for() <= sent.elapsed() + Duration::from_millis(1));
}
