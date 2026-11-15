//! Picture search: real pictures of a thing, each with the page it came
//! from (`doc/chat-spec.md` § Sources and pictures in answers).
//!
//! One web search (the same own-key / Linggen Cloud path as `WebSearch`,
//! text only, so it stays fast), then the top result pages are fetched at
//! once and their pictures read the way `WebFetch` reads them
//! (`page_pictures`: the pictures in the text, then the page's cover). A
//! page whose title and address do not carry the query, and a picture
//! whose own words share none with it, are left out (`image_relevance`).
//! A slow page is left out rather than waited for. No picture is
//! downloaded here; a model that sees gets a contact sheet of them
//! (`picture_sheet`), and the answer cache keeps the ones an answer shows.

use super::image_relevance::Query;
use super::page_pictures::WebPicture;
use super::web_search::WebSearchResult;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Search results asked for; the top [`PAGES`] that are about the query and
/// are not videos are read.
const SEARCHED: usize = 10;
const PAGES: usize = 5;
/// Pictures kept from one page, and in all.
const PER_PAGE: usize = 3;
const MAX_PICTURES: usize = 10;
/// The pages' share of the search: one past it is left out.
const PAGES_BUDGET: Duration = Duration::from_secs(5);
/// Bytes of a page's text read (its pictures are read from the whole page).
const PAGE_TEXT: usize = 4 * 1024;

/// A picture found for a search: what it shows and the page it is on.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct FoundPicture {
    #[serde(flatten)]
    pub picture: WebPicture,
    /// The page it was found on, and that page's title.
    pub page: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub page_title: String,
}

/// Pictures for `query`, best page first.
pub async fn image_search(query: &str) -> Result<Vec<FoundPicture>> {
    let results = super::web_search::web_search(query, SEARCHED).await?;
    let judge = Query::new(query);
    let pages: Vec<WebSearchResult> = results
        .into_iter()
        .filter(|r| !super::video_search::is_video_page(&r.url))
        .filter(|r| {
            judge
                .as_ref()
                .is_none_or(|q| q.covers_page(&r.title, &r.url))
        })
        .take(PAGES)
        .collect();
    let read = pages.iter().map(|r| page_pictures(r, judge.as_ref()));
    Ok(gather(futures_util::future::join_all(read).await))
}

/// One page's pictures about the query, or none when it is slow, fails or
/// is not HTML.
async fn page_pictures(result: &WebSearchResult, judge: Option<&Query>) -> Vec<FoundPicture> {
    let fetched = tokio::time::timeout(
        PAGES_BUDGET,
        super::web_fetch::fetch_url(&result.url, Some(PAGE_TEXT)),
    )
    .await;
    let Ok(Ok(page)) = fetched else {
        return Vec::new();
    };
    let title = first_non_empty(&page.title, &result.title);
    page.pictures
        .into_iter()
        .filter(|p| judge.is_none_or(|q| q.allows_picture(&p.own_words())))
        .take(PER_PAGE)
        .map(|picture| FoundPicture {
            picture,
            page: result.url.clone(),
            page_title: title.clone(),
        })
        .collect()
}

fn first_non_empty(a: &str, b: &str) -> String {
    if a.trim().is_empty() { b } else { a }.trim().to_string()
}

/// Every page's pictures taken in turn — each page's best first, so one
/// picture-heavy page cannot crowd the others out — each address once.
fn gather(pages: Vec<Vec<FoundPicture>>) -> Vec<FoundPicture> {
    let mut out: Vec<FoundPicture> = Vec::new();
    let deepest = pages.iter().map(Vec::len).max().unwrap_or(0);
    for rank in 0..deepest {
        for found in pages.iter().filter_map(|p| p.get(rank)) {
            if out.len() >= MAX_PICTURES {
                return out;
            }
            if out.iter().all(|o| o.picture.url != found.picture.url) {
                out.push(found.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(page: &str, url: &str) -> FoundPicture {
        FoundPicture {
            picture: WebPicture {
                url: url.into(),
                caption: format!("from {page}"),
                ..Default::default()
            },
            page: page.into(),
            page_title: String::new(),
        }
    }

    #[test]
    fn pages_take_turns_and_an_address_counts_once() {
        let a = vec![found("a", "1"), found("a", "2"), found("a", "3")];
        let b = vec![found("b", "1"), found("b", "4")];
        let got: Vec<(String, String)> = gather(vec![a, Vec::new(), b])
            .into_iter()
            .map(|f| (f.page, f.picture.url))
            .collect();
        let want = [("a", "1"), ("a", "2"), ("b", "4"), ("a", "3")];
        let want: Vec<(String, String)> = want
            .iter()
            .map(|(p, u)| (p.to_string(), u.to_string()))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn at_most_ten_pictures() {
        let pages: Vec<Vec<FoundPicture>> = (0..6)
            .map(|p| {
                (0..3)
                    .map(|i| found(&format!("p{p}"), &format!("{p}-{i}")))
                    .collect()
            })
            .collect();
        assert_eq!(gather(pages).len(), MAX_PICTURES);
    }

    #[test]
    fn a_page_without_a_title_is_named_by_its_search_result() {
        assert_eq!(first_non_empty("  ", "Search title"), "Search title");
        assert_eq!(first_non_empty("Page", "Search title"), "Page");
    }
}
