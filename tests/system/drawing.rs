//! Drawing a picture in a plain chat (doc/tool-spec.md GenerateImage): on
//! the fixture's ChatGPT sign-in the tool is offered, the picture comes
//! from ChatGPT's image generation (a loopback stand-in here), is kept with
//! the session under a number the answer shows, and dies with the session.
//! A refusal is the tool's result in one line — nothing drawn elsewhere.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use axum::{
    extract::State, http::header, http::HeaderMap, http::StatusCode, response::IntoResponse,
    routing::post, Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn png() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(64, 64, image::Rgb([120, 60, 30]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

/// What the stand-in answers.
#[derive(Clone, Copy)]
enum Answer {
    Picture,
    Limit,
}

/// ChatGPT's `/responses` with the `image_generation` tool, on loopback;
/// it keeps each request's body and auth header.
struct ImageApi {
    url: String,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
}

type Seen = Arc<Mutex<Vec<(String, Value)>>>;

async fn respond(
    State((answer, seen)): State<(Answer, Seen)>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    seen.lock()
        .unwrap()
        .push((auth, serde_json::from_str(&body).unwrap_or(Value::Null)));
    if let Answer::Limit = answer {
        let body = json!({"error": {"type": "usage_limit_reached", "message": "The usage limit has been reached"}});
        return (StatusCode::TOO_MANY_REQUESTS, body.to_string()).into_response();
    }
    use base64::Engine as _;
    let item = json!({"type": "response.output_item.done", "item": {
        "type": "image_generation_call", "status": "completed",
        "result": base64::engine::general_purpose::STANDARD.encode(png())}});
    let sse = format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\n",
        json!({"type": "response.created"}),
        item,
        json!({"type": "response.completed", "response": {}})
    );
    ([(header::CONTENT_TYPE, "text/event-stream")], sse).into_response()
}

impl ImageApi {
    async fn start(answer: Answer) -> Self {
        let seen: Seen = Arc::default();
        let app = Router::new()
            .route("/responses", post(respond))
            .with_state((answer, seen.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind image api");
        let url = format!("http://{}", listener.local_addr().expect("image api addr"));
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { url, seen }
    }

    fn requests(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

const PROMPT: &str = "A coffee machine, flat illustration";

async fn world(
    answer: Answer,
    replies: Vec<crate::support::model::Reply>,
) -> (World, ImageApi, String) {
    let api = ImageApi::start(answer).await;
    let setup = Setup {
        image_api: Some(api.url.clone()),
        ..Setup::default()
    };
    let w = World::start(setup.script(Agent::Ling, replies)).await;
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "draw", "project_root": root}))
        .await;
    w.turn(&sid, &root, "画一个咖啡机", Agent::Ling).await;
    (w, api, sid)
}

#[tokio::test]
async fn a_plain_chat_draws_and_shows_the_picture_by_number() {
    let (w, api, sid) = world(
        Answer::Picture,
        vec![
            tool("GenerateImage", json!({"prompt": PROMPT})),
            text("Here it is [turn1image1]."),
        ],
    )
    .await;

    let first = &w.model.calls_of(Agent::Ling)[0];
    assert!(
        first.tool_names().contains(&"GenerateImage".to_string()),
        "not offered"
    );

    // One request to ChatGPT, on the sign-in, asking for one picture.
    let requests = api.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let (auth, body) = &requests[0];
    assert_eq!(auth, "Bearer test-access-not-a-token");
    assert_eq!(body["tools"][0]["type"], "image_generation");
    assert_eq!(body["model"], "gpt-6-luna");

    // The model read a number and how to show it — no bytes, no address.
    let read = w.model.calls_of(Agent::Ling)[1].text();
    assert!(read.contains("drawn by ChatGPT"), "{read}");
    assert!(read.contains(&format!("[turn1image1] {PROMPT}")), "{read}");
    for wire in ["data:image", "/api/sessions/images", "iVBOR"] {
        assert!(!read.contains(wire), "the model read {wire}");
    }

    // The chat resolves the number to the session's kept copy.
    let history = w.api.history(&sid, &w.work()).await;
    let src = history["refs"]["turn1image1"]["src"]
        .as_str()
        .expect("src")
        .to_string();
    let bytes = reqwest::get(format!("{}{src}", w.engine.base_url()))
        .await
        .expect("get picture")
        .bytes()
        .await
        .expect("bytes");
    assert_eq!(
        bytes.as_ref(),
        png().as_slice(),
        "the drawn picture, as drawn"
    );

    // Gone with the session.
    let dir = w.home.linggen_home().join("sessions").join(&sid);
    assert!(dir.join("images").exists());
    w.api
        .delete("/api/sessions/all", json!({"session_id": sid}))
        .await;
    assert!(!dir.exists(), "the drawn picture outlived its session");
    w.assert_all_scripted();
}

#[tokio::test]
async fn a_usage_limit_is_said_plainly_and_nothing_is_drawn_elsewhere() {
    let (w, api, sid) = world(
        Answer::Limit,
        vec![
            tool("GenerateImage", json!({"prompt": PROMPT})),
            text("ChatGPT will not draw right now."),
        ],
    )
    .await;

    assert_eq!(api.requests().len(), 1);
    let read = w.model.calls_of(Agent::Ling)[1].text();
    assert!(
        read.contains("ChatGPT's usage limit is reached (429): The usage limit has been reached"),
        "{read}"
    );
    let history = w.api.history(&sid, &w.work()).await;
    let refs = history["refs"].as_object().map(|r| r.len()).unwrap_or(0);
    assert_eq!(refs, 0, "a refused picture got a number: {history}");
    let images = w
        .home
        .linggen_home()
        .join("sessions")
        .join(&sid)
        .join("images");
    assert!(!images.exists(), "something was drawn anyway");
    w.assert_all_scripted();
}
