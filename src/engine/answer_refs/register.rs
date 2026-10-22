//! Numbering a tool result: each source and picture it carries gets its id
//! in the session's table, and the id is written into the result so the
//! model reads it beside the thing it names.

use super::{picture_id, source_id, table, Ref};
use crate::engine::tools::ToolResult;
use crate::engine::web_search::{WebPicture, WebSearchResult};
use crate::state_fs::sessions::SessionStore;

/// Number `result` in `session_id`'s table. A result with nothing to number,
/// or a session whose table cannot be written, is left as it was.
pub fn number(store: &SessionStore, session_id: &str, result: &mut ToolResult) {
    if !numberable(result) || !valid_session(session_id) {
        return;
    }
    let dir = store.session_dir(session_id);
    match result {
        ToolResult::WebSearchResults {
            results, pictures, ..
        } => number_search(&dir, results, pictures),
        ToolResult::WebFetchContent {
            url,
            title,
            pictures,
            id,
            ..
        } => number_page(&dir, url, title, pictures, id),
        ToolResult::Screenshot { url, base64, id } => {
            number_screenshot(store, session_id, url, base64, id)
        }
        ToolResult::Picture {
            base64,
            caption,
            id,
            ..
        } => number_drawn(store, session_id, base64, caption, id),
        ToolResult::PictureFile {
            base64, name, id, ..
        } => number_picture_file(store, session_id, base64, name, id),
        _ => {}
    }
}

fn numberable(result: &ToolResult) -> bool {
    match result {
        ToolResult::WebSearchResults {
            results, pictures, ..
        } => !results.is_empty() || !pictures.is_empty(),
        ToolResult::WebFetchContent { .. } => true,
        ToolResult::Screenshot { base64, .. } => !base64.is_empty(),
        ToolResult::Picture { base64, .. } => !base64.is_empty(),
        ToolResult::PictureFile { base64, .. } => !base64.is_empty(),
        _ => false,
    }
}

fn valid_session(id: &str) -> bool {
    !id.is_empty() && !id.contains("..") && !id.contains('/') && !id.contains('\\')
}

fn number_search(
    dir: &std::path::Path,
    results: &mut [WebSearchResult],
    pictures: &mut [WebPicture],
) {
    let turn = table::number(dir, |turn| {
        let sources = results
            .iter()
            .enumerate()
            .map(|(i, r)| Ref::source(source_id(turn, i + 1), &r.url, &r.title));
        sources.chain(picture_rows(turn, pictures, "")).collect()
    });
    let Some(turn) = turn else { return };
    for (i, r) in results.iter_mut().enumerate() {
        r.id = source_id(turn, i + 1);
    }
    label_pictures(turn, pictures);
}

fn number_page(
    dir: &std::path::Path,
    url: &str,
    title: &str,
    pictures: &mut [WebPicture],
    id: &mut String,
) {
    let turn = table::number(dir, |turn| {
        let page = Ref::source(source_id(turn, 1), url, title);
        std::iter::once(page)
            .chain(picture_rows(turn, pictures, url))
            .collect()
    });
    let Some(turn) = turn else { return };
    *id = source_id(turn, 1);
    label_pictures(turn, pictures);
}

/// A screenshot is the session's own capture: kept at once (it is already
/// in the conversation), so showing it needs no download.
fn number_screenshot(
    store: &SessionStore,
    session_id: &str,
    url: &str,
    base64: &str,
    id: &mut String,
) {
    let Some(file) = store.keep_images(session_id, &[base64.to_string()]).pop() else {
        return;
    };
    let dir = store.session_dir(session_id);
    let turn = table::number(&dir, |turn| {
        let mut shot = Ref::picture(picture_id(turn, 1), "", &format!("Screenshot of {url}"));
        shot.page = url.to_string();
        shot.file = Some(file);
        vec![shot]
    });
    if let Some(turn) = turn {
        *id = picture_id(turn, 1);
    }
}

