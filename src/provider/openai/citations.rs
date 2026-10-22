//! ChatGPT-trained models write their own citation markers into the reply
//! text: `\u{e200}cite\u{e202}turn2search1\u{e202}turn2search2\u{e201}` —
//! private-use characters (start U+E200, separator U+E202, end U+E201) that a
//! chat would show as box glyphs. Each id inside a marker that names a
//! numbered result (`turn{N}search{i}`, `turn{N}image{i}` — the numbers the
//! engine gave the session's tool results, `engine::answer_refs`) comes out
//! as the plain form every model is told, `[turn2search1]`, so the engine
//! resolves one form for all models. The rest of a marker (its kind word,
//! an `image_group` query, ids of another shape) is dropped. A marker can
//! span stream deltas: text from an open U+E200 is held until its U+E201
//! arrives.
//!
//! When the API does attach `url_citation` annotations (its hosted web
//! search), they are kept and read as markdown links after the reply.

use serde_json::Value;

const OPEN: char = '\u{e200}';
const CLOSE: char = '\u{e201}';
/// A held marker longer than this was never one: give the text back.
const MAX_HELD: usize = 400;

fn is_marker_char(c: char) -> bool {
    ('\u{e200}'..='\u{e2ff}').contains(&c)
}

#[derive(Default)]
pub(super) struct CitationFilter {
    held: String,
    links: Vec<(String, String)>,
}

impl CitationFilter {
    /// The visible part of the next piece of text; an unfinished marker is
    /// held for the next call.
    pub(super) fn push(&mut self, text: &str) -> String {
        self.held.push_str(text);
        let mut out = String::new();
        loop {
            let Some(open) = self.held.find(OPEN) else {
                out.push_str(&strip_strays(&self.held));
                self.held.clear();
                return out;
            };
            out.push_str(&strip_strays(&self.held[..open]));
            let rest = &self.held[open..];
            let Some(close) = rest.find(CLOSE) else {
                self.held = rest.to_string();
                if self.held.len() > MAX_HELD {
                    out.push_str(&strip_strays(&self.held));
                    self.held.clear();
                }
                return out;
            };
            out.push_str(&marker_refs(&rest[..close]));
            self.held = rest[close + CLOSE.len_utf8()..].to_string();
        }
    }

    /// A `response.output_text.annotation.added` event: keep a url_citation.
    pub(super) fn note_annotation(&mut self, val: &Value) {
        let Some(a) = val.get("annotation") else {
            return;
        };
        if a.get("type").and_then(Value::as_str) != Some("url_citation") {
            return;
        }
        let Some(url) = a.get("url").and_then(Value::as_str) else {
            return;
        };
        if self.links.iter().any(|(u, _)| u == url) {
            return;
        }
        let title = a.get("title").and_then(Value::as_str).unwrap_or(url);
        self.links.push((url.to_string(), title.to_string()));
    }

    /// The end of the reply: the sources as markdown links, if any came.
    /// A marker still open at the end is dropped.
    pub(super) fn finish(&mut self) -> String {
        self.held.clear();
        if self.links.is_empty() {
            return String::new();
        }
        let links: Vec<String> = self
            .links
            .drain(..)
            .map(|(url, title)| format!("[{}]({url})", title.replace(['[', ']'], "")))
            .collect();
        format!("\n\n{}", links.join(" · "))
    }
}

/// The plain `[id]` form of each numbered-result id in one marker (its text
/// from U+E200 up to U+E201).
fn marker_refs(marker: &str) -> String {
    marker
        .split(|c: char| is_marker_char(c))
        .map(str::trim)
        .filter(|part| crate::engine::answer_refs::is_ref_id(part))
        .map(|id| format!("[{id}]"))
        .collect()
}

fn strip_strays(text: &str) -> String {
    text.chars().filter(|c| !is_marker_char(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// As gpt-6-luna wrote it in a real chat (bytes from a saved session).
    const REAL: &str = "可以在 Canada 申请。\u{e200}cite\u{e202}turn2search1\u{e202}turn2search2\u{e201}下一步是填表。";

    #[test]
    fn a_whole_marker_becomes_plain_ids() {
        let mut f = CitationFilter::default();
        assert_eq!(
            f.push(REAL),
            "可以在 Canada 申请。[turn2search1][turn2search2]下一步是填表。"
        );
        assert_eq!(f.finish(), "");
    }

    #[test]
    fn a_picture_marker_keeps_its_ids_and_a_query_marker_is_dropped() {
        let mut f = CitationFilter::default();
        let text = "Two kettles:\u{e200}i\u{e202}turn1image1\u{e202}turn1image3\u{e201} \
                    \u{e200}image_group\u{e202}{\"query\":[\"kettle\"]}\u{e201}done";
        assert_eq!(f.push(text), "Two kettles:[turn1image1][turn1image3] done");
    }

    #[test]
    fn a_marker_split_across_deltas_still_resolves() {
        let mut f = CitationFilter::default();
        let mut out = String::new();
        for piece in [
            "可以在 Canada 申请。\u{e200}ci",
            "te\u{e202}turn2sea",
            "rch1\u{e202}turn2search2",
            "\u{e201}下一步",
            "是填表。",
        ] {
            out.push_str(&f.push(piece));
        }
        out.push_str(&f.finish());
        assert_eq!(
            out,
            "可以在 Canada 申请。[turn2search1][turn2search2]下一步是填表。"
        );
    }

    #[test]
    fn plain_text_passes_and_a_runaway_open_gives_its_text_back() {
        let mut f = CitationFilter::default();
        assert_eq!(f.push("Hello, world."), "Hello, world.");
        let long = format!("\u{e200}{}", "a".repeat(MAX_HELD));
        assert_eq!(f.push(&long), "a".repeat(MAX_HELD));
    }

    #[test]
    fn url_citations_read_as_links_after_the_reply() {
        let mut f = CitationFilter::default();
        let ann = |url: &str, title: &str| {
            json!({"type": "response.output_text.annotation.added",
                   "annotation": {"type": "url_citation", "url": url, "title": title,
                                  "start_index": 0, "end_index": 4}})
        };
        f.note_annotation(&ann("https://a.example/x", "A [news]"));
        f.note_annotation(&ann("https://a.example/x", "A again"));
        f.note_annotation(&ann("https://b.example/y", "B"));
        assert_eq!(
            f.finish(),
            "\n\n[A news](https://a.example/x) · [B](https://b.example/y)"
        );
    }
}
