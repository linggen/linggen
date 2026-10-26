//! Keeping the pictures an answer shows. When an answer lands, each picture
//! it names that is not kept yet is downloaded once, scaled to about
//! [`LONG_EDGE`] px and kept under the session's `images/` — named by its
//! content, so one picture behind two addresses is kept once — beside the
//! person's own pictures, served the same way (`GET /api/sessions/images`,
//! over the data channel). Past the session's cap a picture loads from its
//! address instead; a failed download is never shown. Pictures a search
//! returned that no answer shows are never fetched.

use super::{table, Kind, Ref};
use crate::state_fs::sessions::SessionStore;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

/// The longest side a kept picture is scaled to.
pub const LONG_EDGE: u32 = 1000;
/// Answer pictures kept per session, in bytes; past it, pictures load from
/// their address. `LINGGEN_PICTURE_CACHE_CAP` (bytes) overrides it.
pub const DEFAULT_CAP: u64 = 30 * 1024 * 1024;
/// Largest download read.
const MAX_DOWNLOAD: usize = 12 * 1024 * 1024;
/// One answer waits at most this long for its pictures.
const ANSWER_BUDGET: Duration = Duration::from_secs(10);
/// Downloads at once.
const PARALLEL: usize = 6;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .expect("failed to build the picture client")
});

fn cap() -> u64 {
    std::env::var("LINGGEN_PICTURE_CACHE_CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CAP)
}

/// Keep the pictures `answer` shows in `session_id`. Returns whether any
/// picture's state changed.
pub async fn show(store: &SessionStore, session_id: &str, answer: &str) -> bool {
    let ids = super::ids_in(answer);
    if ids.is_empty() || session_id.contains('/') || session_id.contains("..") {
        return false;
    }
    let dir = store.session_dir(session_id);
    let refs = table::load(&dir);
    let wanted: Vec<Ref> = ids
        .iter()
        .filter_map(|id| refs.get(id))
        .filter(|r| r.kind == Kind::Picture && !r.settled())
        .cloned()
        .collect();
    if wanted.is_empty() {
        return false;
    }
    let mut used = kept_bytes(&dir, &refs);
    let fetched = download_all(&wanted).await;
    let mut updated = Vec::new();
    for (mut r, bytes) in wanted.into_iter().zip(fetched) {
        settle(&dir, &mut r, bytes, &mut used);
        updated.push(r);
    }
    table::update(&dir, &updated)
}

/// Every wanted picture's bytes, in order; one still downloading when the
/// answer's budget runs out counts as failed.
async fn download_all(wanted: &[Ref]) -> Vec<Option<Vec<u8>>> {
    use futures_util::stream::{self, StreamExt};
    let deadline = tokio::time::Instant::now() + ANSWER_BUDGET;
    let urls: Vec<String> = wanted.iter().map(|r| r.url.clone()).collect();
    stream::iter(urls.into_iter().map(move |url| download_by(url, deadline)))
        .buffered(PARALLEL)
        .collect()
        .await
}

/// A picture's new state from its download: kept, past the cap, or failed.
fn settle(dir: &Path, r: &mut Ref, bytes: Option<Vec<u8>>, used: &mut u64) {
    let Some(kept) = bytes.and_then(|b| scaled(&b)) else {
        r.failed = true;
        return;
    };
    let path = dir.join("images").join(&kept.name);
    if path.exists() {
        r.file = Some(kept.name);
        return;
    }
    if *used + kept.bytes.len() as u64 > cap() {
        r.from_url = true;
        return;
    }
    let written = std::fs::create_dir_all(dir.join("images"))
        .and_then(|_| std::fs::write(&path, &kept.bytes));
    if written.is_err() {
        r.failed = true;
        return;
    }
    *used += kept.bytes.len() as u64;
    r.file = Some(kept.name);
}

/// Bytes of answer pictures the session already keeps (each file once).
fn kept_bytes(dir: &Path, refs: &std::collections::HashMap<String, Ref>) -> u64 {
    let mut files: Vec<&str> = refs
        .values()
        .filter(|r| r.kind == Kind::Picture && !r.url.is_empty())
        .filter_map(|r| r.file.as_deref())
        .collect();
    files.sort_unstable();
    files.dedup();
    files
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join("images").join(f)).ok())
        .map(|m| m.len())
        .sum()
}

