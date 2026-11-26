//! Settings → Memory → Settings: memory's two own knobs, how long a
//! short-term row is kept and how selective recall is. Memory owns them
//! (its `/api/config`); these routes call it in-process
//! (`memory_host::local_call`), because the page reaches only the engine's
//! `/api/*` over the data channel and memory's routes are not that.
//!
//! - `GET /api/ling-mem/config` — `{episodic_ttl_days, recall_min_score}`.
//! - `PUT /api/ling-mem/config` — the same two fields, either or both.
//!
//! The owner only (`Caller::is_owner`, the one rule `/api/memory/<verb>`
//! uses). Only those two fields cross: memory's `network` choice and any key
//! it may gain are neither shown nor accepted here — the body is rebuilt from
//! the two, never passed on.

use axum::{
    extract::ConnectInfo,
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Map, Value};
use std::future::Future;
use std::net::SocketAddr;

use crate::memory_host::caller::{self, Caller};

const PATH: &str = "/api/config";
const TTL_DAYS: &str = "episodic_ttl_days";
const MIN_SCORE: &str = "recall_min_score";
/// The range is memory's (`memory::expiry`), not repeated here.
fn ttl_ok(days: u64) -> bool {
    u32::try_from(days).is_ok_and(|d| ling_mem::memory::expiry::TTL_DAYS_RANGE.contains(&d))
}

