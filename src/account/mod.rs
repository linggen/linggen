//! linggen.dev account layer — sign-in, entitlement, checkout, machine link.
//!
//! `~/.linggen/account.toml` (0600) holds the ONE account credential and is the
//! only place a token lives. Everything account-facing reads it: billing,
//! entitlement, the cloud LLM proxy, relay signaling, and rooms. Signing out
//! deletes the file, so cloud models and app gating switch off and the relay
//! goes quiet (BYOK keeps working).
//!
//! Machine identity is separate and is NOT a credential: `~/.linggen/instance_id`
//! is a stable address for this install, like a serial number. Registration with
//! linggen.dev is an idempotent upsert on that id, so the daemon simply asserts
//! it at boot and after sign-in — there is nothing to persist and nothing that
//! can go stale. See linggensite/doc/entitlement-spec.md.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEFAULT_SITE_URL: &str = "https://linggen.dev";

pub fn site_url() -> String {
    std::env::var("LINGGEN_SITE_URL").unwrap_or_else(|_| DEFAULT_SITE_URL.to_string())
}

fn account_path() -> PathBuf {
    crate::paths::linggen_home().join("account.toml")
}

fn entitlement_snapshot_path() -> PathBuf {
    crate::paths::linggen_home().join("account-entitlement.json")
}

fn instance_id_path() -> PathBuf {
    crate::paths::linggen_home().join("instance_id")
}

/// This install's stable address on linggen.dev. Not a secret — it identifies
/// which machine to signal, and survives sign-out, sign-in, and account
/// switches. Created once, on first use.
pub fn instance_id() -> Result<String> {
    let path = instance_id_path();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let id = existing.trim().to_string();
        if !id.is_empty() {
            return Ok(id);
        }
    }
    let id = format!(
        "inst-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .take(3)
            .collect::<Vec<_>>()
            .join("")
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create ~/.linggen")?;
    }
    std::fs::write(&path, &id).context("write instance_id")?;
    Ok(id)
}

/// How this machine appears on the dashboard — read fresh, never cached, so a
/// renamed machine corrects itself on the next registration.
pub fn instance_name() -> String {
    gethostname::gethostname().to_string_lossy().to_string()
}