async fn download_by(url: String, deadline: tokio::time::Instant) -> Option<Vec<u8>> {
    tokio::time::timeout_at(deadline, download(&url))
        .await
        .ok()
        .flatten()
}

async fn download(url: &str) -> Option<Vec<u8>> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return None;
    }
    let resp = HTTP.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    if resp
        .content_length()
        .is_some_and(|n| n as usize > MAX_DOWNLOAD)
    {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    (bytes.len() <= MAX_DOWNLOAD).then(|| bytes.to_vec())
}

/// A picture as kept: its name (by the downloaded content) and bytes.
pub struct Kept {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The downloaded picture scaled to [`LONG_EDGE`]: a picture already that
/// small is kept as it came; a larger one is re-encoded (JPEG, or PNG when
/// it has transparency). Anything that is not a PNG, JPEG, GIF or WebP is
/// refused.
pub fn scaled(bytes: &[u8]) -> Option<Kept> {
    let kind = crate::pictures::kind::Kind::drawable(bytes)?;
    let ext = kind.ext();
    let stem = content_stem(bytes);
    let img = crate::pictures::for_model::decode(bytes, kind)?;
    if img.width().max(img.height()) <= LONG_EDGE {
        return Some(Kept {
            name: format!("{stem}.{ext}"),
            bytes: bytes.to_vec(),
        });
    }
    let small = img.resize(LONG_EDGE, LONG_EDGE, image::imageops::FilterType::Triangle);
    encode(small, &stem)
}

fn encode(img: image::DynamicImage, stem: &str) -> Option<Kept> {
    let mut out = std::io::Cursor::new(Vec::new());
    let ext = if img.color().has_alpha() {
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        "png"
    } else {
        let rgb = image::DynamicImage::ImageRgb8(img.to_rgb8());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
        rgb.write_with_encoder(encoder).ok()?;
        "jpg"
    };
    Some(Kept {
        name: format!("{stem}.{ext}"),
        bytes: out.into_inner(),
    })
}

/// 16 hex digits of the content's SHA-256 — the naming the session's
/// `images/` uses for every picture.
fn content_stem(bytes: &[u8]) -> String {
    Sha256::digest(bytes)[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            w,
            h,
            image::Rgb([200, 30, 30]),
        ));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_large_picture_is_scaled_and_a_small_one_kept_as_it_came() {
        let big = scaled(&png(2400, 1200)).expect("decodes");
        assert!(big.name.ends_with(".jpg"));
        let back = image::load_from_memory(&big.bytes).unwrap();
        assert_eq!((back.width(), back.height()), (1000, 500));

        let small_png = png(300, 200);
        let small = scaled(&small_png).unwrap();
        assert_eq!(small.bytes, small_png);
        assert!(crate::state_fs::session_images::valid_name(&small.name));
        assert!(scaled(b"<svg></svg>").is_none());
        assert!(
            scaled(&[0x89, b'P', b'N', b'G', 0, 0]).is_none(),
            "a broken PNG"
        );
    }

    #[test]
    fn settling_dedupes_by_content_and_stops_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut used = 0;
        let bytes = png(50, 50);
        let mut a = Ref::picture("turn1image1".into(), "https://a.example/1.png", "");
        let mut b = Ref::picture("turn1image2".into(), "https://b.example/same.png", "");
        settle(dir.path(), &mut a, Some(bytes.clone()), &mut used);
        settle(dir.path(), &mut b, Some(bytes.clone()), &mut used);
        assert_eq!(a.file, b.file, "one picture behind two addresses");
        assert_eq!(
            std::fs::read_dir(dir.path().join("images"))
                .unwrap()
                .count(),
            1
        );

        let mut c = Ref::picture("turn1image3".into(), "https://c.example/c.png", "");
        let mut over = cap();
        settle(dir.path(), &mut c, Some(png(60, 60)), &mut over);
        assert!(
            c.from_url && c.file.is_none(),
            "past the cap it loads from its address"
        );

        let mut d = Ref::picture("turn1image4".into(), "https://d.example/d.png", "");
        settle(dir.path(), &mut d, None, &mut used);
        assert!(d.failed, "a failed download is never shown");
    }
}
