//! `ling claude` — Claude Code on Linggen's models. Points one `claude`
//! process at the engine's Anthropic gateway (`/anthropic`) through its
//! environment, then becomes that process. A plain `claude` launch keeps its
//! own login; nothing on disk changes.

use anyhow::{bail, Context, Result};
use std::os::unix::process::CommandExt;

pub async fn run(port: u16, args: &[String]) -> Result<()> {
    let base = format!("http://127.0.0.1:{port}/anthropic");
    let models = served_models(&base).await?;
    let Some(main) = pick(&models, |id| !id.contains("haiku")).or(models.first()) else {
        bail!("Linggen has no Anthropic API-key model to serve. Add one in Settings → Models.");
    };
    // Every family the picker shows lands on a served model: its own when
    // Linggen has one, else the main one — a built-in row never 404s.
    let family = |name: &str| pick(&models, |id| id.contains(name));
    let small = family("haiku").unwrap_or(main);
    let opus = family("opus").unwrap_or(main);
    let fable = family("fable").unwrap_or(opus);
    println!("  Claude Code on Linggen — {main} (small: {small})");

    let err = std::process::Command::new("claude")
        .args(args)
        .env("ANTHROPIC_BASE_URL", &base)
        .env("ANTHROPIC_AUTH_TOKEN", "linggen")
        .env_remove("ANTHROPIC_API_KEY")
        .env("ANTHROPIC_MODEL", main)
        .env("ANTHROPIC_DEFAULT_FABLE_MODEL", fable)
        .env("ANTHROPIC_DEFAULT_OPUS_MODEL", opus)
        .env("ANTHROPIC_DEFAULT_SONNET_MODEL", main)
        .env("ANTHROPIC_DEFAULT_HAIKU_MODEL", small)
        .env("ANTHROPIC_SMALL_FAST_MODEL", small)
        // `/model` lists every model the gateway serves.
        .env("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY", "1")
        .exec();
    Err(err).context("could not start `claude` — is Claude Code installed?")
}

/// The model ids the gateway serves; fails when the engine is down.
async fn served_models(base: &str) -> Result<Vec<String>> {
    let resp = reqwest::get(format!("{base}/v1/models"))
        .await
        .context("Linggen engine is not running — start it with `ling`")?;
    let body: serde_json::Value = resp.json().await?;
    let ids = body["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(ids)
}

fn pick<'a>(ids: &'a [String], want: impl Fn(&str) -> bool) -> Option<&'a String> {
    ids.iter().find(|id| want(id))
}
