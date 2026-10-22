//! Drawing with ChatGPT: the Responses API's `image_generation` tool on the
//! person's ChatGPT sign-in, through the default built-in model (Luna, on
//! every plan). About 30 s a picture; the plan's picture allowance is
//! small, and a refusal is reported as it came.

use super::{Ask, Drawn, Shape, Source};
use crate::provider::codex_auth;
use crate::provider::openai::{OpenAiClient, PictureRequest};
use anyhow::{Context, Result};

/// Where the picture request goes. `LINGGEN_IMAGE_API_URL` points it at a
/// stand-in (the hermetic system tests); otherwise the ChatGPT backend.
fn api_base() -> String {
    std::env::var("LINGGEN_IMAGE_API_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| codex_auth::CHATGPT_API_BASE.to_string())
}

/// ChatGPT's sizes for each shape.
fn size(shape: Shape) -> &'static str {
    match shape {
        Shape::Square => "1024x1024",
        Shape::Landscape => "1536x1024",
    }
}

pub async fn draw(ask: &Ask) -> Result<Drawn> {
    let tokens = codex_auth::CodexAuthTokens::load(&codex_auth::codex_auth_file());
    let client = OpenAiClient::new_chatgpt_oauth(
        api_base(),
        tokens.access_token.unwrap_or_default(),
        tokens.account_id,
    );
    let reference = match &ask.reference {
        Some(path) => Some(read_reference(path).await?),
        None => None,
    };
    let started = std::time::Instant::now();
    let drawn = client
        .draw_picture(&PictureRequest {
            model: crate::provider::models::CHATGPT_BUILTIN_MODEL_ID,
            prompt: &ask.prompt,
            size: size(ask.shape),
            reference: reference.as_ref().map(|(b, m)| (b.as_slice(), *m)),
        })
        .await?;
    Ok(Drawn {
        png: drawn.png,
        source: Source::ChatGpt,
        seconds: started.elapsed().as_secs_f64(),
        seed: None,
    })
}

async fn read_reference(path: &std::path::Path) -> Result<(Vec<u8>, &'static str)> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read reference picture {}", path.display()))?;
    let mime = match path.extension().and_then(|e| e.to_str()) {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    };
    Ok((bytes, mime))
}
