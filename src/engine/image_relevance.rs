//! Whether a page or a picture `ImageSearch` found is about the query, by
//! the words they share (`doc/chat-spec.md` § Sources and pictures in
//! answers). Plain token overlap with stop-words: no model, no site names.
//! A query in a script without spaces between words (Chinese, Japanese,
//! Korean) is not split into words here, so it is not filtered at all.

/// Words that carry no subject: English function words and search filler.
const STOP: &[&str] = &[
    "a",
    "an",
    "the",
    "and",
    "or",
    "of",
    "for",
    "to",
    "in",
    "on",
    "at",
    "by",
    "with",
    "from",
    "is",
    "are",
    "be",
    "it",
    "its",
    "this",
    "that",
    "these",
    "those",
    "as",
    "into",
    "about",
    "vs",
    "versus",
    "between",
    "difference",
    "differences",
    "compare",
    "comparison",
    "how",
    "what",
    "which",
    "why",
    "when",
    "where",
    "who",
    "do",
    "does",
    "best",
    "top",
    "good",
    "new",
    "guide",
    "picture",
    "pictures",
    "photo",
    "photos",
    "image",
    "images",
    "diagram",
    "diagrams",
    "view",
    "look",
    "looks",
    "like",
    "show",
    "shows",
    "example",
    "examples",
    "type",
    "types",
    "kind",
    "kinds",
    "not",
    "no",
    "your",
    "my",
    "our",
    "you",
    "we",
    "can",
    "will",
    "html",
    "htm",
    "php",
    "aspx",
    "www",
    "com",
    "en",
    "us",
    "blog",
    "blogs",
];

/// The query's subject words, or `None` when it has none to judge by.
pub struct Query {
    words: Vec<String>,
}

impl Query {
    pub fn new(query: &str) -> Option<Self> {
        if query.chars().any(is_cjk) {
            return None;
        }
        let mut words = words(query);
        words.sort();
        words.dedup();
        (!words.is_empty()).then_some(Query { words })
    }

    /// A page is about the query when its title and address path carry at
    /// least half the query's words: a business listing that shares one or
    /// two of them by its name is not. (`non` counts: "non-pressurized" is
    /// the point of a query that has it.)
    pub fn covers_page(&self, title: &str, url: &str) -> bool {
        let page = words(&format!("{title} {}", url_path(url)));
        let hits = self.words.iter().filter(|q| has(&page, q)).count();
        hits * 2 >= self.words.len()
    }

    /// A picture is about the query unless its own words (caption, file
    /// name) share none with it. A picture with no words is kept.
    pub fn allows_picture(&self, own_words: &str) -> bool {
        let own = words(own_words);
        own.is_empty() || self.words.iter().any(|q| has(&own, q))
    }
}

fn has(words: &[String], q: &str) -> bool {
    words.iter().any(|w| same_word(w, q))
}

/// Equal, or one is the other's form: `descale` / `descaling`,
/// `pressure` / `pressurized` — five letters in common and at least three
/// quarters of the shorter word.
fn same_word(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let common = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let shorter = a.chars().count().min(b.chars().count());
    common >= 5 && common * 4 >= shorter * 3
}

/// Lower-case subject words, plurals folded (`baskets` → `basket`).
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() > 1 && !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !STOP.contains(&w.as_str()))
        .map(singular)
        .collect()
}

fn singular(w: String) -> String {
    match w.strip_suffix('s') {
        Some(stem) if stem.chars().count() > 3 && !stem.ends_with('s') => stem.to_string(),
        _ => w,
    }
}

/// An address's path, never its host: no site's name decides anything.
fn url_path(url: &str) -> String {
    url::Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_default()
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUERY: &str = "pressurized portafilter basket vs non-pressurized basket espresso";

    #[test]
    fn business_listings_that_share_a_word_are_not_about_the_query() {
        let q = Query::new(QUERY).unwrap();
        assert!(q.covers_page(
            "Pressurized vs. non-pressurized baskets",
            "https://www.breville.com/us/en/blog/coffee-and-espresso/pressurized-vs-non-pressurized-portafilter.html",
        ));
        assert!(q.covers_page(
            "Pressurized vs Non-Pressurized Baskets: Understanding Dual Wall vs Single Wall Espresso",
            "https://www.hibrew.com/blogs/brew-guides/dual-wall-vs-single-wall-espresso-baskets",
        ));
        assert!(!q.covers_page(
            "Flower Basket & Espresso Depot",
            "https://www.mapquest.com/us/nevada/flower-basket-espresso-depot-291770628",
        ));
        assert!(!q.covers_page(
            "Pacific Pressurized Systems and Repair LLC",
            "https://www.mapquest.com/us/washington/pacific-pressurized-systems-repair-llc-708789733",
        ));
    }

    #[test]
    fn a_picture_whose_own_words_miss_the_query_is_dropped() {
        let q = Query::new(QUERY).unwrap();
        assert!(!q.allows_picture("everything you need to know about vacuum"));
        assert!(!q.allows_picture("Cherry Blossom"));
        assert!(q.allows_picture("pressurized basket double wall"));
        assert!(q.allows_picture("pressurized vs non pressurized baskets"));
        assert!(q.allows_picture(""), "a picture with no words is kept");
    }

    #[test]
    fn word_forms_match_and_a_host_never_counts() {
        let q = Query::new("descale breville espresso machine").unwrap();
        assert!(q.covers_page(
            "How to descaling your machine",
            "https://a.example/espresso"
        ));
        assert!(!q.covers_page("Home", "https://descale-breville-espresso.example/"));
        assert!(!same_word("portable", "portafilter"));
        let short = Query::new("gooseneck kettle").unwrap();
        assert!(short.covers_page(
            "Fellow Stagg EKG Electric Kettle",
            "https://a.example/p/stagg"
        ));
    }

    #[test]
    fn a_cjk_or_empty_query_is_not_judged() {
        assert!(Query::new("加压粉碗 basket").is_none());
        assert!(Query::new("the best photos").is_none());
    }
}
