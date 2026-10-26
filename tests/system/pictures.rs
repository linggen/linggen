//! Sources and pictures in answers (doc/chat-spec.md): pictures come from a
//! fetched page or a picture search, never WebSearch (text only, so it stays
//! fast); videos from a video search. A page fetched in a turn numbers its
//! sources and pictures, the answer shows some by number, and the history
//! view hands out exactly those that resolve — each picture kept once,
//! scaled, with the session, and gone with it.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use axum::{
    http::header,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::Engine as _;
use serde_json::{json, Value};

/// A product page on loopback: a cover picture (2400 px wide), a step
/// picture, the same step picture at a second address, and one that 404s;
/// and a business listing that only a search finds.
/// It also stands in for the cloud search (`/api/search`, finding the page,
/// or a video for a `site:youtube.com` query) and for YouTube's oEmbed,
/// watch page and stills.
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

const LISTING: &str = r#"<html><head><title>Pacific Systems and Repair LLC</title></head>
<body><img src="/listing.png" alt="Gooseneck kettle repair shop"></body></html>"#;

impl Site {
    async fn start() -> Self {
        let html = || async { ([(header::CONTENT_TYPE, "text/html")], PAGE).into_response() };
        let picture = |bytes: Vec<u8>| {
            move || {
                let bytes = bytes.clone();
                async move { ([(header::CONTENT_TYPE, "image/png")], bytes).into_response() }
            }
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind site");
        let url = format!("http://{}", listener.local_addr().expect("site addr"));
        let site = url.clone();
        let search = move |Json(body): Json<Value>| {
            let site = site.clone();
            async move { Json(search_results(&body, &site)) }
        };
        let app = Router::new()
            .route("/kettle", get(html))
            .route(
                "/biz/pacific-repair-708789733",
                get(|| async { ([(header::CONTENT_TYPE, "text/html")], LISTING).into_response() }),
            )
            .route("/listing.png", get(picture(png(300, 200, 120))))
            .route("/big.png", get(picture(png(2400, 1200, 10))))
            .route("/step.png", get(picture(png(300, 200, 200))))
            .route("/again/step.png", get(picture(png(300, 200, 200))))
            .route("/api/search", post(search))
            .route(
                "/oembed",
                get(|| async { Json(json!({"author_name": "Kettle School"})) }),
            )
            .route(
                "/watch",
                get(|| async { r#"<script>var p = {"lengthSeconds":"245"};</script>"# }),
            )
            .route(
                &format!("/vi/{VIDEO}/hqdefault.jpg"),
                get(picture(png(480, 360, 60))),
            );
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { url }
    }
}

const VIDEO: &str = "Kett1e_vid0";

/// What the cloud search answers: a business listing that shares no word
/// with the query and the kettle page, or for a query narrowed to YouTube
/// one video.
fn search_results(body: &Value, site: &str) -> Value {
    let query = body["query"].as_str().unwrap_or_default();
    if query.starts_with("site:youtube.com ") {
        let watch = format!("https://www.youtube.com/watch?v={VIDEO}");
        return json!({"results": [
            {"title": "How to fill a gooseneck - YouTube", "url": watch, "content": "Fill to the line."},
            {"title": "A channel", "url": "https://www.youtube.com/@KettleSchool", "content": ""}
        ]});
    }
    json!({"results": [
        {"title": "Pacific Systems and Repair LLC", "url": format!("{site}/biz/pacific-repair-708789733"), "content": ""},
        {"title": "Gooseneck kettle: fill it step by step", "url": format!("{site}/kettle"), "content": "A kettle."}
    ]})
}

const ANSWER: &str = "The Gooseneck [turn1image4] pours well [turn1search1].\n\n\
    1. Fill it [turn1image1][turn1image2]\n2. Boil it [turn1image3]\n\nAlso [turn9image9].";

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
    assert!(
        read.contains("[turn1image1] Step 1: fill to the line"),
        "{read}"
    );
    assert!(
        read.contains(
            "[turn1image4] big (the page's cover picture) — Gooseneck Kettle · 89.00 USD"
        ),
        "the cover is marked and never captioned by the page title: {read}"
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
        vec!["turn1image1", "turn1image2", "turn1image4", "turn1search1"],
        "a 404 or a made-up number resolved: {refs:?}"
    );
    assert_eq!(refs["turn1search1"]["url"], page);
    assert_eq!(refs["turn1image4"]["price"], "89.00 USD");
    assert_eq!(
        refs["turn1image1"]["src"], refs["turn1image2"]["src"],
        "one picture behind two addresses is kept once"
    );
    let saved = history["messages"].to_string();
    assert!(
        saved.contains("[turn1image4]"),
        "the answer lost its numbers"
    );

    // Kept with the session: two files, the big one scaled to 1000 px.
    let images = session_dir(&w, &sid).join("images");
    let files: Vec<_> = std::fs::read_dir(&images)
        .expect("images/")
        .flatten()
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
    let src = refs["turn1image4"]["src"].as_str().expect("src");
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

/// What a picture search asks: the page's title carries it, the listing's
/// does not; the step pictures' words share some of it, the cover's and the
/// broken one's none.
const QUERY: &str = "fill a gooseneck kettle step by step";

/// A picture search and a video search, each numbered in what the model
/// reads (no address in it), and the answer shows a found picture with the
/// page it came from and a video card with its still kept like a picture.
#[tokio::test]
async fn found_pictures_and_videos_show_with_where_they_came_from() {
    let site = Site::start().await;
    let answer = "The Gooseneck [turn1image1].\n\nHow to fill it: [turn2video1]";
    let mut setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool("ImageSearch", json!({"query": QUERY})),
            tool("VideoSearch", json!({"query": "fill a gooseneck kettle"})),
            text(answer),
        ],
    );
    setup.env = vec![
        ("LINGGEN_SITE_URL", site.url.clone()),
        ("LINGGEN_YOUTUBE_URL", site.url.clone()),
    ];
    let w = World::start(setup).await;
    std::fs::write(
        w.home.linggen_home().join("account.toml"),
        "api_token = \"test-token\"\n",
    )
    .expect("sign in");
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "kettle", "project_root": root}))
        .await;
    let history = fetch_and_answer(&w, &sid).await;

    // What the model was told, and what it read.
    let calls = w.model.calls_of(Agent::Ling);
    let first = &calls[0];
    assert!(
        first.text().contains("Show real pictures and videos"),
        "no pictures rule"
    );
    assert!(first
        .tool_description("ImageSearch")
        .contains("[turn3image1]"));
    let read = calls.last().expect("answer call").text();
    assert!(
        read.contains("[turn1image1] Step 1: fill to the line — from Gooseneck Kettle (127.0.0.1)"),
        "{read}"
    );
    assert!(read.contains("[turn1image2] The same step"), "{read}");
    for left_out in ["repair shop", "Gone", "cover picture", "turn1image3"] {
        assert!(
            !read.contains(left_out),
            "kept what has no words of the query: {left_out}\n{read}"
        );
    }
    assert!(
        !read.contains("contact sheet") && calls.iter().all(|c| c.images().is_empty()),
        "a blind model was sent a contact sheet"
    );
    assert!(
        read.contains("[turn2video1] How to fill a gooseneck — Kettle School · 4:05"),
        "{read}"
    );
    for address in ["big.png", "hqdefault", "watch?v="] {
        assert!(
            !read.contains(address),
            "the model read an address: {address}"
        );
    }

    // What the chat reads.
    let refs = history["refs"].as_object().expect("refs");
    let picture = &refs["turn1image1"];
    assert_eq!(picture["page"], format!("{}/kettle", site.url));
    assert!(picture["src"]
        .as_str()
        .is_some_and(|s| s.starts_with("/api/sessions/images")));
    let video = &refs["turn2video1"];
    assert_eq!(video["kind"], "video");
    assert_eq!(
        video["url"],
        format!("https://www.youtube.com/watch?v={VIDEO}")
    );
    assert_eq!(
        video["embed"],
        format!("https://www.youtube-nocookie.com/embed/{VIDEO}")
    );
    assert_eq!(
        (video["channel"].as_str(), video["duration"].as_str()),
        (Some("Kettle School"), Some("4:05"))
    );
    assert!(
        video["src"]
            .as_str()
            .is_some_and(|s| s.starts_with("/api/sessions/images")),
        "the still is kept with the session: {video}"
    );
    w.assert_all_scripted();
}

