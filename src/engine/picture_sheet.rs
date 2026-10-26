//! The contact sheet a model that sees gets with `ImageSearch`'s pictures:
//! each candidate downloaded within a short budget and tiled, labelled by
//! its number, so the model picks by what a picture shows rather than by a
//! caption that can be wrong (doc/chat-spec.md § Sources and pictures in
//! answers). A blind model never pays for it.

use super::image_search::FoundPicture;
use crate::pictures::{for_model, kind::Kind, sheet};
use base64::Engine as _;
use std::time::Duration;

/// The whole sheet's download share; a picture still loading is left out.
const BUDGET: Duration = Duration::from_secs(4);

/// The sheet, and which of `pictures` are on it (in order).
pub struct Sheet {
    pub base64: String,
    pub on_sheet: Vec<bool>,
}

pub async fn build(pictures: &[FoundPicture]) -> Option<Sheet> {
    let urls = pictures.iter().map(|p| p.picture.url.clone()).collect();
    let fetched = super::answer_refs::cache::fetch_all(urls, BUDGET).await;
    let labels: Vec<u32> = pictures
        .iter()
        .enumerate()
        .map(|(i, p)| label(&p.picture.id, i))
        .collect();
    tokio::task::spawn_blocking(move || compose(labels, fetched))
        .await
        .ok()
        .flatten()
}

fn compose(labels: Vec<u32>, fetched: Vec<Option<Vec<u8>>>) -> Option<Sheet> {
    let decoded: Vec<Option<image::DynamicImage>> = fetched
        .iter()
        .map(|b| b.as_deref().and_then(decode))
        .collect();
    let on_sheet: Vec<bool> = decoded.iter().map(Option::is_some).collect();
    let tiles: Vec<(u32, image::DynamicImage)> = labels
        .into_iter()
        .zip(decoded)
        .filter_map(|(label, img)| Some((label, img?)))
        .collect();
    let bytes = sheet::compose(&tiles)?;
    Some(Sheet {
        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        on_sheet,
    })
}

fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    for_model::decode(bytes, Kind::drawable(bytes)?)
}

/// A tile's label: the number after `image` in `turn3image7`, else its
/// place in the list.
fn label(id: &str, i: usize) -> u32 {
    id.rsplit_once("image")
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(i as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(40, 30));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_picture_that_did_not_load_is_left_off_the_sheet() {
        let sheet = compose(
            vec![1, 2, 3],
            vec![Some(png()), None, Some(b"junk".to_vec())],
        )
        .expect("one picture loaded");
        assert_eq!(sheet.on_sheet, vec![true, false, false]);
        assert!(!sheet.base64.is_empty());
        assert!(compose(vec![1], vec![None]).is_none());
    }

    #[test]
    fn a_tile_is_labelled_by_its_number() {
        assert_eq!(label("turn3image7", 0), 7);
        assert_eq!(label("", 4), 5);
    }
}
