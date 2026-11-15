//! Asset packs (doc/asset-pack-spec.md): a skill's page asks for a file its
//! folder lacks and the manifest lists. The dice fixture lists three files:
//! `index.html` (in its folder), `art/held.svg` (in the store, put there by
//! the fixture) and `art/late.svg` (nowhere, and its base refuses every
//! connection). What is fetched, ordered and swept is the unit tests'; this
//! is the route, the serving and the state, on a real engine.

use crate::support::{Setup, World};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use serde_json::Value;
use std::time::Duration;

const HELD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#;

async fn get(w: &World, path: &str) -> reqwest::Response {
    let url = format!("{}{path}", w.engine.base_url());
    reqwest::get(url).await.expect("get")
}

/// The engine reads the manifests just after it starts; wait for that.
async fn state_of(w: &World, done: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..200 {
        let resp = get(w, "/api/assets/dice").await;
        if resp.status() == 200 {
            let state: Value = resp.json().await.expect("json");
            if done(&state) {
                return state;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the asset state never got there");
}

#[tokio::test]
async fn a_listed_file_is_served_from_the_store_and_the_rest_as_before() {
    let w = World::start(Setup::default()).await;
    state_of(&w, |_| true).await;

    // In the folder: served from the folder, listed or not.
    let page = get(&w, "/apps/dice/index.html").await;
    assert_eq!(page.status(), 200);
    let folder = w.home.linggen_home().join("skills/dice/index.html");
    assert_eq!(
        page.text().await.expect("text"),
        std::fs::read_to_string(folder).expect("read")
    );

    // Listed and stored: the stored object, with its content type.
    let held = get(&w, "/apps/dice/art/held.svg").await;
    assert_eq!(held.status(), 200);
    assert_eq!(held.headers()[CONTENT_TYPE], "image/svg+xml");
    assert_eq!(held.text().await.expect("text"), HELD);

    // A range, and a HEAD, as for any file.
    let url = format!("{}/apps/dice/art/held.svg", w.engine.base_url());
    let client = reqwest::Client::new();
    let part = client
        .get(&url)
        .header(RANGE, "bytes=5-9")
        .send()
        .await
        .expect("range");
    assert_eq!(part.status(), 206);
    assert_eq!(part.headers()[CONTENT_RANGE], "bytes 5-9/62");
    assert_eq!(part.text().await.expect("text"), "xmlns");
    let head = client.head(&url).send().await.expect("head");
    assert_eq!(head.status(), 200);
    assert_eq!(head.headers()[CONTENT_LENGTH], "62");

    // Listed and not stored, and not listed: a plain 404 either way.
    assert_eq!(get(&w, "/apps/dice/art/late.svg").await.status(), 404);
    assert_eq!(get(&w, "/apps/dice/art/nothing.svg").await.status(), 404);
}

#[tokio::test]
async fn the_state_counts_what_is_wanted_held_and_failing() {
    let w = World::start(Setup::default()).await;
    // late.svg is tried against a base that refuses; it shows as failing.
    let state = state_of(&w, |s| s["packs"][1]["failing"] == 1).await;

    assert_eq!(state["files"], 3);
    assert_eq!(state["bytes"], 62 + 1193 + 4);
    assert_eq!(state["held_files"], 2);
    assert_eq!(state["held_bytes"], 62 + 1193);
    let (shared, later) = (&state["packs"][0], &state["packs"][1]);
    assert_eq!(shared["pack"], "shared");
    assert_eq!(shared["files"], 2);
    assert_eq!(shared["held_files"], 2);
    assert_eq!(shared["failing"], 0);
    assert_eq!(later["pack"], "later");
    assert_eq!(later["files"], 1);
    assert_eq!(later["held_files"], 0);
    assert_eq!(later["bytes"], 4);

    // A skill without a manifest has no state.
    assert_eq!(get(&w, "/api/assets/quiet").await.status(), 404);
}
