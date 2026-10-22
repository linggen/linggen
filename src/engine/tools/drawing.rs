//! `GenerateImage`: draw one picture (`crate::pictures`) and hand it back
//! as a [`ToolResult::Picture`], which the session numbers and keeps like a
//! screenshot. In a skill the skill also gets its own copy under
//! `data/pictures/`, for its page.

use super::builtin::GenerateImageArgs;
use super::{ToolResult, Tools};
use crate::pictures::{self, Ask, Source};
use anyhow::{anyhow, bail, Result};
use base64::Engine as _;
use std::path::{Path, PathBuf};

/// What the chat shows under the picture.
const CAPTION_CHARS: usize = 120;

impl Tools {
    pub(crate) async fn generate_image(&self, args: GenerateImageArgs) -> Result<ToolResult> {
        let source = Source::here().ok_or_else(|| {
            anyhow!("Nothing here can draw: sign in to ChatGPT in Settings → Models.")
        })?;
        let skill = self.active_skill().cloned();
        if skill.is_none() && self.session_id.is_none() {
            bail!("GenerateImage needs a conversation to keep the picture in.");
        }
        if source == Source::ThisMac {
            if let Some(line) = local_lane_not_ready()? {
                return Ok(ToolResult::Success(line));
            }
        }
        let ask = Ask {
            prompt: args.prompt.clone(),
            shape: pictures::Shape::parse(args.shape.as_deref())?,
            reference: reference_for(skill.as_ref(), args.reference.as_deref())?,
            seed: args.seed.unwrap_or_else(clock_seed),
        };
        let drawn = pictures::draw(source, ask).await?;
        let saved = match &skill {
            Some((name, dir)) => {
                Some(save_in_skill(name, dir, args.name.as_deref(), &drawn.png).await?)
            }
            None => None,
        };
        Ok(ToolResult::Picture {
            base64: base64::engine::general_purpose::STANDARD.encode(&drawn.png),
            caption: caption(&args.prompt),
            source: drawn.source.label().to_string(),
            seconds: drawn.seconds,
            seed: drawn.seed,
            saved,
            id: String::new(),
        })
    }
}

/// This Mac draws only once its lane is installed: the first call starts
/// the install and says so in one line.
fn local_lane_not_ready() -> Result<Option<String>> {
    if let Err(reason) = crate::runtime::lane_gate(&crate::runtime::PICTURES_LANE) {
        bail!("this Mac cannot draw pictures: {reason}");
    }
    if crate::runtime::pictures_ready() {
        return Ok(None);
    }
    crate::runtime::ensure_pictures_in_background();
    Ok(Some(
        "The picture model is being installed on this Mac (about 5 GB). Try again in a few minutes."
            .to_string(),
    ))
}

/// A reference picture is a file inside the active skill; outside a skill
/// there is none to name.
fn reference_for(
    skill: Option<&(String, PathBuf)>,
    reference: Option<&str>,
) -> Result<Option<PathBuf>> {
    let Some(reference) = reference.filter(|r| !r.trim().is_empty()) else {
        return Ok(None);
    };
    let Some((_, dir)) = skill else {
        bail!("A reference picture works only inside a skill.");
    };
    Ok(Some(pictures::reference_inside(dir, reference)?))
}

/// The skill's copy: `data/pictures/<name>.png`, as (url, path). Without a
/// name it gets a fresh one, so an older picture is never overwritten.
async fn save_in_skill(
    skill: &str,
    dir: &Path,
    name: Option<&str>,
    png: &[u8],
) -> Result<(String, String)> {
    let stem = match name.filter(|n| !n.trim().is_empty()) {
        Some(name) => pictures::safe_stem(name),
        None => format!("picture-{}", uuid::Uuid::new_v4().simple()),
    };
    let rel = format!("data/pictures/{stem}.png");
    let path = dir.join(&rel);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, png).await?;
    Ok((format!("/apps/{skill}/{rel}"), path.display().to_string()))
}

/// The prompt as a caption: whole words up to [`CAPTION_CHARS`], with `…`
/// when cut.
fn caption(prompt: &str) -> String {
    let prompt = prompt.trim();
    if prompt.chars().count() <= CAPTION_CHARS {
        return prompt.to_string();
    }
    let cut: String = prompt.chars().take(CAPTION_CHARS).collect();
    let words = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{}…", words.trim_end_matches([',', ';', ' ']))
}

fn clock_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() % 1_000_000)
        .unwrap_or(7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_prompt_is_cut_at_a_word() {
        assert_eq!(caption(" a coffee machine "), "a coffee machine");
        let long = format!("{} enamel, small glass cup", "word ".repeat(23));
        let cut = caption(&long);
        assert!(cut.ends_with("word…"), "{cut}");
        assert!(cut.chars().count() <= CAPTION_CHARS + 1);
    }

    #[test]
    fn a_reference_needs_a_skill() {
        assert!(reference_for(None, None).unwrap().is_none());
        assert!(reference_for(None, Some("  ")).unwrap().is_none());
        let err = reference_for(None, Some("plate.png"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("only inside a skill"), "{err}");
    }

    #[tokio::test]
    async fn a_skill_keeps_its_own_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (url, path) = save_in_skill("game", dir.path(), Some("fu zhu"), b"png")
            .await
            .unwrap();
        assert_eq!(url, "/apps/game/data/pictures/fu-zhu.png");
        assert_eq!(std::fs::read(path).unwrap(), b"png");
    }
}
