//! What kind of picture some bytes are, by their first bytes — never by a
//! file name. One table for every place that asks: the session's kept
//! pictures, the answer cache, `Read`, and the providers' media types.

use base64::Engine as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
    /// HEIC/HEIF (an iPhone photo): not drawable as is; `Read` converts it
    /// on macOS.
    Heic,
}

/// HEIF brands at offset 8 of an ISO media file (`....ftyp<brand>`).
const HEIC_BRANDS: [&[u8; 4]; 7] = [
    b"heic", b"heix", b"hevc", b"heim", b"heis", b"mif1", b"msf1",
];

impl Kind {
    /// The kind of `bytes` by their head; `None` for anything else.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0x89, b'P', b'N', b'G', ..] => Some(Kind::Png),
            [0xFF, 0xD8, 0xFF, ..] => Some(Kind::Jpeg),
            [b'G', b'I', b'F', b'8', ..] => Some(Kind::Gif),
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(Kind::Webp),
            [_, _, _, _, b'f', b't', b'y', b'p', a, b, c, d, ..]
                if HEIC_BRANDS.contains(&&[*a, *b, *c, *d]) =>
            {
                Some(Kind::Heic)
            }
            _ => None,
        }
    }

    /// The kind of a drawable picture: PNG, JPEG, GIF or WebP.
    pub fn drawable(bytes: &[u8]) -> Option<Self> {
        Self::sniff(bytes).filter(|k| *k != Kind::Heic)
    }

    pub fn ext(self) -> &'static str {
        match self {
            Kind::Png => "png",
            Kind::Jpeg => "jpg",
            Kind::Gif => "gif",
            Kind::Webp => "webp",
            Kind::Heic => "heic",
        }
    }

    pub fn media_type(self) -> &'static str {
        match self {
            Kind::Png => "image/png",
            Kind::Jpeg => "image/jpeg",
            Kind::Gif => "image/gif",
            Kind::Webp => "image/webp",
            Kind::Heic => "image/heic",
        }
    }

    /// How a result names it: "PNG".
    pub fn label(self) -> &'static str {
        match self {
            Kind::Png => "PNG",
            Kind::Jpeg => "JPEG",
            Kind::Gif => "GIF",
            Kind::Webp => "WebP",
            Kind::Heic => "HEIC",
        }
    }

    /// The decoder for it; HEIC has none here.
    pub fn format(self) -> Option<image::ImageFormat> {
        match self {
            Kind::Png => Some(image::ImageFormat::Png),
            Kind::Jpeg => Some(image::ImageFormat::Jpeg),
            Kind::Gif => Some(image::ImageFormat::Gif),
            Kind::Webp => Some(image::ImageFormat::WebP),
            Kind::Heic => None,
        }
    }
}

/// The media type of a picture sent as bare base64 — what a provider names
/// it as. PNG when the head says nothing (the old default).
pub fn media_type_of_base64(b64: &str) -> &'static str {
    // 16 base64 characters decode to the 12 bytes the sniff needs.
    let head = b64.get(..16).unwrap_or(b64);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(head)
        .unwrap_or_default();
    Kind::drawable(&bytes).map_or("image/png", Kind::media_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_head_names_the_kind() {
        assert_eq!(Kind::sniff(b"\x89PNG\r\n\x1a\n...."), Some(Kind::Png));
        assert_eq!(Kind::sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0]), Some(Kind::Jpeg));
        assert_eq!(Kind::sniff(b"GIF89a...."), Some(Kind::Gif));
        assert_eq!(Kind::sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Kind::Webp));
        assert_eq!(Kind::sniff(b"\0\0\0\x18ftypheic\0\0"), Some(Kind::Heic));
        assert_eq!(Kind::sniff(b"\0\0\0\x18ftypmif1\0\0"), Some(Kind::Heic));
        assert_eq!(Kind::sniff(b"\0\0\0\x18ftypisom\0\0"), None, "an mp4");
        assert_eq!(Kind::sniff(b"RIFF\0\0\0\0WAVEfmt "), None, "a wav");
        assert_eq!(Kind::sniff(b"just text, named .png"), None);
        assert_eq!(Kind::sniff(b""), None);
        assert_eq!(Kind::drawable(b"\0\0\0\x18ftypheic\0\0"), None);
    }

    #[test]
    fn a_sent_picture_is_named_by_its_bytes() {
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let jpeg = b64(&[
            0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1,
        ]);
        assert_eq!(media_type_of_base64(&jpeg), "image/jpeg");
        assert_eq!(media_type_of_base64(&b64(b"GIF89a..........")), "image/gif");
        assert_eq!(
            media_type_of_base64(&b64(b"\x89PNG\r\n\x1a\n........")),
            "image/png"
        );
        assert_eq!(media_type_of_base64("not base64!"), "image/png");
    }
}
