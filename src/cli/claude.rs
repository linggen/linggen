//! `ling claude` — Claude Code on Linggen's models. Points one `claude`
//! process at the engine's Anthropic gateway (`/anthropic`) through its
//! environment, then becomes that process. A plain `claude` launch keeps its
//! own login; nothing on disk changes.

use anyhow::{bail, Context, Result};
use std::os::unix::process::CommandExt;

/// Told to every session this command starts: where its model lives.
fn session_note(port: u16) -> String {
    format!(
        "You run on Linggen: every model call goes through the local engine's \
         Anthropic gateway at 127.0.0.1:{port}/anthropic, not Anthropic directly. \
         Stopping that engine stops you."
    )
}

/// The picker's model families, each with its `ANTHROPIC_DEFAULT_*` slot.
const FAMILIES: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];

pub async fn run(port: u16, args: &[String]) -> Result<()> {
    let base = format!("http://127.0.0.1:{port}/anthropic");
    let models = served_models(&base).await?;
    let family = |name: &str| pick(&models, |id| id.contains(name));
    let Some(main) = family("sonnet")
        .or_else(|| pick(&models, |id| !id.contains("haiku")))
        .or(models.first())
    else {
        bail!("Linggen has no Anthropic API-key model to serve. Add one in Settings → Models.");
    };
    let small = family("haiku").unwrap_or(main);
    println!("  Claude Code on Linggen — {main} (small: {small})");

    let mut cmd = std::process::Command::new("claude");
    // Only the families Linggen serves stay in `/model`; the rest are hidden
    // for this launch rather than redirected to some other model.
    let mut allowed = models.clone();
    for name in FAMILIES {
        let Some(id) = family(name) else { continue };
        cmd.env(
            format!("ANTHROPIC_DEFAULT_{}_MODEL", name.to_uppercase()),
            id,
        );
        allowed.push(name.to_string());
    }
    let settings = serde_json::json!({ "availableModels": allowed });
    let err = cmd
        .arg("--settings")
        .arg(settings.to_string())
        .arg("--append-system-prompt")
        .arg(session_note(port))
        .args(args)
        .env("ANTHROPIC_BASE_URL", &base)
        .env("ANTHROPIC_AUTH_TOKEN", "linggen")
        .env_remove("ANTHROPIC_API_KEY")
        .env("ANTHROPIC_MODEL", main)
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

#[cfg(test)]
mod tests {
    use super::session_note;

    #[test]
    fn the_note_names_the_configured_port_and_no_restart_recipe() {
        let note = session_note(9631);
        assert!(note.contains("127.0.0.1:9631/anthropic"));
        assert!(!note.contains("9527"));
        for recipe in ["/usr/local/bin", "Linggen.app", "mv", "nohup"] {
            assert!(!note.contains(recipe), "{recipe}");
        }
    }
}
