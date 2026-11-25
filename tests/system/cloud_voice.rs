//! Linggen Cloud's voice uses (live lessons and read-aloud): the engine asks a
//! stand-in for linggen.dev (`LINGGEN_SITE_URL`, loopback; no request leaves
//! this machine) with the account key, never a provider. Who pays is decided
//! in one place: the person's own key first, then Linggen Cloud for a subscriber.

use crate::support::{Setup, World};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const ACCOUNT_KEY: &str = "usr-test-account-key-not-real";
const SHORT_KEY: &str = "st-FAKE-cloud-short-key-not-real";

#[derive(Clone, Copy, PartialEq)]
enum Plan {
    Subscriber,
    Free,
}

#[derive(Clone, Copy)]
enum Verdict {
    Allow,
    /// The site's refusal: status and fixed code.
    Refuse(u16, &'static str),
}

struct Site {
    url: String,
    /// (path, bearer, body) of every request that is not the entitlement read.
    seen: Arc<Mutex<Vec<(String, String, Value)>>>,
}

#[derive(Clone)]
struct SiteState {
    plan: Plan,
    verdict: Verdict,
    seen: Arc<Mutex<Vec<(String, String, Value)>>>,
}

fn wav() -> Vec<u8> {
    // 24 kHz mono 16-bit, 0.1 s of a quiet tone: a clip the engine's joiner can read.
    let samples: Vec<i16> = (0..2400)
        .map(|i| ((i as f32 / 20.0).sin() * 3000.0) as i16)
        .collect();
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = b"RIFF".to_vec();
    out.extend((36 + data.len() as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(24_000u32.to_le_bytes());
    out.extend(48_000u32.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

async fn entitlement(State(s): State<SiteState>) -> axum::response::Response {
    let apps = if s.plan == Plan::Subscriber {
        json!({"suite": {"status": "active", "current_period_end": null}})
    } else {
        json!({})
    };
    axum::Json(json!({"developer": false, "apps": apps, "trial": {}, "usage": {}})).into_response()
}

async fn live_token(
    State(s): State<SiteState>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    s.seen.lock().unwrap().push((
        "/api/live/token".into(),
        bearer(&headers),
        serde_json::from_str(&body).unwrap_or(Value::Null),
    ));
    match s.verdict {
        Verdict::Refuse(code, why) => (StatusCode::from_u16(code).unwrap(), axum::Json(json!({"error": why}))).into_response(),
        Verdict::Allow => axum::Json(json!({
            "provider": "alibaba",
            "model": "qwen-audio-3.1-realtime-flash",
            "ws_url": format!("wss://ap-southeast-1.maas.qwencloudapi.com/api-ws/v1/realtime?model=qwen-audio-3.1-realtime-flash&api_key={SHORT_KEY}"),
            "token": SHORT_KEY,
            "expires": "2027-01-15T08:00:00Z",
            "max_minutes": 20,
            "lessons_left": 11,
        }))
        .into_response(),
    }
}

async fn tts(
    State(s): State<SiteState>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    s.seen.lock().unwrap().push((
        "/api/tts".into(),
        bearer(&headers),
        serde_json::from_str(&body).unwrap_or(Value::Null),
    ));
    match s.verdict {
        Verdict::Refuse(code, why) => (
            StatusCode::from_u16(code).unwrap(),
            axum::Json(json!({"error": why})),
        )
            .into_response(),
        Verdict::Allow => ([("content-type", "audio/wav")], wav()).into_response(),
    }
}

impl Site {
    async fn start(plan: Plan, verdict: Verdict) -> Self {
        let seen: Arc<Mutex<Vec<(String, String, Value)>>> = Arc::default();
        let state = SiteState {
            plan,
            verdict,
            seen: seen.clone(),
        };
        let app = Router::new()
            .route("/api/entitlement", get(entitlement))
            .route("/api/live/token", post(live_token))
            .route("/api/tts", post(tts))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind site");
        let url = format!("http://{}", listener.local_addr().expect("site addr"));
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { url, seen }
    }

    fn requests(&self) -> Vec<(String, String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

/// An engine that talks to `site`, signed in with the account key, holding no Qwen key of its own.
async fn world(site: &Site, signed_in: bool) -> World {
    let w = World::start(Setup {
        env: vec![("LINGGEN_SITE_URL", site.url.clone())],
        ..Setup::default()
    })
    .await;
    if signed_in {
        std::fs::write(
            w.home.linggen_home().join("account.toml"),
            format!("api_token = \"{ACCOUNT_KEY}\"\n"),
        )
        .expect("sign in");
    }
    w
}

#[tokio::test]
async fn a_subscriber_without_a_key_of_their_own_is_given_a_lesson_key_by_linggen_cloud() {
    let site = Site::start(Plan::Subscriber, Verdict::Allow).await;
    let w = world(&site, true).await;

    let listed = w.api.get("/api/live").await;
    assert_eq!(
        listed,
        json!({"providers": [{"id": "alibaba", "ready": true, "via": "linggen_cloud"}]}),
        "{listed}"
    );

    let (status, got) = w
        .api
        .post_with_status(
            "/api/live/token",
            json!({"provider": "alibaba", "setup": {"systemInstruction": "x"}}),
        )
        .await;
    assert_eq!(status, 200, "{got}");
    assert_eq!(got["provider"], "alibaba");
    assert_eq!(got["token"], SHORT_KEY);
    assert_eq!(
        (got["max_minutes"].as_u64(), got["lessons_left"].as_u64()),
        (Some(20), Some(11)),
        "{got}"
    );
    assert!(
        got["ws_url"]
            .as_str()
            .is_some_and(|u| u.starts_with("wss://") && u.contains(SHORT_KEY)),
        "{got}"
    );

    // The site was asked with the account key and the service's name, and nothing of the page's setup.
    let asked: Vec<_> = site
        .requests()
        .into_iter()
        .filter(|(p, ..)| p == "/api/live/token")
        .collect();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].1, format!("Bearer {ACCOUNT_KEY}"));
    assert_eq!(asked[0].2, json!({"provider": "alibaba"}));

    // Neither key is in the log, and the log is being read.
    let log = w.engine.log();
    assert!(
        log.contains("live token made for alibaba (linggen_cloud)"),
        "{log}"
    );
    for secret in [ACCOUNT_KEY, SHORT_KEY, "api_key="] {
        assert!(!log.contains(secret), "the log holds `{secret}`");
    }
}

#[tokio::test]
async fn a_person_who_is_signed_out_or_not_subscribed_gets_no_lesson_from_linggen_cloud() {
    for (plan, signed_in) in [(Plan::Subscriber, false), (Plan::Free, true)] {
        let site = Site::start(plan, Verdict::Allow).await;
        let w = world(&site, signed_in).await;
        let listed = w.api.get("/api/live").await;
        assert_eq!(
            listed,
            json!({"providers": [{"id": "alibaba", "ready": false, "why": "no_key"}]}),
            "{listed}"
        );
        let (status, got) = w
            .api
            .post_with_status("/api/live/token", json!({"provider": "alibaba"}))
            .await;
        assert_eq!(
            (status, got["error"].as_str()),
            (409, Some("no_key")),
            "{got}"
        );
        assert!(
            site.requests().is_empty(),
            "the site was asked for a key this person may not have"
        );
    }
}

#[tokio::test]
async fn the_sites_refusals_reach_the_page_as_fixed_words() {
    for (code, why, want, status) in [
        (402, "not_subscribed", "not_subscribed", 402),
        (429, "lessons_used", "lessons_used", 429),
        (429, "daily_cap", "daily_cap", 429),
        (503, "not_configured", "provider_down", 502),
    ] {
        let site = Site::start(Plan::Subscriber, Verdict::Refuse(code, why)).await;
        let w = world(&site, true).await;
        let (got_status, got) = w
            .api
            .post_with_status("/api/live/token", json!({"provider": "alibaba"}))
            .await;
        assert_eq!(
            (got_status, got["error"].as_str()),
            (status, Some(want)),
            "{why}: {got}"
        );
    }
}

#[tokio::test]
async fn a_subscribers_default_voice_is_linggen_clouds_serena() {
    let site = Site::start(Plan::Subscriber, Verdict::Allow).await;
    let w = world(&site, true).await;

    let uses = w.api.get("/api/uses").await;
    let cloud = uses["speak"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "linggen-cloud")
        .expect("the picker lists it");
    assert_eq!(cloud["name"], "Serena");
    assert_eq!(
        cloud["source"],
        json!({"way": "account", "provider": "Linggen Cloud"})
    );
    assert_eq!(cloud["state"], "ready");
    assert_eq!(uses["speak"]["effective"], "linggen-cloud");
    assert_eq!(w.api.get("/api/tts").await["natural"], true);

    // One line is one request, the site is told the text alone, and the clip comes back as the site made it.
    let (status, clip) = w
        .api
        .post_for_bytes(
            "/api/tts",
            json!({"text": "Hello there, I am here.", "voice": "vivian"}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(clip, wav());
    let asked: Vec<_> = site
        .requests()
        .into_iter()
        .filter(|(p, ..)| p == "/api/tts")
        .collect();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].1, format!("Bearer {ACCOUNT_KEY}"));
    assert_eq!(
        asked[0].2,
        json!({"text": "Hello there, I am here."}),
        "the voice is the site's, not the caller's"
    );

    // The person's own pick still wins.
    let (code, answer) = w
        .api
        .put_json("/api/uses", json!({"use": "speak", "choice": "system"}))
        .await;
    assert_eq!(code, 200, "{answer}");
    assert_eq!(answer["speak"]["effective"], "system");
}

#[tokio::test]
async fn read_aloud_is_not_offered_to_a_person_who_is_not_a_subscriber() {
    for (plan, signed_in, state) in [
        (Plan::Subscriber, false, "signed_out"),
        (Plan::Free, true, "unavailable"),
    ] {
        let site = Site::start(plan, Verdict::Allow).await;
        let w = world(&site, signed_in).await;
        let uses = w.api.get("/api/uses").await;
        let cloud = uses["speak"]["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == "linggen-cloud")
            .unwrap();
        assert_eq!(cloud["state"], state);
        assert_ne!(uses["speak"]["effective"], "linggen-cloud");
        let (code, _) = w
            .api
            .put_json(
                "/api/uses",
                json!({"use": "speak", "choice": "linggen-cloud"}),
            )
            .await;
        assert_eq!(code, 400, "a voice that is not ready cannot be chosen");
        assert!(
            site.requests().iter().all(|(p, ..)| p != "/api/tts"),
            "the site was asked to read to a non-subscriber"
        );
    }
}

/// The site says today's read-aloud is used up: the line is asked once, the next voice (this Mac's)
/// takes over at once, and no clause after it goes to the site again.
#[tokio::test]
async fn a_refusal_hands_back_to_this_mac_and_the_site_is_not_asked_again() {
    let site = Site::start(Plan::Subscriber, Verdict::Refuse(429, "daily_cap")).await;
    let w = world(&site, true).await;
    assert_eq!(
        w.api.get("/api/uses").await["speak"]["effective"],
        "linggen-cloud"
    );

    // Whether this machine can then speak (a `say`, a local model) is its own fact; the site is the point.
    let _ = w
        .api
        .post_for_bytes("/api/tts", json!({"text": "Hello there, I am here."}))
        .await;
    let asked = |site: &Site| {
        site.requests()
            .iter()
            .filter(|(p, ..)| p == "/api/tts")
            .count()
    };
    assert_eq!(asked(&site), 1, "the line was asked of the site once");

    let _ = w
        .api
        .post_for_bytes("/api/tts", json!({"text": "And another line."}))
        .await;
    assert_eq!(
        asked(&site),
        1,
        "a refused voice is not asked again for a while"
    );
    let uses = w.api.get("/api/uses").await;
    assert_ne!(uses["speak"]["effective"], "linggen-cloud", "{uses}");
    let cloud = uses["speak"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "linggen-cloud")
        .unwrap();
    assert_eq!(cloud["state"], "unavailable");
}
