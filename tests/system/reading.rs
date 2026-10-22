//! Reading a picture file (doc/tool-spec.md Read): a model that sees gets
//! the picture once, scaled like a person's pasted one, in the turn that
//! read it; a blind model gets the plain facts. Either way the picture is
//! numbered, kept with the session, and shown by number in the answer —
//! still there after the file is gone.

use crate::support::model::{Agent, Call};
use crate::support::{text, tool, World};
use base64::Engine as _;
use serde_json::{json, Value};

/// A noisy picture, so it is large on disk the way a screenshot is.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let v = ((x * 7919 + y * 104_729) % 251) as u8;
        image::Rgb([v, v.wrapping_mul(3), v.wrapping_mul(7)])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

/// Every image the call carried, as its `data:` URL.
fn images(call: &Call) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                if let Some(url) = map.get("image_url").and_then(|u| u["url"].as_str()) {
                    out.push(url.to_string());
                }
                map.values().for_each(|v| walk(v, out));
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&call.body, &mut out);
    out
}

/// A session reading `shot.png` (2000×1200) on `model`, answering with
/// its number, then a second turn.
async fn read_and_answer(model: &str) -> (World, String) {
    let w = World::start(crate::support::Setup::default().script(
        Agent::Ling,
        vec![
            tool("Read", json!({"path": "shot.png"})),
            text("Here it is [turn1image1]."),
            text("You're welcome."),
        ],
    ))
    .await;
    let root = w.work();
    std::fs::write(
        std::path::Path::new(&root).join("shot.png"),
        png(2000, 1200),
    )
    .expect("write shot.png");
    let sid = w
        .api
        .create_session(json!({"title": "shot", "project_root": root}))
        .await;
    w.api
        .patch(
            "/api/sessions",
            json!({"session_id": sid, "agent_id": "ling", "model_id": model}),
        )
        .await;
    w.turn(&sid, &root, "What does shot.png show?", Agent::Ling)
        .await;
    w.turn(&sid, &root, "Thanks", Agent::Ling).await;
    (w, sid)
}

/// The number resolves to the session's kept copy, which outlives the file.
async fn assert_shown_after_the_file_is_gone(w: &World, sid: &str) {
    std::fs::remove_file(std::path::Path::new(&w.work()).join("shot.png")).expect("rm");
    let history = w.api.history(sid, &w.work()).await;
    let shown = &history["refs"]["turn1image1"];
    assert_eq!(shown["caption"], "shot.png", "{history}");
    let src = shown["src"].as_str().expect("src");
    let bytes = reqwest::get(format!("{}{src}", w.engine.base_url()))
        .await
        .expect("get picture")
        .bytes()
        .await
        .expect("bytes");
    let kept = image::load_from_memory(&bytes).expect("a picture");
    assert_eq!(kept.width().max(kept.height()), 1568, "kept as sent");
    let dir = w
        .home
        .linggen_home()
        .join("sessions")
        .join(sid)
        .join("images");
    assert_eq!(std::fs::read_dir(dir).expect("images/").count(), 1);
}

#[tokio::test]
async fn a_seeing_model_gets_the_picture_once_and_the_answer_shows_it() {
    let (w, sid) = read_and_answer("fake-eye").await;
    let calls = w.model.calls_of(Agent::Ling);
    assert!(
        calls.iter().all(|c| c.model() == "fake-eye-1"),
        "not the seeing model"
    );

    // The turn that read it: the facts, the number, and the picture once,
    // scaled like a pasted one.
    let read = calls[1].text();
    assert!(read.contains("a picture (PNG, 2000×1200"), "{read}");
    assert!(read.contains("[turn1image1] shot.png"), "{read}");
    assert!(read.contains("attached below"), "{read}");
    let sent = images(&calls[1]);
    assert_eq!(sent.len(), 1, "one picture in its turn");
    let b64 = sent[0]
        .strip_prefix("data:image/jpeg;base64,")
        .expect("named as JPEG");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("b64");
    let pic = image::load_from_memory(&bytes).expect("a picture");
    assert_eq!(pic.width(), 1568, "scaled to the chat input's long edge");

    // The next turn: no picture rides back.
    let next = calls.last().expect("next turn");
    assert!(images(next).is_empty(), "a picture was resent");
    assert!(!next.text().contains("data:image"));

    assert_shown_after_the_file_is_gone(&w, &sid).await;
    w.assert_all_scripted();
}

#[tokio::test]
async fn a_blind_model_reads_the_facts_and_the_answer_still_shows_it() {
    let (w, sid) = read_and_answer("fake-ling").await;
    let calls = w.model.calls_of(Agent::Ling);
    let read = calls[1].text();
    assert!(read.contains("a picture (PNG, 2000×1200"), "{read}");
    assert!(read.contains("cannot see pictures"), "{read}");
    assert!(read.contains("[turn1image1] shot.png"), "{read}");
    assert!(
        calls.iter().all(|c| images(c).is_empty()),
        "bytes reached a blind model"
    );
    assert!(
        !read.contains("PNG\\r\\n") && !read.contains("iVBOR"),
        "garbage text"
    );

    assert_shown_after_the_file_is_gone(&w, &sid).await;
    w.assert_all_scripted();
}