/// A drawn picture is kept at once, like a screenshot, and its bytes leave
/// the result: the session's copy is the one the chat shows.
fn number_drawn(
    store: &SessionStore,
    session_id: &str,
    base64: &mut String,
    caption: &str,
    id: &mut String,
) {
    let Some(file) = store
        .keep_images(session_id, &[std::mem::take(base64)])
        .pop()
    else {
        return;
    };
    let dir = store.session_dir(session_id);
    let turn = table::number(&dir, |turn| {
        let mut drawn = Ref::picture(picture_id(turn, 1), "", caption);
        drawn.file = Some(file);
        vec![drawn]
    });
    if let Some(turn) = turn {
        *id = picture_id(turn, 1);
    }
}

/// A picture file `Read` opened is kept at once (its readied copy, by
/// content), so the answer shows it even after the file changes or goes.
/// Its bytes stay in the result: the engine attaches them for a model that
/// can see.
fn number_picture_file(
    store: &SessionStore,
    session_id: &str,
    base64: &str,
    name: &str,
    id: &mut String,
) {
    let Some(file) = store.keep_images(session_id, &[base64.to_string()]).pop() else {
        return;
    };
    let dir = store.session_dir(session_id);
    let turn = table::number(&dir, |turn| {
        let mut read = Ref::picture(picture_id(turn, 1), "", name);
        read.file = Some(file);
        vec![read]
    });
    if let Some(turn) = turn {
        *id = picture_id(turn, 1);
    }
}

fn picture_rows(turn: u32, pictures: &[WebPicture], page: &str) -> Vec<Ref> {
    pictures
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut r = Ref::picture(picture_id(turn, i + 1), &p.url, &p.caption);
            r.name = p.name.clone();
            r.price = p.price.clone();
            r.page = page.to_string();
            r
        })
        .collect()
}

