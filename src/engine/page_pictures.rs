//! The pictures a fetched page carries: its main image (`og:image`, with
//! the page's title and — when the page declares a product — its price),
//! then the pictures in its text with their captions (`<figcaption>`, else
//! `alt`, else `title`, else the words of the file's name, else the page's
//! title). Read from the HTML before its tags are stripped. Nothing here
//! knows any site: only the standard meta tags and elements.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Pictures kept from one page.
const MAX_PICTURES: usize = 12;
/// A picture whose declared width or height is below this is an icon.
const MIN_SIDE: u32 = 100;
/// Words in an address that mark a page's furniture, not its content.
const FURNITURE: &[&str] = &[
    "logo", "icon", "sprite", "spacer", "avatar", "badge", "emoji", "favicon",
];

static META: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<meta\b[^>]*>").unwrap());
static IMG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<img\b[^>]*>").unwrap());
static FIGURE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<figure\b[^>]*>(.*?)</figure>").unwrap());
static FIGCAPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<figcaption\b[^>]*>(.*?)</figcaption>").unwrap());
static ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)([a-z_:-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<title\b[^>]*>(.*?)</title>").unwrap());

/// A picture a fetched page shows: its address, what it shows, and — when the
/// page declared a product — the product's name and price.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct WebPicture {
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub price: String,
    /// Its number in the session (`turn2image1`), once numbered.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
}

/// What a page says about itself: its title, and its pictures.
#[derive(Debug, Default, PartialEq)]
pub struct PageFacts {
    pub title: String,
    pub pictures: Vec<WebPicture>,
}

pub fn read(html: &str, page_url: &str) -> PageFacts {
    let base = url::Url::parse(page_url).ok();
    let meta = Meta::read(html);
    let title = first_non_empty(&[&meta.title, &title_tag(html)]);
    let mut pictures: Vec<WebPicture> = Vec::new();
    if let Some(main) = main_picture(&meta, &title, base.as_ref()) {
        pictures.push(main);
    }
    for mut pic in figures(html, base.as_ref())
        .into_iter()
        .chain(images(html, base.as_ref()))
    {
        if pictures.len() >= MAX_PICTURES {
            break;
        }
        if pictures.iter().any(|p| p.url == pic.url) {
            continue;
        }
        if pic.caption.is_empty() {
            pic.caption = title.clone();
        }
        pictures.push(pic);
    }
    PageFacts { title, pictures }
}

#[derive(Default)]
struct Meta {
    image: String,
    title: String,
    price: String,
    currency: String,
}

impl Meta {
    fn read(html: &str) -> Self {
        let mut meta = Meta::default();
        for tag in META.find_iter(html) {
            let attrs = attrs(tag.as_str());
            let key = attr(&attrs, "property")
                .or_else(|| attr(&attrs, "name"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            let content = attr(&attrs, "content").unwrap_or_default();
            meta.take(&key, content);
        }
        meta
    }

    fn take(&mut self, key: &str, content: String) {
        let slot = match key {
            "og:image" | "og:image:secure_url" | "twitter:image" => &mut self.image,
            "og:title" | "twitter:title" => &mut self.title,
            "product:price:amount" | "og:price:amount" => &mut self.price,
            "product:price:currency" | "og:price:currency" => &mut self.currency,
            _ => return,
        };
        if slot.is_empty() {
            *slot = decode(content.trim());
        }
    }
}

fn main_picture(meta: &Meta, title: &str, base: Option<&url::Url>) -> Option<WebPicture> {
    let url = absolute(&meta.image, base)?;
    let price = match (meta.price.as_str(), meta.currency.as_str()) {
        ("", _) => String::new(),
        (amount, "") => amount.to_string(),
        (amount, currency) => format!("{amount} {currency}"),
    };
    let name = if price.is_empty() { "" } else { title };
    Some(WebPicture {
        url,
        caption: title.to_string(),
        name: name.to_string(),
        price,
        id: String::new(),
    })
}

/// `<figure>` pictures, captioned by their `<figcaption>`.
fn figures(html: &str, base: Option<&url::Url>) -> Vec<WebPicture> {
    FIGURE
        .captures_iter(html)
        .filter_map(|cap| {
            let inner = cap.get(1)?.as_str();
            let mut pic = picture_of(IMG.find(inner)?.as_str(), base)?;
            if let Some(caption) = FIGCAPTION.captures(inner).and_then(|c| c.get(1)) {
                pic.caption = text_of(caption.as_str());
            }
            Some(pic)
        })
        .collect()
}

fn images(html: &str, base: Option<&url::Url>) -> Vec<WebPicture> {
    IMG.find_iter(html)
        .filter_map(|tag| picture_of(tag.as_str(), base))
        .collect()
}

/// One `<img>` as a picture, unless it is furniture: an icon-sized
/// declared size, a vector, an inline `data:` image, a logo or tracker.
fn picture_of(tag: &str, base: Option<&url::Url>) -> Option<WebPicture> {
    let attrs = attrs(tag);
    let src = attr(&attrs, "src")
        .filter(|s| !s.starts_with("data:"))
        .or_else(|| attr(&attrs, "data-src"))?;
    let url = absolute(&src, base)?;
    if is_furniture(&url) || too_small(&attrs) {
        return None;
    }
    let said = attr(&attrs, "alt").or_else(|| attr(&attrs, "title"));
    let caption = said
        .map(|s| decode(s.trim()))
        .unwrap_or_else(|| file_words(&url));
    Some(WebPicture {
        url,
        caption,
        ..Default::default()
    })
}

/// Words a file names its picture by — `move-schwarz-frontal.jpg` →
/// "move schwarz frontal": the name's stem split on `-`, `_`, `.`, spaces
/// and camelCase, without hashes, sizes, bare numbers and camera or CMS
/// filler. Never the address or its folders.
fn file_words(url: &str) -> String {
    let Some(name) = url::Url::parse(url)
        .ok()
        .and_then(|u| u.path_segments()?.next_back().map(str::to_string))
    else {
        return String::new();
    };
    let name = urlencoding::decode(&name).map_or(name.clone(), |n| n.into_owned());
    let stem = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem);
    stem.split(|c: char| matches!(c, '-' | '_' | '.' | ' ' | '+' | '@'))
        .flat_map(split_camel)
        .map(|w| w.to_lowercase())
        .filter(|w| is_word(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// File-name filler: camera prefixes, CMS size and role words.
const FILLER: &[&str] = &[
    "img", "image", "images", "dsc", "dscn", "dscf", "pic", "photo", "picture", "thumb",
    "thumbnail", "scaled", "crop", "cropped", "resized", "copy", "final", "web", "large",
    "medium", "small", "default", "untitled", "jpg", "jpeg", "png", "webp",
];

fn is_word(w: &str) -> bool {
    let digits = w.chars().filter(char::is_ascii_digit).count();
    let hexish = w.len() >= 6 && digits > 0 && w.chars().all(|c| c.is_ascii_hexdigit());
    let sized = SIZE.is_match(w);
    w.chars().count() > 1
        && digits < w.len()
        && digits < 4
        && !hexish
        && !sized
        && !FILLER.contains(&w)
}

/// `1024x768`, `800w`, `2x`, `300px` — a size, not a word.
static SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d+(x\d+|w|h|x|px)?$").unwrap());

/// `MoveSchwarz` → `Move`, `Schwarz`; a run of capitals stays whole.
fn split_camel(part: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut prev_lower = false;
    for c in part.chars() {
        if words.is_empty() || (c.is_uppercase() && prev_lower) {
            words.push(String::new());
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        words.last_mut().expect("a word").push(c);
    }
    words
}

fn is_furniture(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or("");
    path.ends_with(".svg") || FURNITURE.iter().any(|w| path.contains(w))
}

fn too_small(attrs: &[(String, String)]) -> bool {
    ["width", "height"].iter().any(|k| {
        attr(attrs, k)
            .and_then(|v| v.trim().trim_end_matches("px").parse::<u32>().ok())
            .is_some_and(|n| n < MIN_SIDE)
    })
}

fn attrs(tag: &str) -> Vec<(String, String)> {
    ATTR.captures_iter(tag)
        .map(|c| {
            let value = c.get(2).or_else(|| c.get(3)).map_or("", |m| m.as_str());
            (c[1].to_ascii_lowercase(), value.to_string())
        })
        .collect()
}

fn attr(attrs: &[(String, String)], key: &str) -> Option<String> {
    attrs
        .iter()
        .find(|(k, v)| k == key && !v.trim().is_empty())
        .map(|(_, v)| v.clone())
}

fn absolute(src: &str, base: Option<&url::Url>) -> Option<String> {
    let src = decode(src.trim());
    if src.is_empty() {
        return None;
    }
    let url = match base {
        Some(base) => base.join(&src).ok()?,
        None => url::Url::parse(&src).ok()?,
    };
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn title_tag(html: &str) -> String {
    TITLE
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| text_of(m.as_str()))
        .unwrap_or_default()
}

fn text_of(html: &str) -> String {
    let text = TAG.replace_all(html, " ");
    decode(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn first_non_empty(options: &[&str]) -> String {
    options
        .iter()
        .find(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><head>
        <title>Ignored when og:title is set</title>
        <meta property="og:title" content="Fellow Stagg EKG Kettle">
        <meta property="og:image" content="/img/stagg.jpg?w=1200&amp;q=80">
        <meta property="product:price:amount" content="165.00">
        <meta property="product:price:currency" content="USD">
        </head><body>
        <img src="/static/logo.png" alt="Shop">
        <img src="https://cdn.example/pixel.gif" width="1" height="1">
        <figure><img src="steps/1.jpg" alt="old alt"><figcaption>Step 1: <b>fill</b> it</figcaption></figure>
        <img src="data:image/png;base64,AAAA" data-src="/img/side.webp" alt="Side view">
        <img src="/img/tiny.jpg" width="40" alt="thumb">
        <img src="/img/stagg.jpg?w=1200&q=80" alt="dup of main">
        <img src="/icons/cart.svg">
        </body></html>"#;

    #[test]
    fn a_product_page_gives_its_main_picture_then_its_captioned_ones() {
        let facts = read(PAGE, "https://shop.example/p/stagg");
        assert_eq!(facts.title, "Fellow Stagg EKG Kettle");
        let urls: Vec<&str> = facts.pictures.iter().map(|p| p.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://shop.example/img/stagg.jpg?w=1200&q=80",
                "https://shop.example/p/steps/1.jpg",
                "https://shop.example/img/side.webp",
            ]
        );
        let main = &facts.pictures[0];
        assert_eq!(main.name, "Fellow Stagg EKG Kettle");
        assert_eq!(main.price, "165.00 USD");
        assert_eq!(facts.pictures[1].caption, "Step 1: fill it");
        assert_eq!(facts.pictures[2].caption, "Side view");
    }

    /// A maker's page as Profitec's is: no `og:image`, no `alt`.
    const BARE: &str = r#"<html><head><title>Profitec | MOVE</title></head><body>
        <img class="hero-img" src="/media/56c9caaf4c-1787297070/move-schwarz-frontal.jpg">
        <img src="/media/a68b9ecdc1/MoveGelbSchraeg_1024x768.webp">
        <img src="/media/x/DSC_0042.JPG" title="The drip tray">
        <img src="/media/x/IMG_20240101_1200.jpg">
        <img src="/media/x/3f2a9c81d0e4.png">
        </body></html>"#;

    #[test]
    fn an_uncaptioned_picture_is_named_by_its_file_else_the_page() {
        let facts = read(BARE, "https://www.profitec-espresso.com/en/products/move-2-0");
        let captions: Vec<&str> = facts.pictures.iter().map(|p| p.caption.as_str()).collect();
        assert_eq!(
            captions,
            vec![
                "move schwarz frontal",
                "move gelb schraeg",
                "The drip tray",
                "Profitec | MOVE",
                "Profitec | MOVE",
            ]
        );
        assert!(captions.iter().all(|c| !c.contains("media") && !c.contains('/')));
    }

    #[test]
    fn file_words_drop_hashes_sizes_numbers_and_filler() {
        assert_eq!(file_words("https://a.example/p/move-schwarz-fast_heat-up.jpg"), "move schwarz fast heat up");
        assert_eq!(file_words("https://a.example/rav4-cabin%20filter@2x.png"), "rav4 cabin filter");
        assert_eq!(file_words("https://a.example/img_9921-300x200.jpg"), "");
        assert_eq!(file_words("https://a.example/"), "");
    }

    #[test]
    fn a_plain_page_has_no_product_and_its_title_from_the_title_tag() {
        let facts = read(
            "<title>How to descale</title><p>no pictures</p>",
            "https://a.example/x",
        );
        assert_eq!(facts.title, "How to descale");
        assert!(facts.pictures.is_empty());
    }
}