/// A model that sees gets the found pictures once, as one small contact
/// sheet in the turn that searched, and is told to pick by what it shows.
#[tokio::test]
async fn a_seeing_model_gets_one_contact_sheet_of_the_found_pictures() {
    let site = Site::start().await;
    let mut setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool("ImageSearch", json!({"query": QUERY})),
            text("Fill it like this [turn1image1]."),
        ],
    );
    setup.env = vec![("LINGGEN_SITE_URL", site.url.clone())];
    let w = World::start(setup).await;
    std::fs::write(
        w.home.linggen_home().join("account.toml"),
        "api_token = \"test-token\"\n",
    )
    .expect("sign in");
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "kettle", "project_root": root}))
        .await;
    w.api
        .patch(
            "/api/sessions",
            json!({"session_id": sid, "agent_id": "ling", "model_id": "fake-eye"}),
        )
        .await;
    w.turn(&sid, &root, "How do I fill a gooseneck?", Agent::Ling)
        .await;

    let calls = w.model.calls_of(Agent::Ling);
    let answer = calls.last().expect("answer call");
    assert_eq!(answer.model(), "fake-eye-1", "not the seeing model");
    let read = answer.text();
    assert!(read.contains("one contact sheet"), "{read}");
    assert!(read.contains("a caption can be wrong"), "{read}");
    let sent = answer.images();
    assert_eq!(sent.len(), 1, "one sheet for the whole search");
    let b64 = sent[0]
        .strip_prefix("data:image/jpeg;base64,")
        .expect("named as JPEG");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("b64");
    let sheet = image::load_from_memory(&bytes).expect("a picture");
    assert!(sheet.width() <= 1024, "{}", sheet.width());
    assert!(sheet.width() < 600, "two tiles, one row: {}", sheet.width());
    w.assert_all_scripted();
}
