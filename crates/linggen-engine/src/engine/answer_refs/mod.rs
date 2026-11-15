//! Sources and pictures in answers (`doc/chat-spec.md`).
//!
//! Every web result, picture and video a tool returns gets a number in the
//! shape GPT models already write — `turn{N}search{i}` for a source,
//! `turn{N}image{i}` for a picture, `turn{N}video{i}` for a video — kept in a per-session table
//! ([`table`]). The model refers to one by writing `[turn2image1]` in its
//! answer; it never writes its own picture URL. When the answer lands, the
//! pictures it shows are downloaded once, scaled and kept with the session
//! ([`cache`]); the chat surfaces read each shown number back through the
//! state endpoints ([`view`]). A number that matches nothing shows nothing.

pub mod cache;
pub mod register;
pub mod table;
pub mod view;

use serde::{Deserialize, Serialize};

/// What a number points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Source,
    Picture,
    Video,
}

/// One numbered result, as the session's table keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ref {
    pub id: String,
    pub kind: Kind,
    /// A source's page; a picture's original address (the click-through);
    /// a video's watch page.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption: String,
    /// A product's name and price, when the page declared them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub price: String,
    /// The page a picture was found on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub page: String,
    /// A video's still (kept like a picture), the address that plays it in
    /// a page, its channel and length (`4:05`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thumb: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub embed: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub duration: String,
    /// The kept copy under the session's `images/`, once an answer showed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Past the session's cache cap: the picture loads from `url`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_url: bool,
    /// Its download failed: never shown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
}

impl Ref {
    pub fn source(id: String, url: &str, title: &str) -> Self {
        Self::new(id, Kind::Source, url, title)
    }

    pub fn picture(id: String, url: &str, caption: &str) -> Self {
        let mut r = Self::new(id, Kind::Picture, url, "");
        r.caption = caption.to_string();
        r
    }

    pub fn video(id: String, url: &str, title: &str, thumb: &str) -> Self {
        let mut r = Self::new(id, Kind::Video, url, title);
        r.thumb = thumb.to_string();
        r
    }

    fn new(id: String, kind: Kind, url: &str, title: &str) -> Self {
        Self {
            id,
            kind,
            url: url.to_string(),
            title: title.to_string(),
            caption: String::new(),
            name: String::new(),
            price: String::new(),
            page: String::new(),
            thumb: String::new(),
            embed: String::new(),
            channel: String::new(),
            duration: String::new(),
            file: None,
            from_url: false,
            failed: false,
        }
    }

    /// Whether it shows as a picture the answer cache keeps: a picture, or
    /// a video's still.
    pub fn drawn(&self) -> bool {
        matches!(self.kind, Kind::Picture | Kind::Video)
    }

    /// The address its picture downloads from: a video's still, else its own.
    pub fn picture_url(&self) -> &str {
        match self.kind {
            Kind::Video => &self.thumb,
            _ => &self.url,
        }
    }

    /// Settled: kept, past the cap, or failed — nothing left to download.
    pub fn settled(&self) -> bool {
        self.file.is_some() || self.from_url || self.failed
    }
}

/// The id for a tool result's `i`th source or picture (1-based).
pub fn source_id(turn: u32, i: usize) -> String {
    format!("turn{turn}search{i}")
}

pub fn picture_id(turn: u32, i: usize) -> String {
    format!("turn{turn}image{i}")
}

pub fn video_id(turn: u32, i: usize) -> String {
    format!("turn{turn}video{i}")
}

/// `turn{N}search{i}`, `turn{N}image{i}` or `turn{N}video{i}`.
pub fn is_ref_id(s: &str) -> bool {
    parse_id(s).is_some()
}

/// (turn, kind, index) of an id.
pub fn parse_id(s: &str) -> Option<(u32, Kind, usize)> {
    let rest = s.strip_prefix("turn")?;
    let (turn, rest) = split_digits(rest)?;
    let (kind, rest) = KINDS
        .iter()
        .find_map(|(word, kind)| rest.strip_prefix(word).map(|r| (*kind, r)))?;
    let (index, rest) = split_digits(rest)?;
    rest.is_empty().then_some((turn as u32, kind, index))
}

