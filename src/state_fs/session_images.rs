//! The pictures a person's message carried, kept beside the session's
//! transcript: `<session>/images/<sha256-16>.<ext>`. A row names them
//! (`ChatMsg::images`); the chat surfaces fetch each over the data channel
//! (`GET /api/sessions/images`) to draw it again after a reload.
//!
//! Kept as sent — the web UI already scales a picture down before sending
//! (long edge 1568 px). Only PNG, JPEG, GIF and WebP are kept, at most
//! [`MAX_IMAGES`] per message and [`MAX_IMAGE_BYTES`] each; anything else is
//! dropped (the turn still sees it — only the copy for the transcript is
//! skipped).

use super::sessions::SessionStore;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Pictures kept per message.
pub const MAX_IMAGES: usize = 8;
/// Largest picture kept, decoded.
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

const IMAGES_DIR: &str = "images";

/// A picture as the chat sends it: bare base64, or a `data:` URL.
fn decode(sent: &str) -> Option<Vec<u8>> {
    let b64 = match sent.split_once(";base64,") {
        Some((head, body)) if head.starts_with("data:") => body,
        _ => sent,
    };
    base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()
}

/// A kept picture's name: 16 hex digits, a dot, a known extension.
pub fn valid_name(name: &str) -> bool {
    let Some((stem, ext)) = name.split_once('.') else {
        return false;
    };
    stem.len() == 16
        && stem
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && matches!(ext, "png" | "jpg" | "gif" | "webp")
}

/// The media type a kept picture is served as.
pub fn media_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("png") => "image/png",
        Some("jpg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Where the chat surfaces fetch a kept picture.
pub fn url(session_id: &str, name: &str) -> String {
    format!(
        "/api/sessions/images?session_id={}&name={}",
        urlencoding::encode(session_id),
        urlencoding::encode(name)
    )
}

/// A row's meta for the chat surfaces, with the URLs of the pictures the
/// row names (`images`), when it names any.
pub fn with_urls(
    mut meta: serde_json::Value,
    session_id: &str,
    names: &[String],
) -> serde_json::Value {
    if names.is_empty() {
        return meta;
    }
    let urls: Vec<String> = names.iter().map(|n| url(session_id, n)).collect();
    if let Some(obj) = meta.as_object_mut() {
        obj.insert("images".into(), serde_json::json!(urls));
    }
    meta
}

impl SessionStore {
    /// Keep the pictures a message carried; their names, in order. One that
    /// doesn't decode, isn't a known kind or is too big is skipped.
    pub fn keep_images(&self, session_id: &str, sent: &[String]) -> Vec<String> {
        if sent.is_empty() || session_id.contains("..") || session_id.contains('/') {
            return Vec::new();
        }
        let dir = self.session_dir(session_id).join(IMAGES_DIR);
        sent.iter()
            .take(MAX_IMAGES)
            .filter_map(|s| self.keep_one(&dir, s))
            .collect()
    }

    fn keep_one(&self, dir: &std::path::Path, sent: &str) -> Option<String> {
        let bytes = decode(sent)?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let ext = crate::pictures::kind::Kind::drawable(&bytes)?.ext();
        let digest = Sha256::digest(&bytes);
        let stem: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        let name = format!("{stem}.{ext}");
        let path = dir.join(&name);
        if !path.exists() {
            std::fs::create_dir_all(dir).ok()?;
            std::fs::write(&path, &bytes).ok()?;
        }
        Some(name)
    }

    /// The file of a kept picture, when the name is one and it exists.
    pub fn image_path(&self, session_id: &str, name: &str) -> Option<PathBuf> {
        if !valid_name(name) || session_id.is_empty() || session_id.contains("..") {
            return None;
        }
        if session_id.contains('/') || session_id.contains('\\') {
            return None;
        }
        let path = self.session_dir(session_id).join(IMAGES_DIR).join(name);
        path.is_file().then_some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_1PX: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    #[test]
    fn a_messages_pictures_are_kept_and_found_again() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let data_url = format!("data:image/png;base64,{PNG_1PX}");
        let names = store.keep_images(
            "sess-1-a",
            &[
                PNG_1PX.to_string(),
                data_url,
                "not base64!".into(),
                "aGVsbG8=".into(),
            ],
        );
        assert_eq!(names.len(), 2, "the same picture twice, the rest dropped");
        assert_eq!(names[0], names[1], "named by content");
        assert!(names[0].ends_with(".png") && valid_name(&names[0]));
        let path = store.image_path("sess-1-a", &names[0]).expect("kept");
        assert!(std::fs::read(path).unwrap().starts_with(&[0x89, b'P']));
        assert_eq!(media_type(&names[0]), "image/png");
        assert!(store.image_path("sess-1-a", "../session.yaml").is_none());
        assert!(store.image_path("../x", &names[0]).is_none());
        assert!(store.image_path("sess-other", &names[0]).is_none());
    }
}
