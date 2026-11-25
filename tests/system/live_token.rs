//! Live voice for an app's page (`GET /api/live`, `POST /api/live/token`,
//! doc/yinyue-spec.md § Live voice): the engine mints a token for
//! one live session and hands the page only that. Two providers are listed,
//! `alibaba` first, `google` second. Each is a loopback stand-in
//! (`LINGGEN_LIVE_GOOGLE_BASE`, `LINGGEN_LIVE_ALIBABA_BASE`, honoured only by
//! a build with the `test-support` feature; no request leaves this machine).
//! The keys come from the scratch engines' knobs, `LINGGEN_LIVE_GOOGLE_KEY`
//! and `LINGGEN_LIVE_ALIBABA_KEY`.

use crate::support::{Setup, World};
use axum::{
    extract::State, http::HeaderMap, http::StatusCode, response::IntoResponse, routing::post,
    Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const KEY: &str = "AIza-test-key-not-real-0001";
const TOKEN: &str = "auth_tokens/FAKE-token-not-real-0002";
const QWEN_KEY: &str = "sk-ws-test-key-not-real-0003";
const QWEN_TOKEN: &str = "st-FAKE-short-key-not-real-0004";
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

/// Qwen's `POST /api/v1/tokens?expire_in_seconds=60`, on loopback: it keeps
/// each request's authorization header, query and body text; a failure's body
/// repeats the key and a marker, as a provider's error text might.
struct QwenApi {
    url: String,
    seen: Arc<Mutex<Vec<(String, String, String)>>>,
}

async fn qwen_respond(
    State((answer, seen)): State<(Answer, Arc<Mutex<Vec<(String, String, String)>>>)>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    seen.lock()
        .unwrap()
        .push((auth.clone(), uri.query().unwrap_or("").to_string(), body));
    match answer {
        // As the service answers (wire notes): the token and its end in unix seconds.
        Answer::Token => {
            axum::Json(json!({"token": QWEN_TOKEN, "expires_at": 1_800_000_000})).into_response()
        }
        Answer::Status(code) => {
            let text = json!({"code": "InvalidApiKey", "message": format!("{PROVIDER_BODY} {auth}")})
                .to_string();
            (StatusCode::from_u16(code).unwrap(), text).into_response()
        }
    }
}

impl QwenApi {
    async fn start(answer: Answer) -> Self {
        let seen = Arc::default();
        let app = Router::new()
            .route("/api/v1/tokens", post(qwen_respond))
            .with_state((answer, Arc::clone(&seen)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind qwen token api");
        let url = format!("http://{}", listener.local_addr().expect("qwen api addr"));
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { url, seen }
    }

    /// Each request as (authorization header, query, body text).
    fn requests(&self) -> Vec<(String, String, String)> {
        self.seen.lock().unwrap().clone()
    }
}

/// An engine with these environment variables, set last; `lan` binds it to
/// every address for the phone tests.
async fn world_env(mut env: Vec<(&'static str, String)>, lan: bool) -> World {
    // Nothing stored on this machine may stand in for a knob.
    env.push(("LINGGEN_API_KEY_GEMINI", String::new()));
    World::start(Setup {
        env,
        lan,
        ..Setup::default()
    })
    .await
}

/// An engine whose Google key is the knob's (`with_key`) and whose token
/// endpoint is `api`; it holds no Qwen key.
async fn world(api: &TokenApi, with_key: bool, lan: bool) -> World {
    let mut env = vec![("LINGGEN_LIVE_GOOGLE_BASE", api.url.clone())];
    if with_key {
        env.push(("LINGGEN_LIVE_GOOGLE_KEY", KEY.to_string()));
    }
    world_env(env, lan).await
}

/// An engine whose Qwen key is the knob's (`with_key`), whose Qwen endpoint is
/// `api`, and which holds no Google key.
async fn qwen_world(api: &QwenApi, with_key: bool) -> World {
    let mut env = vec![("LINGGEN_LIVE_ALIBABA_BASE", api.url.clone())];
    if with_key {
        env.push(("LINGGEN_LIVE_ALIBABA_KEY", QWEN_KEY.to_string()));
    }
    world_env(env, false).await
}

/// Neither Qwen's key nor its short key, nor a part the provider said, is in
/// the log — and the log is being read.
fn assert_qwen_log_is_clean(w: &World, what: &str) {
    let log = w.engine.log();
    assert!(
        log.contains("live token"),
        "the log shows no live line, so a clean log proves nothing ({what})"
    );
    for secret in [QWEN_KEY, QWEN_TOKEN, "st-FAKE", PROVIDER_BODY, "api_key="] {
        assert!(!log.contains(secret), "the log holds `{secret}` ({what})");
    }
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
        json!({"providers": [{"id": "alibaba", "ready": false, "why": "no_key"}]}),
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
        json!({"providers": [{"id": "alibaba", "ready": false, "why": "no_key"}]}),
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
            json!({"providers": [{"id": "alibaba", "ready": false, "why": "not_owner"}]}),
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

#[tokio::test]
async fn qwen_card_cleanup_keeps_the_model_key_and_the_live_authorization() {
    use linggen_engine::{config::Config, credentials::{self, Credentials}};
    const URL: &str = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1";
    const ROW: &str = r#"
[[models]]
id = "own-flash"
provider = "openai"
url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1"
model = "deepseek-v4.1-flash"
"#;
    const OTHER: &str = "unused-key-marker";
    let api = QwenApi::start(Answer::Token).await;
    let w = World::start(Setup {
        config_append: Some(ROW),
        env: vec![("LINGGEN_LIVE_ALIBABA_BASE", api.url.clone()), ("LINGGEN_LIVE_ALIBABA_KEY", String::new())],
        ..Setup::default()
    }).await;
    assert_eq!(w.api.put_json("/api/credentials", json!({"endpoints":[
        {"provider":"qwen", "url":URL, "api_key":OTHER},
        {"provider":"openai", "url":URL, "api_key":QWEN_KEY}
    ]})).await.0, 200);
    let config: Config = serde_json::from_value(w.api.get("/api/config").await).unwrap();
    let row = config.models.iter().find(|r| r.id == "own-flash").unwrap();
    let path = w.home.linggen_home().join("credentials.json");
    let live = || post_token(&w, json!({"provider":"alibaba"}));
    assert_eq!(live().await.0, 200);
    assert_eq!(credentials::resolve_api_key(row, &Credentials::load(&path)).as_deref(), Some(QWEN_KEY));
    let result = w.api.get("/api/providers").await;
    let groups: Vec<_> = result["providers"].as_array().unwrap().iter().filter(|p| p["label"] == "Qwen Cloud").collect();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["key"], json!({"set":true,"conflict":true,"can_keep_model_key":true}));
    assert_eq!(groups[0]["models"].as_array().unwrap().len(), 2);
    assert!(groups[0]["families"].is_object());
    let id = groups[0]["id"].as_str().unwrap();
    let key_route = format!("/api/providers/{id}/key");
    let action = json!({"action":"keep_model_key"});
    let (status, cleaned) = w.api.put_json(&key_route, action.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(cleaned["providers"].as_array().unwrap().iter().find(|p| p["id"] == id).unwrap()["key"], json!({"set":true}));
    for answer in [result, cleaned] {
        for marker in [QWEN_KEY, OTHER, "api_key"] { assert!(!answer.to_string().contains(marker)); }
    }
    let creds = Credentials::load(&path);
    assert!(creds.endpoint_key("qwen", URL).is_none());
    assert_eq!(credentials::resolve_api_key(row, &creds).as_deref(), Some(QWEN_KEY));
    assert_eq!(live().await.0, 200);
    assert_eq!(api.requests().iter().map(|(auth, _, _)| auth.clone()).collect::<Vec<_>>(), vec![format!("Bearer {QWEN_KEY}"); 2]);
    assert_eq!(w.api.put_json(&key_route, action).await.0, 409);
}

#[tokio::test]
async fn the_owner_is_given_a_qwen_short_key_address_and_never_the_long_lived_key() {
    let api = QwenApi::start(Answer::Token).await;
    let w = qwen_world(&api, true).await;

    let listed = w.api.get("/api/live").await;
    assert_eq!(
        listed,
        json!({"providers": [{"id": "alibaba", "ready": true, "via": "own_key"}]}),
        "{listed}"
    );
    assert!(
        api.requests().is_empty(),
        "asking who is ready made a request to the provider"
    );

    // The page's setup is checked and dropped: nothing of it is sent.
    let setup = json!({"systemInstruction": "You are a teacher.", "tools": [{"x": 1}]});
    let (status, got) = post_token(&w, json!({"provider": "alibaba", "setup": setup})).await;
    assert_eq!(status, 200, "{got}");
    assert_eq!(got["provider"], "alibaba");
    assert_eq!(got["model"], "qwen-audio-3.1-realtime-flash");
    assert_eq!(got["token"], QWEN_TOKEN);
    assert_eq!(
        got["ws_url"],
        format!("wss://ap-southeast-1.maas.qwencloudapi.com/api-ws/v1/realtime?model=qwen-audio-3.1-realtime-flash&api_key={QWEN_TOKEN}")
    );
    assert_eq!(got["expires"], "2027-01-15T08:00:00Z", "the service's own end");
    assert!(
        !got.to_string().contains(QWEN_KEY),
        "the page was handed the long-lived key: {got}"
    );

    // One request: bearer key, the lifetime in the query, no body.
    let seen = api.requests();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let (auth, query, body) = &seen[0];
    assert_eq!(auth, &format!("Bearer {QWEN_KEY}"));
    assert_eq!(query, "expire_in_seconds=60");
    assert_eq!(body, "", "no body is sent, whatever the page's setup holds");
    assert_qwen_log_is_clean(&w, "happy path");

    // A setup that is not an object is refused before anyone is called.
    let (status, got) = post_token(&w, json!({"provider": "alibaba", "setup": "x"})).await;
    assert_eq!((status, got), (400, json!({"error": "bad_setup"})));
    assert_eq!(api.requests().len(), 1);
}

#[tokio::test]
async fn the_providers_are_listed_alibaba_first_then_google() {
    let qwen = QwenApi::start(Answer::Token).await;
    let google = TokenApi::start(Answer::Token).await;
    let w = world_env(
        vec![
            ("LINGGEN_LIVE_ALIBABA_BASE", qwen.url.clone()),
            ("LINGGEN_LIVE_ALIBABA_KEY", QWEN_KEY.to_string()),
            ("LINGGEN_LIVE_GOOGLE_BASE", google.url.clone()),
            ("LINGGEN_LIVE_GOOGLE_KEY", KEY.to_string()),
        ],
        false,
    )
    .await;
    let listed = w.api.get("/api/live").await;
    assert_eq!(
        listed,
        json!({"providers": [{"id": "alibaba", "ready": true, "via": "own_key"}]}),
        "{listed}"
    );
    // Each provider's key goes to its own service alone.
    assert_eq!(post_token(&w, json!({"provider": "google"})).await.0, 200);
    assert_eq!(post_token(&w, json!({"provider": "alibaba"})).await.0, 200);
    assert_eq!(google.requests().len(), 1);
    assert_eq!(google.requests()[0].0, KEY);
    assert_eq!(qwen.requests().len(), 1);
    assert_eq!(qwen.requests()[0].0, format!("Bearer {QWEN_KEY}"));
}

#[tokio::test]
async fn a_mac_with_only_a_gemini_key_has_google_ready_and_alibaba_not() {
    let api = TokenApi::start(Answer::Token).await;
    let w = world(&api, true, false).await;
    let listed = w.api.get("/api/live").await;
    assert_eq!(
        listed,
        json!({"providers": [{"id": "alibaba", "ready": false, "why": "no_key"}]}),
        "{listed}"
    );
    let (status, got) = post_token(&w, json!({"provider": "alibaba"})).await;
    assert_eq!((status, got), (409, json!({"error": "no_key"})));
    assert!(api.requests().is_empty(), "the Gemini key went to a call for Qwen");
}

#[tokio::test]
async fn qwen_with_no_key_is_a_refusal_that_calls_nobody() {
    let api = QwenApi::start(Answer::Token).await;
    let w = qwen_world(&api, false).await;
    let (status, got) = post_token(&w, json!({"provider": "alibaba"})).await;
    assert_eq!((status, got), (409, json!({"error": "no_key"})));
    assert!(api.requests().is_empty(), "a request went out without a key");
    assert!(w
        .engine
        .log()
        .contains("live token refused for alibaba: no_key"));
}

#[tokio::test]
async fn a_qwen_failure_is_one_fixed_word_and_nothing_of_its_body() {
    for (code, word) in [
        (400, "request_refused"),
        (401, "key_refused"),
        (403, "key_refused"),
        (429, "provider_down"),
        (500, "provider_down"),
        (503, "provider_down"),
    ] {
        let api = QwenApi::start(Answer::Status(code)).await;
        let w = qwen_world(&api, true).await;
        let (got_status, got) = post_token(&w, json!({"provider": "alibaba"})).await;
        assert_eq!(
            (got_status, got.clone()),
            (502, json!({"error": word})),
            "provider answered {code}"
        );
        let text = got.to_string();
        for leak in [QWEN_KEY, PROVIDER_BODY, QWEN_TOKEN] {
            assert!(
                !text.contains(leak),
                "the reply holds `{leak}` after a {code}: {text}"
            );
        }
        assert_qwen_log_is_clean(&w, &format!("provider {code}"));
    }
}

#[tokio::test]
async fn a_phone_that_is_not_the_owner_cannot_ask_for_a_qwen_short_key() {
    let api = QwenApi::start(Answer::Token).await;
    let w = world_env(
        vec![
            ("LINGGEN_LIVE_ALIBABA_BASE", api.url.clone()),
            ("LINGGEN_LIVE_ALIBABA_KEY", QWEN_KEY.to_string()),
        ],
        true,
    )
    .await;
    let (client, base) = friends_phone(&w, json!({})).await;
    let refused = client
        .post(format!("{base}/api/live/token"))
        .header("x-linggen-device", "tok-friend-phone")
        .json(&json!({"provider": "alibaba"}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status().as_u16(), 403);
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["error"], "beyond_level", "{body}");
    assert!(
        api.requests().is_empty(),
        "a request went out for a caller that is not the owner"
    );
    assert_eq!(post_token(&w, json!({"provider": "alibaba"})).await.0, 200);
}
