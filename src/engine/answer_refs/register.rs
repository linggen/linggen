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
        _ => false,
    }
}

fn valid_session(id: &str) -> bool {
    !id.is_empty() && !id.contains("..") && !id.contains('/') && !id.contains('\\')
}

fn number_search(dir: &std::path::Path, results: &mut [WebSearchResult], pictures: &mut [WebPicture]) {
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
        assert!(text.contains("[turn2search1] Best kettles — https://a.example/k"), "{text}");
        assert!(text.contains("[turn2image1]") && text.contains("\n[turn2image1] A gooseneck kettle\n"), "{text}");
        assert!(text.contains("write a number such as [turn2search1]"), "{text}");
        assert!(!text.contains("img.example"), "the model never reads a picture's address");

        let refs = table::load(&store.session_dir("sess-1-a"));
        assert_eq!(refs.len(), 4);
        assert_eq!(refs["turn1image1"].url, "https://img.example/k.jpg");
        assert_eq!(refs["turn2search1"].title, "Best kettles");
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
