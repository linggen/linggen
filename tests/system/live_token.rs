//! Live voice for an app's page (`GET /api/live`, `POST /api/live/token`,
//! doc/yinyue-companion-spec.md § Live voice): the engine mints a token for
//! one live session and hands the page only that. The provider is a loopback
//! stand-in (`LINGGEN_LIVE_GOOGLE_BASE`, honoured only by a build with the
//! `test-support` feature; no request leaves this machine). The key comes
//! from the scratch engines' knob, `LINGGEN_LIVE_GOOGLE_KEY`.

use crate::support::{Setup, World};
use axum::{
    extract::State, http::HeaderMap, http::StatusCode, response::IntoResponse, routing::post,
    Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const KEY: &str = "AIza-test-key-not-real-0001";
const TOKEN: &str = "auth_tokens/FAKE-token-not-real-0002";
const PROVIDER_BODY: &str = "PROVIDER-BODY-MUST-NOT-REACH-THE-PAGE";

#[derive(Clone, Copy)]
enum Answer {
    Token,
    Status(u16),
}

type Seen = Arc<Mutex<Vec<(String, Value)>>>;

/// Google's `POST /v1beta/auth_tokens`, on loopback: it keeps each request's
/// key header and body; a failure's body repeats the key and a marker, as a
/// provider's error text might.
struct TokenApi {
    url: String,
    seen: Seen,
}

async fn respond(
    State((answer, seen)): State<(Answer, Seen)>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    let key = headers
        .get("x-goog-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    seen.lock().unwrap().push((
        key.clone(),
        serde_json::from_str(&body).unwrap_or(Value::Null),
    ));
    match answer {
        // As the real service answers (seen 2026-11-23): the token alone, no
        // expiry — the engine tells the page the lifetime it asked for.
        Answer::Token => axum::Json(json!({"name": TOKEN})).into_response(),
        Answer::Status(code) => {
            let text =
                json!({"error": {"message": format!("{PROVIDER_BODY} key={key}")}}).to_string();
            (StatusCode::from_u16(code).unwrap(), text).into_response()
        }
    }
}

