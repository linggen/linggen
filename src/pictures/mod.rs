//! Drawing pictures (`GenerateImage`, doc/tool-spec.md). Two sources, in
//! order: ChatGPT's image generation on the person's ChatGPT sign-in
//! ([`chatgpt`]), else the local FLUX lane on a Mac that can carry it
//! ([`local`]). The tool is offered only when one of them can draw here.
//! Either way the caller gets PNG bytes and keeps them where they belong
//! (the session's pictures, or a skill's `data/pictures/`).

pub mod chatgpt;
pub mod for_model;
pub mod kind;
pub mod local;

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Who draws a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    ChatGpt,
    ThisMac,
}

impl Source {
    /// The source to use: ChatGPT when signed in, else this Mac when its
    /// lane passes, else none (the tool is not offered).
    pub fn pick(chatgpt_signed_in: bool, lane_ok: bool) -> Option<Self> {
        if chatgpt_signed_in {
            return Some(Source::ChatGpt);
        }
        lane_ok.then_some(Source::ThisMac)
    }

    /// The source on this machine, right now.
    pub fn here() -> Option<Self> {
        Self::pick(
            crate::provider::codex_auth::chatgpt_signed_in(),
            crate::runtime::lane_supported(&crate::runtime::PICTURES_LANE),
        )
    }

    /// How a result names it.
    pub fn label(self) -> &'static str {
        match self {
            Source::ChatGpt => "ChatGPT",
            Source::ThisMac => "this Mac",
        }
    }
}

/// True when something here can draw — the tool's offer gate.
pub fn offered() -> bool {
    Source::here().is_some()
}

/// What to draw.
pub struct Ask {
    pub prompt: String,
    pub shape: Shape,
    /// A picture whose pose and shape to keep.
    pub reference: Option<PathBuf>,
    pub seed: u64,
}

/// A drawn picture: PNG bytes and how it was made.
pub struct Drawn {
    pub png: Vec<u8>,
    pub source: Source,
    pub seconds: f64,
    /// The seed, when the source takes one (this Mac).
    pub seed: Option<u64>,
}

/// Draw one picture with `source`.
pub async fn draw(source: Source, ask: Ask) -> Result<Drawn> {
    match source {
        Source::ChatGpt => chatgpt::draw(&ask).await,
        Source::ThisMac => local::draw(ask).await,
    }
}

/// The shapes a caller may ask for. Each source maps them to its own sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Square,
    Landscape,
}

impl Shape {
    pub fn parse(s: Option<&str>) -> Result<Self> {
        match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("square") => Ok(Shape::Square),
            Some("landscape") | Some("wide") => Ok(Shape::Landscape),
            Some(other) => bail!("shape must be square or landscape, not {other:?}"),
        }
    }
}

/// A file stem safe for a URL and a folder: ASCII letters, digits, `-`,
/// `_`; anything else becomes `-`; empty falls back to `picture`.
pub fn safe_stem(name: &str) -> String {
    let mut stem = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            stem.push(c);
        } else if !stem.ends_with('-') {
            stem.push('-');
        }
    }
    let stem = stem.trim_matches('-').to_string();
    if stem.is_empty() {
        "picture".to_string()
    } else {
        stem.chars().take(64).collect()
    }
}

/// Resolve a reference picture the caller named: relative to `skill_dir`,
/// and it must stay inside it — the tool never reads outside the skill.
pub fn reference_inside(skill_dir: &Path, reference: &str) -> Result<PathBuf> {
    let candidate = skill_dir.join(reference.trim_start_matches('/'));
    let real = candidate
        .canonicalize()
        .with_context(|| format!("reference picture not found: {reference}"))?;
    let root = skill_dir.canonicalize().context("skill dir")?;
    if !real.starts_with(&root) {
        bail!("reference must be a file inside the skill's folder");
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatgpt_first_then_this_mac_else_nothing() {
        assert_eq!(Source::pick(true, true), Some(Source::ChatGpt));
        assert_eq!(Source::pick(true, false), Some(Source::ChatGpt));
        assert_eq!(Source::pick(false, true), Some(Source::ThisMac));
        assert_eq!(
            Source::pick(false, false),
            None,
            "nothing draws: not offered"
        );
    }

    #[test]
    fn shapes_parse() {
        assert_eq!(Shape::parse(None).unwrap(), Shape::Square);
        assert_eq!(Shape::parse(Some("wide")).unwrap(), Shape::Landscape);
        assert_eq!(Shape::parse(Some("Landscape")).unwrap(), Shape::Landscape);
        assert!(Shape::parse(Some("huge")).is_err());
    }

    #[test]
    fn stems_are_url_safe() {
        assert_eq!(safe_stem("夫諸 1597 / plate"), "1597-plate");
        assert_eq!(safe_stem("  "), "picture");
        assert_eq!(safe_stem("iron_sword-2"), "iron_sword-2");
    }

    #[test]
    fn reference_stays_inside_the_skill() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("plate.jpg"), b"x").unwrap();
        assert!(reference_inside(dir.path(), "plate.jpg").is_ok());
        assert!(reference_inside(dir.path(), "../plate.jpg").is_err());
        assert!(reference_inside(dir.path(), "missing.jpg").is_err());
    }
}
