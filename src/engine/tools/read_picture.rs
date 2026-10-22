//! `Read` on a picture file (doc/tool-spec.md): a PNG, JPEG, GIF, WebP or
//! HEIC — known by its first bytes, never only by its name — comes back as
//! a [`ToolResult::PictureFile`], readied like a person's pasted picture
//! (`pictures::for_model`), never as text. The engine numbers it into the
//! session and attaches it once when the model can see. A picture that is
//! too big, does not decode, or is a HEIC this machine cannot convert comes
//! back as one plain line of facts.

use super::ToolResult;
use crate::pictures::for_model;
use crate::pictures::kind::Kind;
use anyhow::Result;
use base64::Engine as _;
use std::io::Read as _;
use std::path::Path;

/// The largest picture file `Read` opens.
pub(super) const MAX_PICTURE_FILE: u64 = 20 * 1024 * 1024;

/// The picture at `path`, when its head says it is one; `None` reads it as
/// text.
pub(super) fn read_picture(rel: &str, path: &Path) -> Result<Option<ToolResult>> {
    let Some(kind) = sniff_file(path)? else {
        return Ok(None);
    };
    let size = std::fs::metadata(path)?.len();
    let what = Facts {
        rel,
        kind,
        size,
        dims: None,
    };
    if size > MAX_PICTURE_FILE {
        let what = Facts {
            dims: header_dims(path, kind),
            ..what
        };
        return Ok(Some(fact(&what, "too big to read (over 20 MB)")));
    }
    let bytes = match kind {
        Kind::Heic => match heic_as_jpeg(path) {
            Some(jpeg) => jpeg,
            None => {
                return Ok(Some(fact(
                    &what,
                    "HEIC cannot be converted on this machine",
                )))
            }
        },
        _ => std::fs::read(path)?,
    };
    let Some(ready) = for_model::ready(&bytes) else {
        return Ok(Some(fact(&what, "it does not decode")));
    };
    Ok(Some(ToolResult::PictureFile {
        path: rel.to_string(),
        name: file_name(rel),
        kind: kind.label().to_string(),
        width: ready.width,
        height: ready.height,
        size,
        base64: base64::engine::general_purpose::STANDARD.encode(&ready.bytes),
        seen: false,
        id: String::new(),
    }))
}

fn sniff_file(path: &Path) -> Result<Option<Kind>> {
    let mut head = Vec::with_capacity(16);
    std::fs::File::open(path)?.take(16).read_to_end(&mut head)?;
    Ok(Kind::sniff(&head))
}

/// Width and height from the header alone (no pixels decoded).
fn header_dims(path: &Path, kind: Kind) -> Option<(u32, u32)> {
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    image::ImageReader::with_format(file, kind.format()?)
        .into_dimensions()
        .ok()
}

/// A HEIC as JPEG, through macOS's `sips`; `None` elsewhere or on failure.
fn heic_as_jpeg(path: &Path) -> Option<Vec<u8>> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let out = dir.path().join("picture.jpg");
    let status = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg"])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    status.success().then(|| std::fs::read(&out).ok()).flatten()
}

struct Facts<'a> {
    rel: &'a str,
    kind: Kind,
    size: u64,
    dims: Option<(u32, u32)>,
}

/// One plain line: what the file is, and why it was not read.
fn fact(what: &Facts, why: &str) -> ToolResult {
    let mut parts = vec![what.kind.label().to_string()];
    if let Some((w, h)) = what.dims {
        parts.push(format!("{w}×{h}"));
    }
    parts.push(human_size(what.size));
    ToolResult::Success(format!(
        "{} is a picture ({}); {why}, so it was not read.",
        what.rel,
        parts.join(", ")
    ))
}

fn file_name(rel: &str) -> String {
    Path::new(rel)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(rel)
        .to_string()
}

/// `412 KB`, `3.4 MB`.
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB * KB {
        return format!("{} KB", (b / KB).ceil() as u64);
    }
    format!("{:.1} MB", b / (KB * KB))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([20, 90, 140]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_picture_is_known_by_its_bytes_not_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let disguised = dir.path().join("notes.dat");
        std::fs::write(&disguised, png(64, 48)).unwrap();
        match read_picture("notes.dat", &disguised).unwrap() {
            Some(ToolResult::PictureFile {
                kind,
                width,
                height,
                name,
                base64,
                ..
            }) => {
                assert_eq!((kind.as_str(), width, height), ("PNG", 64, 48));
                assert_eq!(name, "notes.dat");
                assert!(!base64.is_empty());
            }
            other => panic!("expected a picture, got {other:?}"),
        }
        let text = dir.path().join("logo.png");
        std::fs::write(&text, "not a picture at all").unwrap();
        assert!(read_picture("logo.png", &text).unwrap().is_none());
    }

    #[test]
    fn a_picture_that_cannot_be_read_is_one_line_of_facts() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken.png");
        std::fs::write(&broken, b"\x89PNG\r\n\x1a\n-- cut short --").unwrap();
        let Some(ToolResult::Success(line)) = read_picture("broken.png", &broken).unwrap() else {
            panic!("expected facts");
        };
        assert_eq!(
            line,
            "broken.png is a picture (PNG, 1 KB); it does not decode, so it was not read."
        );

        let huge = dir.path().join("huge.png");
        let mut bytes = png(300, 200);
        bytes.resize(MAX_PICTURE_FILE as usize + 1, 0);
        std::fs::write(&huge, bytes).unwrap();
        let Some(ToolResult::Success(line)) = read_picture("huge.png", &huge).unwrap() else {
            panic!("expected facts");
        };
        assert_eq!(
            line,
            "huge.png is a picture (PNG, 300×200, 20.0 MB); too big to read (over 20 MB), so it was not read."
        );
    }

    /// An iPhone photo opens on macOS, converted through `sips`.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_heic_opens_on_macos() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.png");
        std::fs::write(&src, png(80, 60)).unwrap();
        let heic = dir.path().join("photo.heic");
        let made = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "heic"])
            .arg(&src)
            .arg("--out")
            .arg(&heic)
            .output()
            .unwrap();
        if !made.status.success() {
            eprintln!("sips cannot write HEIC here; skipped");
            return;
        }
        match read_picture("photo.heic", &heic).unwrap() {
            Some(ToolResult::PictureFile {
                kind,
                width,
                height,
                base64,
                ..
            }) => {
                assert_eq!((kind.as_str(), width, height), ("HEIC", 80, 60));
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64)
                    .unwrap();
                assert_eq!(Kind::sniff(&bytes), Some(Kind::Jpeg), "sent as JPEG");
            }
            other => panic!("expected a picture, got {other:?}"),
        }
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(human_size(10), "1 KB");
        assert_eq!(human_size(412 * 1024), "412 KB");
        assert_eq!(human_size(3_565_158), "3.4 MB");
    }
}