impl TokenApi {
    async fn start(answer: Answer) -> Self {
        let seen: Seen = Arc::default();
        let app = Router::new()
            .route("/v1beta/auth_tokens", post(respond))
            .with_state((answer, seen.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind token api");
        let url = format!("http://{}", listener.local_addr().expect("token api addr"));
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { url, seen }
    }

    fn requests(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

/// An engine whose Google key is the knob's (`with_key`) and whose token
/// endpoint is `api`; `lan` binds it to every address for the phone tests.
async fn world(api: &TokenApi, with_key: bool, lan: bool) -> World {
    let mut env = vec![
        ("LINGGEN_LIVE_GOOGLE_BASE", api.url.clone()),
        // Nothing stored on this machine may stand in for the knob.
        ("LINGGEN_API_KEY_GEMINI", String::new()),
    ];
    if with_key {
        env.push(("LINGGEN_LIVE_GOOGLE_KEY", KEY.to_string()));
    }
    World::start(Setup {
        env,
        lan,
        ..Setup::default()
    })
    .await
}

/// Neither the key nor the token, nor a part the provider said, is in the log
/// — and the log is being read (the engine said it made a token or refused).
fn assert_log_is_clean(w: &World, what: &str) {
    let log = w.engine.log();
    assert!(
        log.contains("live token"),
        "the log shows no live line, so a clean log proves nothing ({what})"
    );
    for secret in [KEY, TOKEN, "FAKE-token", PROVIDER_BODY, "access_token="] {
        assert!(!log.contains(secret), "the log holds `{secret}` ({what})");
    }
}

async fn post_token(w: &World, body: Value) -> (u16, Value) {
    w.api.post_with_status("/api/live/token", body).await
}

#[tokio::test]
async fn the_owner_is_given_a_token_for_one_session_and_never_the_key() {
    let api = TokenApi::start(Answer::Token).await;
    let w = world(&api, true, false).await;

    let listed = w.api.get("/api/live").await;
    assert_eq!(
        listed,
        json!({"providers": [{"id": "google", "ready": true}]}),
        "{listed}"
    );
    assert!(
        api.requests().is_empty(),
        "asking who is ready made a request to the provider"
    );

    let (status, got) = post_token(&w, json!({"provider": "google"})).await;
    assert_eq!(status, 200, "{got}");
    assert_eq!(got["provider"], "google");
    assert_eq!(got["model"], "gemini-3.8-live");
    assert_eq!(got["token"], TOKEN);
    assert_eq!(
        got["ws_url"],
        format!("wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContentConstrained?access_token={TOKEN}")
    );
    assert!(
        got["expires"].as_str().is_some_and(|e| e.ends_with('Z')),
        "{got}"
    );
    assert!(
        !got.to_string().contains(KEY),
        "the page was handed the key: {got}"
    );

    // One request, with the key in the header, fixing model, audio and one use.
    let seen = api.requests();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let (key, body) = &seen[0];
    assert_eq!(key, KEY);
    assert_eq!(body["uses"], 1);
    assert_eq!(
        body["bidiGenerateContentSetup"]["model"],
        "models/gemini-3.8-live"
    );
    assert_eq!(
        body["bidiGenerateContentSetup"]["generationConfig"]["responseModalities"],
        json!(["AUDIO"])
    );
    assert!(body.get("liveConnectConstraints").is_none(), "{body}");
    assert!(
        body["expireTime"].is_string() && body["newSessionExpireTime"].is_string(),
        "{body}"
    );
    assert_log_is_clean(&w, "happy path");
}

#[tokio::test]
async fn a_page_cannot_choose_the_model_the_provider_or_the_address() {
    let api = TokenApi::start(Answer::Token).await;
    let w = world(&api, true, false).await;
    let (status, got) = post_token(&w, json!({"provider": "nobody"})).await;
    assert_eq!((status, got), (400, json!({"error": "unknown_provider"})));
    let (status, got) = post_token(&w, json!({})).await;
    assert_eq!((status, got), (400, json!({"error": "unknown_provider"})));
    let (status, _) = post_token(
        &w,
        json!({"provider": "google", "model": "other", "base": "http://127.0.0.1:1"}),
    )
    .await;
    assert_eq!(status, 200);
    let seen = api.requests();
    assert_eq!(
        seen.len(),
        1,
        "only the asked, declared provider is called: {seen:?}"
    );
    assert_eq!(
        seen[0].1["bidiGenerateContentSetup"]["model"],
        "models/gemini-3.8-live"
    );
}

#[tokio::test]
async fn no_key_is_not_ready_and_a_refusal_that_calls_nobody() {
    let api = TokenApi::start(Answer::Token).await;
    let w = world(&api, false, false).await;
    let listed = w.api.get("/api/live").await;
    assert_eq!(
        listed,
        json!({"providers": [{"id": "google", "ready": false, "why": "no_key"}]}),
        "{listed}"
    );
    let (status, got) = post_token(&w, json!({"provider": "google"})).await;
    assert_eq!((status, got), (409, json!({"error": "no_key"})));
    assert!(
        api.requests().is_empty(),
        "a request went out without a key"
    );
    assert!(w
        .engine
        .log()
        .contains("live token refused for google: no_key"));
}

#[tokio::test]
async fn a_provider_failure_is_one_fixed_word_and_nothing_of_its_body() {
    for (code, word, status) in [
        (400, "request_refused", 502),
        (401, "key_refused", 502),
        (403, "key_refused", 502),
        (500, "provider_down", 502),
        (503, "provider_down", 502),
    ] {
        let api = TokenApi::start(Answer::Status(code)).await;
        let w = world(&api, true, false).await;
        let (got_status, got) = post_token(&w, json!({"provider": "google"})).await;
        assert_eq!(
            (got_status, got.clone()),
            (status, json!({"error": word})),
            "provider answered {code}"
        );
        let text = got.to_string();
        for leak in [KEY, PROVIDER_BODY, TOKEN] {
            assert!(
                !text.contains(leak),
                "the reply holds `{leak}` after a {code}: {text}"
            );
        }
        assert_log_is_clean(&w, &format!("provider {code}"));
    }
}

#[tokio::test]
async fn a_page_in_a_loop_is_slowed_down() {
    let api = TokenApi::start(Answer::Token).await;
    let w = world(&api, true, false).await;
    let mut statuses = Vec::new();
    for _ in 0..8 {
        statuses.push(post_token(&w, json!({"provider": "google"})).await.0);
    }
    assert_eq!(
        statuses,
        vec![200, 200, 200, 200, 200, 429, 429, 429],
        "five a minute"
    );
    assert_eq!(
        api.requests().len(),
        5,
        "a refused call reached the provider"
    );
    let (_, got) = post_token(&w, json!({"provider": "google"})).await;
    assert_eq!(got, json!({"error": "slow_down"}));
}

/// A phone paired to this Mac and signed in as someone else, over the LAN.
async fn friends_phone(w: &World, extra: Value) -> (reqwest::Client, String) {
    let token = "tok-friend-phone";
    let mut row = json!({
        "id": "phone-friend", "name": "Friend's phone", "secret": token, "created_at": 0,
        "kind": "phone", "account": {"id": "u-friend", "name": "Friend"},
        "verified_account": "u-friend",
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        row[k] = v;
    }
    std::fs::write(
        w.home.linggen_home().join("paired-devices.json"),
        json!([row]).to_string(),
    )
    .unwrap();
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").expect("udp socket");
    socket
        .connect("192.0.2.1:9")
        .expect("this machine has no network address: the LAN path cannot be tested");
    let base = format!(
        "http://{}:{}",
        socket.local_addr().unwrap().ip(),
        w.engine.port
    );
    let client = reqwest::Client::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let known = client
            .get(format!("{base}/api/pair/me"))
            .header("x-linggen-device", token)
            .send()
            .await
            .unwrap();
        if known.status().as_u16() == 200 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the phone is not known"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    (client, base)
}

#[tokio::test]
async fn a_phone_that_is_not_the_owner_is_refused_and_told_why() {
    let api = TokenApi::start(Answer::Token).await;
    let w = world(&api, true, true).await;
    for extra in [json!({}), json!({"contract": 1})] {
        let (client, base) = friends_phone(&w, extra.clone()).await;
        let ask = |method: reqwest::Method, path: &str| {
            client
                .request(method, format!("{base}{path}"))
                .header("x-linggen-device", "tok-friend-phone")
                .json(&json!({"provider": "google"}))
        };
        let refused = ask(reqwest::Method::POST, "/api/live/token")
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status().as_u16(), 403, "{extra}");
        let body: Value = refused.json().await.unwrap();
        assert_eq!(body["error"], "beyond_level", "{body}");
        assert_eq!(body["needs"], "admin", "{body}");
        let listed: Value = ask(reqwest::Method::GET, "/api/live")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            listed,
            json!({"providers": [{"id": "google", "ready": false, "why": "not_owner"}]}),
            "{listed}"
        );
    }
    assert!(
        api.requests().is_empty(),
        "a request went out for a caller that is not the owner"
    );
    // This Mac's own page is the owner: the same engine answers it.
    assert_eq!(post_token(&w, json!({"provider": "google"})).await.0, 200);
}
