//! What the model reads of a picture or video search: each find with its
//! number and what it is — never a picture's, a still's or a video's
//! address, so an answer shows one only by its number.

use super::image_search::FoundPicture;
use super::video_search::{clock, FoundVideo};

/// `sheet`: the pictures ride below as one contact sheet (the model's copy
/// for a model that sees; never the kept one).
pub fn picture_search(query: &str, pictures: &[FoundPicture], sheet: bool) -> String {
    if pictures.is_empty() {
        return format!(
            "ImageSearch: \"{query}\" — no pictures found. WebFetch a page that shows it (the maker's or a product page)."
        );
    }
    let mut out = format!("ImageSearch: \"{query}\" ({} pictures)", pictures.len());
    out.push_str(&how_to_show(
        pictures.first().map_or("", |p| p.picture.id.as_str()),
        "that picture, in the paragraph it illustrates",
    ));
    if sheet {
        out.push_str(&sheet_note(pictures));
    }
    for (i, p) in pictures.iter().enumerate() {
        out.push_str(&format!(
            "{} {} — from {}\n",
            head(&p.picture.id, i),
            picture_words(p),
            page_words(p)
        ));
    }
    out
}

pub fn video_search(query: &str, videos: &[FoundVideo]) -> String {
    if videos.is_empty() {
        return format!("VideoSearch: \"{query}\" — no videos found.");
    }
    let mut out = format!("VideoSearch: \"{query}\" ({} videos)", videos.len());
    out.push_str(&how_to_show(
        videos.first().map_or("", |v| v.id.as_str()),
        "that video as a card the person can play",
    ));
    for (i, v) in videos.iter().enumerate() {
        out.push_str(&format!("{} {}\n", head(&v.id, i), video_words(v)));
        if !v.snippet.is_empty() {
            out.push_str(&format!("   {}\n", v.snippet));
        }
    }
    out
}

fn sheet_note(pictures: &[FoundPicture]) -> String {
    let example = pictures
        .iter()
        .map(|p| p.picture.id.as_str())
        .find_map(|id| Some((id, id.rsplit_once("image")?.1)));
    let tile = match example {
        Some((id, n)) => format!(" (tile {n} is [{id}])"),
        None => String::new(),
    };
    format!(
        "They are attached below as one contact sheet, each tile labelled by its picture's last number{tile}. Pick by what a picture shows; a caption can be wrong.\n"
    )
}

fn how_to_show(first_id: &str, shows: &str) -> String {
    match first_id {
        "" => "\n".to_string(),
        id => format!(" — write a number such as [{id}] in the answer to show {shows}\n"),
    }
}

fn head(id: &str, i: usize) -> String {
    match id {
        "" => format!("{}.", i + 1),
        id => format!("[{id}]"),
    }
}

fn picture_words(p: &FoundPicture) -> String {
    p.picture.described()
}

/// The page by its title and site: enough to say where a picture is from,
/// without an address to copy.
fn page_words(p: &FoundPicture) -> String {
    let site = url::Url::parse(&p.page)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
        .unwrap_or_default();
    match (p.page_title.as_str(), site.as_str()) {
        ("", site) => site.to_string(),
        (title, "") => title.to_string(),
        (title, site) => format!("{title} ({site})"),
    }
}

fn video_words(v: &FoundVideo) -> String {
    let length = v.seconds.map(clock).unwrap_or_default();
    let about = [v.channel.as_str(), length.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    match about.as_str() {
        "" => v.title.clone(),
        about => format!("{} — {about}", v.title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::page_pictures::WebPicture;

    fn picture(id: &str) -> FoundPicture {
        FoundPicture {
            picture: WebPicture {
                url: "https://img.example/basket.jpg".into(),
                caption: "Dual-wall basket".into(),
                id: id.into(),
                ..Default::default()
            },
            page: "https://www.shop.example/baskets".into(),
            page_title: "Filter baskets".into(),
        }
    }

    #[test]
    fn a_picture_reads_as_its_number_caption_and_page() {
        let text = picture_search("basket", &[picture("turn3image1")], false);
        assert!(
            text.contains("write a number such as [turn3image1]"),
            "{text}"
        );
        assert!(
            text.contains("[turn3image1] Dual-wall basket — from Filter baskets (shop.example)\n"),
            "{text}"
        );
        assert!(
            !text.contains("img.example") && !text.contains("https://"),
            "{text}"
        );
        assert!(!text.contains("contact sheet"), "{text}");
        let unnumbered = picture_search("basket", &[picture("")], false);
        assert!(
            unnumbered.contains("1. Dual-wall basket") && !unnumbered.contains("write a number")
        );
        assert!(picture_search("x", &[], false).contains("no pictures found"));
    }

    #[test]
    fn a_seeing_model_is_told_to_pick_by_the_sheet() {
        let text = picture_search("basket", &[picture("turn3image1")], true);
        assert!(
            text.contains("contact sheet") && text.contains("tile 1 is [turn3image1]"),
            "{text}"
        );
        assert!(text.contains("a caption can be wrong"), "{text}");
    }

    #[test]
    fn a_cover_reads_as_the_page_cover_never_its_title() {
        let mut p = picture("turn3image2");
        p.picture.caption = String::new();
        p.picture.cover = true;
        let text = picture_search("basket", &[p], false);
        assert!(
            text.contains("[turn3image2] the page's cover picture — from Filter baskets"),
            "{text}"
        );
    }

    #[test]
    fn a_video_reads_as_its_number_title_channel_and_length() {
        let v = FoundVideo {
            url: "https://www.youtube.com/watch?v=jv_ZAQrKla8".into(),
            title: "Dial in espresso".into(),
            channel: "Bridge Coffee".into(),
            seconds: Some(137),
            thumbnail: "https://i.ytimg.com/vi/jv_ZAQrKla8/hqdefault.jpg".into(),
            embed: "https://www.youtube-nocookie.com/embed/jv_ZAQrKla8".into(),
            snippet: "Grind finer when…".into(),
            id: "turn4video1".into(),
        };
        let text = video_search("dial in", &[v]);
        assert!(
            text.contains(
                "[turn4video1] Dial in espresso — Bridge Coffee · 2:17\n   Grind finer when…\n"
            ),
            "{text}"
        );
        assert!(!text.contains("https://"), "{text}");
    }
}
