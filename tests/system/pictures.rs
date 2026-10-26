//! Sources and pictures in answers (doc/chat-spec.md): pictures come from a
//! fetched page, never a search (search is text only, so it stays fast). A
//! page fetched in a turn numbers its sources and pictures, the answer shows
//! some by number, and the history view hands out exactly those that
//! resolve — each picture kept once, scaled, with the session, and gone
//! with it.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use axum::{http::header, response::IntoResponse, routing::get, Router};
use serde_json::{json, Value};

/// A product page on loopback: a main picture (2400 px wide), a step
/// picture, the same step picture at a second address, and one that 404s.
struct Site {
    url: String,
}

fn png(w: u32, h: u32, shade: u8) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([shade, 90, 140]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

const PAGE: &str = r#"<html><head><title>Kettle</title>
<meta property="og:title" content="Gooseneck Kettle">
<meta property="og:image" content="/big.png">
<meta property="product:price:amount" content="89.00">
<meta property="product:price:currency" content="USD">
</head><body><p>Pour slowly.</p>
<figure><img src="/step.png"><figcaption>Step 1: fill to the line</figcaption></figure>
<img src="/again/step.png" alt="The same step">
<img src="/missing.png" alt="Gone">
</body></html>"#;

impl Site {
    async fn start() -> Self {
        let html = || async { ([(header::CONTENT_TYPE, "text/html")], PAGE).into_response() };
        let picture = |bytes: Vec<u8>| {
            move || {
                let bytes = bytes.clone();
                async move { ([(header::CONTENT_TYPE, "image/png")], bytes).into_response() }
            }
        };
        let app = Router::new()
            .route("/kettle", get(html))
            .route("/big.png", get(picture(png(2400, 1200, 10))))
            .route("/step.png", get(picture(png(300, 200, 200))))
            .route("/again/step.png", get(picture(png(300, 200, 200))));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind site");
        let url = format!("http://{}", listener.local_addr().expect("site addr"));
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { url }
    }
}

const ANSWER: &str = "The Gooseneck [turn1image1] pours well [turn1search1].\n\n\
    1. Fill it [turn1image2][turn1image3]\n2. Boil it [turn1image4]\n\nAlso [turn9image9].";

async fn fetch_and_answer(w: &World, sid: &str) -> Value {
    let root = w.work();
    w.turn(
        sid,
        &root,
        "Which kettle, and how do I use it?",
        Agent::Ling,
    )
    .await;
    w.api.history(sid, &root).await
}

fn session_dir(w: &World, sid: &str) -> std::path::PathBuf {
    w.home.linggen_home().join("sessions").join(sid)
}

/// Numbered in what the model reads, resolved in what the chat reads: the
/// shown pictures kept once each and scaled, a 404 and a made-up number
/// left out, nothing sent back to the model as a picture.
#[tokio::test]
async fn an_answer_shows_the_pictures_it_names_and_nothing_else() {
    let site = Site::start().await;
    let page = format!("{}/kettle", site.url);
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("WebFetch", json!({"url": page})),
            text(ANSWER),
            text("Done."),
        ],
    ))
    .await;
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "kettle", "project_root": root}))
        .await;
    let history = fetch_and_answer(&w, &sid).await;

    // What the model was told: WebFetch brings pictures, WebSearch none.
    let first_call = &w.model.calls_of(Agent::Ling)[0];
    let fetch = first_call.tool_description("WebFetch");
    assert!(
        fetch.contains("[turn2image1]") && fetch.contains("looks like"),
        "{fetch}"
    );
    let search = first_call.tool_description("WebSearch");
    assert!(!search.is_empty() && !search.contains("image1"), "{search}");

    // What the model read: numbers beside each thing, never a picture's address.
    let answer_call = &w.model.calls_of(Agent::Ling)[1];
    let read = answer_call.text();
    assert!(read.contains("[turn1search1]"), "the page has no number");
    assert!(read.contains("[turn1image1] Gooseneck Kettle"), "{read}");
    assert!(
        read.contains("[turn1image2] Step 1: fill to the line"),
        "{read}"
    );
    assert!(
        !read.contains("big.png"),
        "the model read a picture's address"
    );

    // What the chat reads.
    let refs = history["refs"].as_object().expect("refs").clone();
    let mut ids: Vec<&str> = refs.keys().map(String::as_str).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["turn1image1", "turn1image2", "turn1image3", "turn1search1"],
        "a 404 or a made-up number resolved: {refs:?}"
    );
    assert_eq!(refs["turn1search1"]["url"], page);
    assert_eq!(refs["turn1image1"]["price"], "89.00 USD");
    assert_eq!(
        refs["turn1image2"]["src"], refs["turn1image3"]["src"],
        "one picture behind two addresses is kept once"
    );
    let saved = history["messages"].to_string();
    assert!(
        saved.contains("[turn1image1]"),
        "the answer lost its numbers"
    );

    // Kept with the session: two files, the big one scaled to 1000 px.
    let images = session_dir(&w, &sid).join("images");
    let files: Vec<_> = std::fs::read_dir(&images)
        .expect("images/")
        .flatten()
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
    let src = refs["turn1image1"]["src"].as_str().expect("src");
    let bytes = reqwest::get(format!("{}{src}", w.engine.base_url()))
        .await
        .expect("get picture")
        .bytes()
        .await
        .expect("picture bytes");
    let big = image::load_from_memory(&bytes).expect("a picture");
    assert_eq!((big.width(), big.height()), (1000, 500));

    // The next turn: no picture rides back to the model.
    w.turn(&sid, &root, "Thanks", Agent::Ling).await;
    let next = w
        .model
        .calls_of(Agent::Ling)
        .pop()
        .expect("next call")
        .text();
    for wire in ["image_url", "input_image", "data:image"] {
        assert!(!next.contains(wire), "a picture was resent ({wire})");
    }

    // Deleted with the session.
    w.api
        .delete("/api/sessions/all", json!({"session_id": sid}))
        .await;
    assert!(
        !session_dir(&w, &sid).exists(),
        "the session's pictures outlived it"
    );
    w.assert_all_scripted();
}

/// On the Responses API a GPT model writes its own citation markers; each
/// id inside one resolves like the plain form.
#[tokio::test]
async fn gpt_citation_markers_resolve_like_plain_numbers() {
    let site = Site::start().await;
    let page = format!("{}/kettle", site.url);
    let marked = "Pick this one.\u{e200}i\u{e202}turn1image1\u{e201} \
        \u{e200}cite\u{e202}turn1search1\u{e201}";
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![tool("WebFetch", json!({"url": page})), text(marked)],
    ))
    .await;
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "kettle", "project_root": root}))
        .await;
    w.api
        .patch(
            "/api/sessions",
            json!({"session_id": sid, "agent_id": "ling", "model_id": "fake-sol"}),
        )
        .await;
    let history = fetch_and_answer(&w, &sid).await;
    let calls = w.model.calls_of(Agent::Ling);
    assert!(
        calls.iter().all(|c| c.path == "/v1/responses"),
        "not the Responses API"
    );

    let refs = history["refs"].as_object().expect("refs");
    assert!(
        refs.contains_key("turn1image1") && refs.contains_key("turn1search1"),
        "{refs:?}"
    );
    let saved = history["messages"].to_string();
    assert!(
        saved.contains("Pick this one.[turn1image1] [turn1search1]"),
        "{saved}"
    );
    assert!(!saved.contains('\u{e200}'), "a raw marker was saved");
    w.assert_all_scripted();
}