/// One request to the hosted memory; `memory_host::local_call` in the engine,
/// a recorder in the tests.
trait MemoryCall: Fn(Method, &'static str, Option<Value>) -> Self::Fut {
    type Fut: Future<Output = (StatusCode, Value)>;
}
impl<F, Fut> MemoryCall for F
where
    F: Fn(Method, &'static str, Option<Value>) -> Fut,
    Fut: Future<Output = (StatusCode, Value)>,
{
    type Fut = Fut;
}

/// `GET /api/ling-mem/config`
pub async fn get_config(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let who = caller::resolve(peer.ip(), &headers);
    read(&who, &crate::memory_host::local_call).await
}

/// `PUT /api/ling-mem/config`
pub async fn put_config(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let who = caller::resolve(peer.ip(), &headers);
    write(&who, body, &crate::memory_host::local_call).await
}

async fn read(who: &Caller, call: &impl MemoryCall) -> Response {
    if let Err(refused) = owner_only(who) {
        return refused;
    }
    let (status, envelope) = call(Method::GET, PATH, None).await;
    answer(status, envelope)
}

async fn write(who: &Caller, body: Value, call: &impl MemoryCall) -> Response {
    if let Err(refused) = owner_only(who) {
        return refused;
    }
    let sent = match checked_fields(&body) {
        Ok(sent) => sent,
        Err(why) => return error(StatusCode::BAD_REQUEST, why, "BAD_REQUEST"),
    };
    // Memory takes the whole record, so a field left out keeps its stored value.
    let (status, current) = call(Method::GET, PATH, None).await;
    if !status.is_success() {
        return answer(status, current);
    }
    let mut merged = settings_of(&current["data"]);
    merged.extend(sent);
    let (status, envelope) = call(Method::PUT, PATH, Some(Value::Object(merged))).await;
    answer(status, envelope)
}

fn owner_only(who: &Caller) -> Result<(), Response> {
    if who.is_owner() {
        return Ok(());
    }
    Err(error(
        StatusCode::FORBIDDEN,
        "Only this Mac's owner can change these.".into(),
        "FORBIDDEN",
    ))
}

/// The two fields of a request, checked; everything else in the body is dropped.
fn checked_fields(body: &Value) -> Result<Map<String, Value>, String> {
    let obj = body.as_object().ok_or("Send a JSON object.")?;
    let mut sent = Map::new();
    if let Some(days) = obj.get(TTL_DAYS) {
        let days = days.as_u64().filter(|d| ttl_ok(*d));
        let days = days.ok_or("episodic_ttl_days must be a whole number from 1 to 365.")?;
        sent.insert(TTL_DAYS.into(), json!(days));
    }
    if let Some(score) = obj.get(MIN_SCORE) {
        let score = score.as_f64().filter(|s| (0.0..=1.0).contains(s));
        let score = score.ok_or("recall_min_score must be a number from 0 to 1.")?;
        sent.insert(MIN_SCORE.into(), json!(score));
    }
    if sent.is_empty() {
        return Err("Send episodic_ttl_days or recall_min_score.".into());
    }
    Ok(sent)
}

/// Memory's record cut to the two fields this door shows.
fn settings_of(record: &Value) -> Map<String, Value> {
    [TTL_DAYS, MIN_SCORE]
        .into_iter()
        .filter_map(|k| Some((k.to_string(), record.get(k)?.clone())))
        .map(|(k, v)| (k, as_typed(v)))
        .collect()
}

/// Memory keeps the threshold as an f32, which its JSON writes widened to
/// f64 (0.7 as 0.699999988079071). Shown as the f32 reads: 0.7.
fn as_typed(v: Value) -> Value {
    match v.as_f64().filter(|_| v.is_f64()) {
        Some(wide) => json!((wide as f32).to_string().parse::<f64>().unwrap_or(wide)),
        None => v,
    }
}

/// Memory's answer under its own status, with a success cut to the two fields.
fn answer(status: StatusCode, mut envelope: Value) -> Response {
    if status.is_success() {
        envelope["data"] = Value::Object(settings_of(&envelope["data"]));
    }
    (status, Json(envelope)).into_response()
}

fn error(status: StatusCode, message: String, code: &str) -> Response {
    (
        status,
        Json(json!({ "ok": false, "error": message, "code": code })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::api::pair::PairedDevice;
    use crate::state_fs::devices::DeviceKind;
    use std::sync::{Arc, Mutex};

    type Sent = Arc<Mutex<Vec<(Method, Option<Value>)>>>;

    /// A stand-in for memory's config door: it holds a record (with a
    /// `network` the real one also holds) and notes what it was sent.
    fn fake_memory() -> (impl MemoryCall, Sent) {
        let sent: Sent = Arc::default();
        let record = Arc::new(Mutex::new(json!({
            "episodic_ttl_days": 7, "recall_min_score": 0.7, "network": "lan"
        })));
        let log = sent.clone();
        let call = move |method: Method, path: &'static str, body: Option<Value>| {
            assert_eq!(path, PATH);
            log.lock().unwrap().push((method.clone(), body.clone()));
            let record = record.clone();
            async move {
                let mut held = record.lock().unwrap();
                if let Some(Value::Object(new)) = body.filter(|_| method == Method::PUT) {
                    held.as_object_mut().unwrap().extend(new);
                }
                (StatusCode::OK, json!({ "ok": true, "data": *held }))
            }
        };
        (call, sent)
    }

    fn machine(kind: DeviceKind) -> PairedDevice {
        PairedDevice {
            id: "dev-1".into(),
            name: "Alex's device".into(),
            secret: "s".into(),
            created_at: 0,
            kind: Some(kind),
            account: None,
            device_id: None,
            settings: serde_json::Map::new(),
            relay_grant: None,
            superseded_grant: None,
            ..Default::default()
        }
    }

    async fn body_of(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn the_owner_reads_the_two_settings_and_nothing_else() {
        let (call, _) = fake_memory();
        let (status, body) = body_of(read(&Caller::Owner, &call).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"],
            json!({ "episodic_ttl_days": 7, "recall_min_score": 0.7 })
        );
    }

    /// Memory's threshold is an f32 widened by its JSON: 0.7 arrives as
    /// 0.699999988079071 and must reach the page as 0.7.
    #[tokio::test]
    async fn a_widened_f32_is_shown_as_typed() {
        let call = |_: Method, _: &'static str, _: Option<Value>| async {
            let data =
                json!({ "episodic_ttl_days": 7, "recall_min_score": 0.7f32, "network": "lan" });
            (StatusCode::OK, json!({ "ok": true, "data": data }))
        };
        let (_, body) = body_of(read(&Caller::Owner, &call).await).await;
        assert_eq!(body["data"]["recall_min_score"], json!(0.7));
        assert_eq!(body["data"]["episodic_ttl_days"], json!(7));
    }

    #[tokio::test]
    async fn the_owner_writes_and_an_extra_key_is_not_forwarded() {
        let (call, sent) = fake_memory();
        let put =
            json!({ "episodic_ttl_days": 30, "recall_min_score": 0.5, "network": "local", "x": 1 });
        let (status, body) = body_of(write(&Caller::Owner, put, &call).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"],
            json!({ "episodic_ttl_days": 30, "recall_min_score": 0.5 })
        );
        let forwarded: Vec<_> = sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(m, b)| b.clone().filter(|_| *m == Method::PUT))
            .collect();
        assert_eq!(
            forwarded,
            vec![json!({ "episodic_ttl_days": 30, "recall_min_score": 0.5 })]
        );
    }

    #[tokio::test]
    async fn a_field_left_out_keeps_its_stored_value() {
        let (call, sent) = fake_memory();
        write(&Caller::Owner, json!({ "episodic_ttl_days": 14 }), &call).await;
        let put = sent
            .lock()
            .unwrap()
            .iter()
            .find(|(m, _)| *m == Method::PUT)
            .cloned()
            .unwrap();
        assert_eq!(
            put.1,
            Some(json!({ "episodic_ttl_days": 14, "recall_min_score": 0.7 }))
        );
    }

    /// The same rule as `/api/memory/<verb>`: `is_owner`. A phone and a
    /// caller memory does not know are refused, and memory is never asked.
    #[tokio::test]
    async fn a_phone_and_an_unknown_caller_get_403_and_memory_is_not_asked() {
        for who in [Caller::Phone(machine(DeviceKind::Phone)), Caller::Unknown] {
            let (call, sent) = fake_memory();
            let (status, body) = body_of(read(&who, &call).await).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who:?}");
            assert_eq!(body["code"], "FORBIDDEN");
            let put = json!({ "episodic_ttl_days": 30 });
            let (status, _) = body_of(write(&who, put, &call).await).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who:?}");
            assert!(sent.lock().unwrap().is_empty(), "{who:?}");
        }
    }

    /// An approved machine is the owner's other computer: `is_owner` says so,
    /// and this door follows it.
    #[tokio::test]
    async fn an_approved_machine_follows_the_owner_rule() {
        let (call, _) = fake_memory();
        let who = Caller::Machine(machine(DeviceKind::Machine));
        assert!(who.is_owner());
        let (status, _) = body_of(read(&who, &call).await).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_bad_value_is_a_400_with_one_line_and_memory_is_not_written() {
        let bad = [
            json!({ "episodic_ttl_days": 0 }),
            json!({ "episodic_ttl_days": 366 }),
            json!({ "episodic_ttl_days": 7.5 }),
            json!({ "episodic_ttl_days": -3 }),
            json!({ "episodic_ttl_days": "7" }),
            json!({ "recall_min_score": 1.5 }),
            json!({ "recall_min_score": -0.1 }),
            json!({ "recall_min_score": "high" }),
            json!({ "network": "local" }),
            json!({}),
            json!([1]),
        ];
        for put in bad {
            let (call, sent) = fake_memory();
            let (status, body) = body_of(write(&Caller::Owner, put.clone(), &call).await).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{put}");
            assert_eq!(body["ok"], false);
            assert!(!body["error"].as_str().unwrap().contains('\n'));
            assert!(sent.lock().unwrap().is_empty(), "{put}");
        }
    }

    #[tokio::test]
    async fn memory_unavailable_is_passed_on() {
        let call = |_: Method, _: &'static str, _: Option<Value>| async {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({ "ok": false, "error": "down" }),
            )
        };
        let (status, _) = body_of(read(&Caller::Owner, &call).await).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let (status, _) =
            body_of(write(&Caller::Owner, json!({ "episodic_ttl_days": 9 }), &call).await).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