fn label_pictures(turn: u32, pictures: &mut [WebPicture]) {
    for (i, p) in pictures.iter_mut().enumerate() {
        p.id = picture_id(turn, i + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::render::render_tool_result;

    fn search() -> ToolResult {
        ToolResult::WebSearchResults {
            query: "kettle".into(),
            results: vec![WebSearchResult {
                title: "Best kettles".into(),
                url: "https://a.example/k".into(),
                snippet: "…".into(),
                id: String::new(),
            }],
            pictures: vec![WebPicture {
                url: "https://img.example/k.jpg".into(),
                caption: "A gooseneck kettle".into(),
                ..Default::default()
            }],
        }
    }

    #[test]
    fn results_are_numbered_in_the_table_and_in_what_the_model_reads() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let mut first = search();
        number(&store, "sess-1-a", &mut first);
        let mut second = search();
        number(&store, "sess-1-a", &mut second);

        let text = render_tool_result(&second);
        assert!(
            text.contains("[turn2search1] Best kettles — https://a.example/k"),
            "{text}"
        );
        assert!(
            text.contains("[turn2image1]") && text.contains("\n[turn2image1] A gooseneck kettle\n"),
            "{text}"
        );
        assert!(
            text.contains("write a number such as [turn2search1]"),
            "{text}"
        );
        assert!(
            !text.contains("img.example"),
            "the model never reads a picture's address"
        );

        let refs = table::load(&store.session_dir("sess-1-a"));
        assert_eq!(refs.len(), 4);
        assert_eq!(refs["turn1image1"].url, "https://img.example/k.jpg");
        assert_eq!(refs["turn2search1"].title, "Best kettles");
    }

    fn drawn(saved: Option<(String, String)>) -> ToolResult {
        use base64::Engine as _;
        ToolResult::Picture {
            base64: base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\ndrawn"),
            caption: "A coffee machine, flat style".into(),
            source: "ChatGPT".into(),
            seconds: 31.6,
            seed: None,
            saved,
            id: String::new(),
        }
    }

    #[test]
    fn a_drawn_picture_is_kept_at_once_and_shown_by_number() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let mut first = search();
        number(&store, "sess-1-a", &mut first);
        let mut picture = drawn(None);
        number(&store, "sess-1-a", &mut picture);

        let ToolResult::Picture {
            ref base64, ref id, ..
        } = picture
        else {
            unreachable!()
        };
        assert_eq!(id, "turn2image1");
        assert!(base64.is_empty(), "the bytes leave the result once kept");
        let text = render_tool_result(&picture);
        assert!(text.contains("drawn by ChatGPT in 32 s"), "{text}");
        assert!(
            text.contains("[turn2image1] A coffee machine, flat style"),
            "{text}"
        );
        assert!(text.contains("writing [turn2image1]"), "{text}");
        assert!(
            !text.contains("/api/") && !text.contains("http"),
            "no address to write: {text}"
        );

        let kept = &table::load(&store.session_dir("sess-1-a"))["turn2image1"];
        assert!(kept.settled() && kept.url.is_empty());
        let file = kept.file.clone().unwrap();
        assert!(
            store.image_path("sess-1-a", &file).is_some(),
            "kept in the session's images/"
        );
    }

    #[test]
    fn a_skill_picture_names_the_skills_copy_too() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let saved = (
            "/apps/game/data/pictures/fuzhu.png".to_string(),
            "/x/fuzhu.png".to_string(),
        );
        let mut picture = drawn(Some(saved));
        number(&store, "sess-1-a", &mut picture);
        let text = render_tool_result(&picture);
        assert!(
            text.contains("[turn1image1]") && text.contains("/apps/game/data/pictures/fuzhu.png"),
            "{text}"
        );
    }

    fn picture_file(png: &[u8]) -> ToolResult {
        use base64::Engine as _;
        ToolResult::PictureFile {
            path: "shots/login.png".into(),
            name: "login.png".into(),
            kind: "PNG".into(),
            width: 64,
            height: 48,
            size: 2048,
            base64: base64::engine::general_purpose::STANDARD.encode(png),
            seen: false,
            id: String::new(),
        }
    }

    fn png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(64, 48, image::Rgb([9, 99, 199]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// A picture file is numbered and kept with its file name as caption;
    /// read twice it is two numbers on one kept copy.
    #[test]
    fn a_read_picture_is_numbered_and_kept_once_by_content() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let mut first = picture_file(&png());
        number(&store, "sess-1-a", &mut first);
        let mut again = picture_file(&png());
        number(&store, "sess-1-a", &mut again);

        let ToolResult::PictureFile {
            ref id, ref base64, ..
        } = again
        else {
            unreachable!()
        };
        assert_eq!(id, "turn2image1");
        assert!(!base64.is_empty(), "the bytes stay for the model");
        let refs = table::load(&store.session_dir("sess-1-a"));
        assert_eq!(refs["turn1image1"].caption, "login.png");
        assert!(refs["turn1image1"].url.is_empty() && refs["turn1image1"].settled());
        assert_eq!(refs["turn1image1"].file, refs["turn2image1"].file);
        let kept = std::fs::read_dir(store.session_dir("sess-1-a").join("images"))
            .unwrap()
            .count();
        assert_eq!(kept, 1, "one copy for one content");
    }

    /// What the model reads: facts and the number, and whether it sees the
    /// picture; the kept row holds facts and number alone.
    #[test]
    fn a_read_picture_reads_as_facts_and_a_number() {
        use crate::engine::render::render_tool_result_public;
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let mut blind = picture_file(&png());
        number(&store, "sess-1-a", &mut blind);
        let text = render_tool_result(&blind);
        assert!(
            text.starts_with("Read: shots/login.png — a picture (PNG, 64×48, 2 KB), not text.\n"),
            "{text}"
        );
        assert!(text.contains("This model cannot see pictures"), "{text}");
        assert!(
            text.contains("[turn1image1] login.png\nShow it by writing [turn1image1]"),
            "{text}"
        );
        assert!(!text.contains("iVBOR"), "the bytes are never text");

        let ToolResult::PictureFile { ref mut seen, .. } = blind else {
            unreachable!()
        };
        *seen = true;
        let text = render_tool_result(&blind);
        assert!(text.contains("attached below"), "{text}");
        let kept = render_tool_result_public(&blind);
        assert!(
            !kept.contains("attached") && !kept.contains("cannot see"),
            "{kept}"
        );
        assert!(kept.contains("[turn1image1] login.png"), "{kept}");

        let mut unkept = picture_file(&png());
        number(&store, "../x", &mut unkept);
        assert!(render_tool_result(&unkept).contains("Not kept in this conversation"));
    }

    #[test]
    fn without_a_session_nothing_is_numbered() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let mut r = search();
        number(&store, "../x", &mut r);
        let text = render_tool_result(&r);
        assert!(text.contains("1. Best kettles"), "{text}");
        assert!(!text.contains("Pictures") && !text.contains("write a number"));
    }
}
