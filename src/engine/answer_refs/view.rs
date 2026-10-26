//! What the chat surfaces read of the table: for each number the answers
//! write that resolves, what to draw. A source is a link; a picture is its
//! kept copy's URL (`src`, fetched over the data channel), or past the cap
//! its own address; a video is its still's, plus where it plays. A picture
//! not kept yet, one whose download failed, and a number that matches
//! nothing are left out — they show nothing.

use super::{table, Kind, Ref};
use crate::state_fs::sessions::SessionStore;
use serde_json::{json, Map, Value};

/// `{id: {kind, url, title, …, src}}` for every resolvable id in `texts`.
pub fn refs_for<'a>(
    store: &SessionStore,
    session_id: &str,
    texts: impl IntoIterator<Item = &'a str>,
) -> Map<String, Value> {
    let mut ids: Vec<String> = Vec::new();
    for text in texts {
        ids.extend(super::ids_in(text));
    }
    let mut out = Map::new();
    if ids.is_empty() || session_id.is_empty() || session_id.contains('/') || session_id.contains("..") {
        return out;
    }
    let refs = table::load(&store.session_dir(session_id));
    for id in ids {
        let Some(shown) = refs.get(&id).and_then(|r| shown(session_id, r)) else {
            continue;
        };
        out.insert(id, shown);
    }
    out
}

/// The words of a session's rows that are someone's words — a tool's result
/// row carries numbers too, but only an answer shows them.
pub fn spoken_texts(rows: &[crate::state_fs::sessions::ChatMsg]) -> Vec<String> {
    rows.iter()
        .filter(|m| !m.is_observation && !super::ids_in(&m.content).is_empty())
        .map(|m| m.content.clone())
        .collect()
}

fn shown(session_id: &str, r: &Ref) -> Option<Value> {
    let mut v = json!({ "kind": r.kind, "url": r.url });
    let obj = v.as_object_mut()?;
    for (key, value) in [
        ("title", &r.title),
        ("caption", &r.caption),
        ("name", &r.name),
        ("price", &r.price),
        ("page", &r.page),
        ("embed", &r.embed),
        ("channel", &r.channel),
        ("duration", &r.duration),
    ] {
        if !value.is_empty() {
            obj.insert(key.into(), json!(value));
        }
    }
    if r.kind == Kind::Source {
        return (!r.url.is_empty()).then_some(v);
    }
    let src = picture_src(session_id, r)?;
    obj.insert("src".into(), json!(src));
    Some(v)
}

fn picture_src(session_id: &str, r: &Ref) -> Option<String> {
    if r.failed {
        return None;
    }
    if let Some(file) = &r.file {
        return Some(crate::state_fs::session_images::url(session_id, file));
    }
    r.from_url.then(|| r.picture_url().to_string())
}

#[cfg(test)]
mod tests {
    use super::super::{picture_id, source_id};
    use super::*;

    #[test]
    fn only_numbers_that_resolve_to_something_drawable_are_handed_out() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let sess = store.session_dir("sess-1-a");
        table::number(&sess, |t| {
            let mut kept = Ref::picture(picture_id(t, 1), "https://i.example/1.jpg", "kept");
            kept.file = Some("0123456789abcdef.jpg".into());
            let pending = Ref::picture(picture_id(t, 2), "https://i.example/2.jpg", "pending");
            let mut failed = Ref::picture(picture_id(t, 3), "https://i.example/3.jpg", "failed");
            failed.failed = true;
            let mut remote = Ref::picture(picture_id(t, 4), "https://i.example/4.jpg", "remote");
            remote.from_url = true;
            vec![Ref::source(source_id(t, 1), "https://a.example", "A"), kept, pending, failed, remote]
        });
        let refs = refs_for(
            &store,
            "sess-1-a",
            ["See [turn1search1] [turn1image1] [turn1image2]", "[turn1image3][turn1image4] [turn7image1]"],
        );
        let keys: Vec<&str> = refs.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["turn1search1", "turn1image1", "turn1image4"]);
        assert_eq!(
            refs["turn1image1"]["src"],
            "/api/sessions/images?session_id=sess-1-a&name=0123456789abcdef.jpg"
        );
        assert_eq!(refs["turn1image4"]["src"], "https://i.example/4.jpg");
        assert_eq!(refs["turn1search1"]["kind"], "source");
    }

    #[test]
    fn a_video_draws_its_still_and_says_where_it_plays() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let sess = store.session_dir("sess-1-a");
        table::number(&sess, |t| {
            let watch = "https://www.youtube.com/watch?v=jv_ZAQrKla8";
            let still = "https://i.ytimg.com/vi/jv_ZAQrKla8/hqdefault.jpg";
            let mut v = Ref::video(super::super::video_id(t, 1), watch, "Dial in", still);
            v.embed = "https://www.youtube-nocookie.com/embed/jv_ZAQrKla8".into();
            v.channel = "Bridge Coffee".into();
            v.duration = "2:17".into();
            v.from_url = true;
            let unkept = Ref::video(super::super::video_id(t, 2), watch, "Again", still);
            vec![v, unkept]
        });
        let refs = refs_for(&store, "sess-1-a", ["Watch [turn1video1] [turn1video2]"]);
        assert_eq!(refs.len(), 1, "a still not kept yet shows nothing");
        let v = &refs["turn1video1"];
        assert_eq!(v["kind"], "video");
        assert_eq!(v["src"], "https://i.ytimg.com/vi/jv_ZAQrKla8/hqdefault.jpg");
        assert_eq!(v["url"], "https://www.youtube.com/watch?v=jv_ZAQrKla8");
        assert_eq!(
            v["embed"],
            "https://www.youtube-nocookie.com/embed/jv_ZAQrKla8"
        );
        assert_eq!(
            (v["channel"].as_str(), v["duration"].as_str()),
            (Some("Bridge Coffee"), Some("2:17"))
        );
    }
}
