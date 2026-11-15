//! A picture made ready for a model the way the chat input readies a
//! person's pasted one (`ui/src/lib/imageEncode.ts` `readImageForSend`):
//! one small enough on both counts ([`MAX_EDGE`] px on the long edge,
//! [`PASSTHROUGH_BYTES`]) goes as it came; a heavier one is re-encoded as
//! JPEG (q0.85, transparency on white) when that is smaller. One past the
//! edge is always scaled to fit — as JPEG, or PNG when a flat screenshot
//! is smaller that way — so a 2400 px capture never goes whole. Vision
//! models resample past ~1568 px anyway.

use super::kind::Kind;

pub const MAX_EDGE: u32 = 1568;
pub const PASSTHROUGH_BYTES: usize = 256 * 1024;
const JPEG_QUALITY: u8 = 85;

/// A picture as it goes to the model and into the session's `images/`.
pub struct Ready {
    pub bytes: Vec<u8>,
    /// The original's size in pixels.
    pub width: u32,
    pub height: u32,
}

/// Decode `bytes` of `kind` with bounds, so a hostile file cannot take the
/// machine's memory.
pub fn decode(bytes: &[u8], kind: Kind) -> Option<image::DynamicImage> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), kind.format()?);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_000);
    limits.max_image_height = Some(16_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().ok()
}

/// `bytes` (PNG, JPEG, GIF or WebP) ready for the model; `None` when they
/// do not decode.
pub fn ready(bytes: &[u8]) -> Option<Ready> {
    let kind = Kind::drawable(bytes)?;
    let img = decode(bytes, kind)?;
    let (width, height) = (img.width(), img.height());
    let as_came = || Ready {
        bytes: bytes.to_vec(),
        width,
        height,
    };
    let within = width.max(height) <= MAX_EDGE;
    if within && bytes.len() <= PASSTHROUGH_BYTES {
        return Some(as_came());
    }
    let smaller = if within {
        jpeg(&img).filter(|j| j.len() < bytes.len())
    } else {
        smallest(&fit(img))
    };
    let Some(bytes) = smaller else {
        return Some(as_came());
    };
    Some(Ready {
        bytes,
        width,
        height,
    })
}

/// A scaled picture in whichever of JPEG and PNG is smaller.
fn smallest(img: &image::DynamicImage) -> Option<Vec<u8>> {
    let jpeg = jpeg(img)?;
    let mut png = std::io::Cursor::new(Vec::new());
    if img.write_to(&mut png, image::ImageFormat::Png).is_err() {
        return Some(jpeg);
    }
    let png = png.into_inner();
    Some(if png.len() < jpeg.len() { png } else { jpeg })
}

fn fit(img: image::DynamicImage) -> image::DynamicImage {
    if img.width().max(img.height()) <= MAX_EDGE {
        return img;
    }
    img.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Triangle)
}

/// JPEG has no alpha: paint it on white, as the chat input does.
fn jpeg(img: &image::DynamicImage) -> Option<Vec<u8>> {
    let rgba = img.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (out, px) in rgb.pixels_mut().zip(rgba.pixels()) {
        let a = px[3] as u32;
        let on_white = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        *out = image::Rgb([on_white(px[0]), on_white(px[1]), on_white(px[2])]);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
    image::DynamicImage::ImageRgb8(rgb)
        .write_with_encoder(encoder)
        .ok()?;
    Some(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        // Noise, so the PNG is big and a JPEG is smaller.
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            let v = ((x * 7919 + y * 104_729) % 251) as u8;
            image::Rgba([v, v.wrapping_mul(3), v.wrapping_mul(7), 128])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_large_picture_is_scaled_to_1568_as_jpeg() {
        let r = ready(&png(2000, 1000)).unwrap();
        assert_eq!(Kind::sniff(&r.bytes), Some(Kind::Jpeg));
        assert_eq!((r.width, r.height), (2000, 1000), "the original's size");
        let back = image::load_from_memory(&r.bytes).unwrap();
        assert_eq!((back.width(), back.height()), (1568, 784));
    }

    /// A flat screenshot past the edge is still scaled, kept as PNG when
    /// that is smaller than JPEG.
    #[test]
    fn a_flat_wide_screenshot_is_scaled_even_when_small_on_disk() {
        let flat = image::RgbImage::from_pixel(2400, 1500, image::Rgb([240, 240, 240]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(flat)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        let r = ready(&out.into_inner()).unwrap();
        let back = image::load_from_memory(&r.bytes).unwrap();
        assert_eq!((back.width(), back.height()), (1568, 980));
        assert_eq!((r.width, r.height), (2400, 1500));
    }

    #[test]
    fn a_small_picture_goes_as_it_came() {
        let small = png(40, 30);
        let r = ready(&small).unwrap();
        assert_eq!(r.bytes, small);
        assert!(ready(b"\x89PNG\r\n\x1a\nbroken").is_none());
    }
}