/// The word between an id's two numbers, for each kind.
const KINDS: [(&str, Kind); 3] = [
    ("search", Kind::Source),
    ("image", Kind::Picture),
    ("video", Kind::Video),
];

fn split_digits(s: &str) -> Option<(usize, &str)> {
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    if n == 0 || n > 6 {
        return None;
    }
    Some((s[..n].parse().ok()?, &s[n..]))
}

/// The ids an answer writes, `[turn2image1]`, in order, each once.
pub fn ids_in(text: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("[turn") {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        let id = &after[..close];
        if is_ref_id(id) && !ids.iter().any(|i| i == id) {
            ids.push(id.to_string());
        }
        rest = &after[close.min(after.len())..];
    }
    ids
}

/// A number a model wrote as a markdown link or image — `![alt](turn2image1)`,
/// `[site](turn2search1)` — in the plain form every model is told,
/// `[turn2image1]`, so a saved answer holds one form for every model.
pub fn plain_refs(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains("](") {
        return std::borrow::Cow::Borrowed(text);
    }
    LINKED_REF.replace_all(text, "[$1]")
}

static LINKED_REF: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"!?\[[^\]\n]*\]\(\s*<?(turn\d{1,6}(?:search|image|video)\d{1,6})>?\s*\)").unwrap()
});

/// `text` without its `[turn…]` numbers — the words alone, for a voice or a
/// surface that cannot draw them.
pub fn strip_ids(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("[turn") {
        let after = &rest[open + 1..];
        let close = after.find(']');
        let is_id = close.is_some_and(|c| is_ref_id(&after[..c]));
        let Some(close) = close.filter(|_| is_id) else {
            out.push_str(&rest[..open + 1]);
            rest = after;
            continue;
        };
        out.push_str(rest[..open].trim_end_matches(' '));
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_parse_in_both_shapes_and_nothing_else() {
        assert_eq!(parse_id("turn2search1"), Some((2, Kind::Source, 1)));
        assert_eq!(parse_id("turn12image3"), Some((12, Kind::Picture, 3)));
        assert_eq!(parse_id("turn4video2"), Some((4, Kind::Video, 2)));
        for bad in ["turn2search", "turnsearch1", "turn2news1", "turn2image1x", "Turn1image1", ""] {
            assert!(!is_ref_id(bad), "{bad}");
        }
        assert_eq!(source_id(3, 1), "turn3search1");
        assert_eq!(picture_id(3, 2), "turn3image2");
        assert_eq!(video_id(3, 1), "turn3video1");
    }

    #[test]
    fn an_answer_names_its_ids_in_order_once() {
        let text = "Kettle A [turn1image2] or B [turn1image1][turn1search1]; \
                    again [turn1image2], a [link](x) and [turn9news1].";
        assert_eq!(ids_in(text), vec!["turn1image2", "turn1image1", "turn1search1"]);
        assert!(ids_in("[turn").is_empty());
    }

    #[test]
    fn a_number_written_as_a_markdown_link_or_image_becomes_plain() {
        let text = "看图：\n![MOVE 2.0 官网图 1](turn2image1)\n![](turn2image2) \
                    from [the maker]( turn2search1 ), a [real link](https://a.example/turn1image1) \
                    and [turn3image1] as is.";
        assert_eq!(
            plain_refs(text),
            "看图：\n[turn2image1]\n[turn2image2] from [turn2search1], a \
             [real link](https://a.example/turn1image1) and [turn3image1] as is."
        );
        assert_eq!(plain_refs("Watch [the video](turn4video1)."), "Watch [turn4video1].");
        assert!(matches!(plain_refs("no links"), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn stripping_leaves_the_words() {
        assert_eq!(
            strip_ids("Buy A [turn1image1][turn1search2]. See [notes] and [turn1x]."),
            "Buy A. See [notes] and [turn1x]."
        );
    }
}