/// Assert this machine's registration with linggen.dev. Idempotent upsert on
/// the instance id, so it is safe to call at boot and after every sign-in —
/// which is exactly what keeps "signed in" and "machine online" the same fact.
/// Returns the user id the server recognised, when it says.
pub async fn register_instance() -> Result<Option<String>> {
    let (token, _) = resolve_token().ok_or_else(|| anyhow!("not signed in"))?;
    let body = serde_json::json!({
        "instance_id": instance_id()?,
        "name": instance_name(),
    });
    let resp = http()
        .post(format!("{}/api/instances", site_url()))
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .context("connect to linggen.dev")?;
    if !resp.status().is_success() {
        bail!("instance registration failed: {}", resp.status());
    }
    let v: serde_json::Value = resp.json().await.unwrap_or_default();
    Ok(v.get("user_id")
        .and_then(|u| u.as_str())
        .map(|s| s.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountConfig {
    pub api_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

pub fn load_account() -> Option<AccountConfig> {
    let content = std::fs::read_to_string(account_path()).ok()?;
    toml::from_str(&content).ok()
}

pub fn save_account(config: &AccountConfig) -> Result<()> {
    let path = account_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create ~/.linggen")?;
    }
    let toml_str = toml::to_string_pretty(config)?;
    std::fs::write(&path, toml_str).context("write account.toml")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    invalidate_entitlement_cache();
    Ok(())
}

/// One-time migration off the old `remote.toml`, which carried a second copy
/// of the account token and drifted out of sync with `account.toml` (the copy
/// went stale, the relay 401'd, and the machine showed offline while the app
/// looked signed in). Adopt its token only when there is nothing signed in,
/// then delete the file — the machine link no longer needs storing.
pub fn migrate_remote_toml() {
    let path = crate::paths::linggen_home().join("remote.toml");
    if !path.exists() {
        return;
    }
    if load_account().is_none() {
        if let Some(token) = std::fs::read_to_string(&path).ok().and_then(|raw| {
            raw.parse::<toml::Table>()
                .ok()?
                .get("api_token")?
                .as_str()
                .map(|s| s.to_string())
        }) {
            let _ = save_account(&AccountConfig {
                api_token: token,
                user_id: None,
                user_name: None,
                avatar_url: None,
            });
            tracing::info!("Adopted the remote-access token into account.toml");
        }
    }
    if std::fs::remove_file(&path).is_ok() {
        tracing::info!("Removed remote.toml — the account token is the single credential now");
    }
}

/// Returns whether a config existed and was removed.
pub fn delete_account() -> Result<bool> {
    let path = account_path();
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&path).context("remove account.toml")?;
    let _ = std::fs::remove_file(entitlement_snapshot_path());
    invalidate_entitlement_cache();
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenSource {
    Account,
}

/// Billing token resolution: the local `account.toml` only. A remote-access
/// link is never a billing fallback (see module docs).
pub fn resolve_token() -> Option<(String, TokenSource)> {
    let acc = load_account()?;
    Some((acc.api_token, TokenSource::Account))
}

/// Display name from the signed-in account.
pub fn resolved_user_name() -> Option<String> {
    load_account().and_then(|a| a.user_name)
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

/// GET {site}/api/auth/me — display name + avatars for sign-in UX.
pub async fn fetch_me(token: &str) -> Result<serde_json::Value> {
    let resp = http()
        .get(format!("{}/api/auth/me", site_url()))
        .bearer_auth(token)
        .send()
        .await
        .context("connect to linggen.dev")?;
    if !resp.status().is_success() {
        bail!("auth/me failed: {}", resp.status());
    }
    Ok(resp.json().await?)
}

/// Build an AccountConfig from a fresh token + the /api/auth/me payload.
pub fn config_from_me(token: String, me: &serde_json::Value) -> AccountConfig {
    let avatar = ["github_avatar_url", "google_avatar_url"]
        .iter()
        .find_map(|k| me.get(*k).and_then(|v| v.as_str()))
        .map(String::from);
    AccountConfig {
        api_token: token,
        user_id: me.get("id").and_then(|v| v.as_str()).map(String::from),
        user_name: me.get("display_name").and_then(|v| v.as_str()).map(String::from),
        avatar_url: avatar,
    }
}

// --- Entitlement (short-TTL cache + disk snapshot for offline grace) ---

struct CachedEntitlement {
    fetched: Instant,
    value: serde_json::Value,
}

static ENT_CACHE: Mutex<Option<CachedEntitlement>> = Mutex::new(None);
const ENT_TTL: Duration = Duration::from_secs(60);

fn invalidate_entitlement_cache() {
    *ENT_CACHE.lock().unwrap() = None;
}

async fn fetch_entitlement(token: &str) -> Result<serde_json::Value> {
    let resp = http()
        .get(format!("{}/api/entitlement", site_url()))
        .bearer_auth(token)
        .send()
        .await
        .context("connect to linggen.dev")?;
    if !resp.status().is_success() {
        bail!("entitlement failed: {}", resp.status());
    }
    Ok(resp.json().await?)
}

/// `{ apps, trial }` from linggen.dev, with a 60s memory cache. On fetch
/// failure the last known state is served (memory, then the disk snapshot),
/// flagged offline — consumers bound the grace via `current_period_end` /
/// `expires_at`. None = offline with nothing cached.
pub async fn entitlement_cached(token: &str) -> Option<(serde_json::Value, bool)> {
    if let Some(c) = ENT_CACHE.lock().unwrap().as_ref() {
        if c.fetched.elapsed() < ENT_TTL {
            return Some((c.value.clone(), false));
        }
    }
    match fetch_entitlement(token).await {
        Ok(v) => {
            *ENT_CACHE.lock().unwrap() = Some(CachedEntitlement {
                fetched: Instant::now(),
                value: v.clone(),
            });
            let _ = std::fs::write(entitlement_snapshot_path(), v.to_string());
            Some((v, false))
        }
        Err(e) => {
            tracing::warn!("entitlement fetch failed, serving last known state: {e:#}");
            if let Some(c) = ENT_CACHE.lock().unwrap().as_ref() {
                return Some((c.value.clone(), true));
            }
            let disk = std::fs::read_to_string(entitlement_snapshot_path()).ok()?;
            serde_json::from_str(&disk).ok().map(|v| (v, true))
        }
    }
}

/// Whether the entitlement payload entitles `app` right now. Mirrors the
/// site's rowEntitles (active/trialing, else paid-through period end); the
/// site already expands `suite` into per-app keys. Any entitling app covers
/// the bare-engine 'linggen' bucket.
pub fn app_entitled(ent: &serde_json::Value, app: &str, now: i64) -> bool {
    let Some(apps) = ent.get("apps").and_then(|v| v.as_object()) else {
        return false;
    };
    let row_ok = |row: &serde_json::Value| {
        let status = row.get("status").and_then(|s| s.as_str()).unwrap_or("");
        if status == "active" || status == "trialing" {
            return true;
        }
        row.get("current_period_end")
            .and_then(|v| v.as_i64())
            .map(|end| end > now)
            .unwrap_or(false)
    };
    if app == "linggen" {
        return apps.values().any(row_ok);
    }
    apps.get(app).map(row_ok).unwrap_or(false)
}

/// POST {site}/api/checkout — returns the Stripe Checkout URL to open.
pub async fn create_checkout(token: &str, app: &str) -> Result<String> {
    let resp = http()
        .post(format!("{}/api/checkout", site_url()))
        .bearer_auth(token)
        .json(&serde_json::json!({ "app": app }))
        .send()
        .await
        .context("connect to linggen.dev")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    if !status.is_success() {
        let msg = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("checkout failed");
        bail!("{msg} ({status})");
    }
    body.get("url")
        .and_then(|u| u.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow!("checkout response missing url"))
}
