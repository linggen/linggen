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

use crate::util::LockExt;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub mod cloud;
pub mod cloud_save;
pub mod cloud_voice;

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
    read_local_account().ok().flatten()
}

/// Read local identity for an offline operation that must distinguish a
/// signed-out machine from an unreadable or damaged account file.
pub fn read_local_account() -> Result<Option<AccountConfig>> {
    let path = account_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content)
            .map(Some)
            .map_err(|_| anyhow!("invalid local account file")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context("cannot read local account file"),
    }
}

pub fn save_account(config: &AccountConfig) -> Result<()> {
    // A freshly saved key is a new fact; the site has not judged it yet.
    *TOKEN_REJECTED.lock_ok() = None;
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
    migrate_from(&path, || load_account(), save_account);
}

/// [`migrate_remote_toml`] from the old file at `path`, with the account
/// read by `load` and kept by `save`. The old file is removed only once its
/// token is written and read back: until then it may be the only copy.
fn migrate_from(
    path: &std::path::Path,
    load: impl Fn() -> Option<AccountConfig>,
    save: impl FnOnce(&AccountConfig) -> Result<()>,
) {
    if load().is_none() {
        if let Some(token) = std::fs::read_to_string(path).ok().and_then(|raw| {
            raw.parse::<toml::Table>()
                .ok()?
                .get("api_token")?
                .as_str()
                .map(|s| s.to_string())
        }) {
            let account = AccountConfig {
                api_token: token.clone(),
                user_id: None,
                user_name: None,
                avatar_url: None,
            };
            if let Err(e) = save(&account) {
                tracing::warn!("remote.toml kept: its token could not be saved into account.toml: {e}");
                return;
            }
            if load().is_none_or(|kept| kept.api_token != token) {
                tracing::warn!("remote.toml kept: account.toml does not read back its token");
                return;
            }
            tracing::info!("Adopted the remote-access token into account.toml");
        }
    }
    if std::fs::remove_file(path).is_ok() {
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
    cloud::forget_readings();
    cloud_voice::forget();
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenSource {
    Account,
}

/// Billing token resolution: the local `account.toml` only. A remote-access
/// link is never a billing fallback (see module docs).
/// DELETE /api/auth/token with this machine's own key: the site revokes that
/// one device row and nothing else. Silent on any failure — a sign-out must
/// not hang on the network, and a key that stays alive unused is harmless.
pub async fn revoke_own_token() {
    let Some((token, _)) = resolve_token() else {
        return;
    };
    let _ = http()
        .delete(format!("{}/api/auth/token", site_url()))
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await;
}

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
        user_name: me
            .get("display_name")
            .and_then(|v| v.as_str())
            .map(String::from),
        avatar_url: avatar,
    }
}

// --- Entitlement (short-TTL cache + disk snapshot for offline grace) ---

struct CachedEntitlement {
    /// The account it is about ([`account_key`]).
    account: String,
    fetched: Instant,
    value: serde_json::Value,
}

static ENT_CACHE: Mutex<Option<CachedEntitlement>> = Mutex::new(None);
const ENT_TTL: Duration = Duration::from_secs(60);

fn invalidate_entitlement_cache() {
    *ENT_CACHE.lock_ok() = None;
}

/// The key every cached account fact is kept under: the account's token,
/// hashed (the token itself is kept in account.toml alone). A fact fetched
/// for one account is never read for another, however late it arrives.
pub(crate) fn account_key(token: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(token.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// The cached entitlement of `account`, and whether it is still fresh.
fn entitlement_in_memory(account: &str) -> Option<(serde_json::Value, bool)> {
    let cache = ENT_CACHE.lock_ok();
    let c = cache.as_ref().filter(|c| c.account == account)?;
    Some((c.value.clone(), c.fetched.elapsed() < ENT_TTL))
}

fn keep_entitlement(account: &str, value: &serde_json::Value) {
    *ENT_CACHE.lock_ok() = Some(CachedEntitlement {
        account: account.to_string(),
        fetched: Instant::now(),
        value: value.clone(),
    });
}

/// The offline snapshot at `path`, when it is `account`'s.
fn snapshot_of(path: &std::path::Path, account: &str) -> Option<serde_json::Value> {
    let kept: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (kept["account"].as_str() == Some(account)).then(|| kept["value"].clone())
}

fn write_snapshot(path: &std::path::Path, account: &str, value: &serde_json::Value) {
    let kept = serde_json::json!({ "account": account, "value": value });
    let _ = std::fs::write(path, kept.to_string());
}

/// The token linggen.dev last refused with 401, if any. A key on disk is
/// presence, not liveness: the site can revoke it (the account page, a
/// device signing out) while the file stays. Every caller that hears a 401
/// records it here, and the auth pre-flight reads it, so the models page and
/// the fallback chain say "sign in" the minute the site does — not after a
/// failed turn (2026-09-09: the page said signed in for two hours on a dead
/// key). Cleared when a fetch succeeds or a new key is saved.
static TOKEN_REJECTED: Mutex<Option<String>> = Mutex::new(None);

/// linggen.dev answered 401 to [token].
pub fn note_token_rejected(token: &str) {
    let mut g = TOKEN_REJECTED.lock_ok();
    if g.as_deref() != Some(token) {
        tracing::warn!("linggen.dev refused the account key; sign in again to replace it");
        *g = Some(token.to_string());
    }
}

/// linggen.dev accepted [token]: any earlier refusal was of a key since replaced.
pub fn note_token_accepted(token: &str) {
    let mut g = TOKEN_REJECTED.lock_ok();
    if g.as_deref() == Some(token) {
        *g = None;
    }
}

/// Whether the key on disk is one the site has refused since it was saved.
pub fn token_rejected(token: &str) -> bool {
    TOKEN_REJECTED.lock_ok().as_deref() == Some(token)
}

async fn fetch_entitlement(token: &str) -> Result<serde_json::Value> {
    let resp = http()
        .get(format!("{}/api/entitlement", site_url()))
        .bearer_auth(token)
        .send()
        .await
        .context("connect to linggen.dev")?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        note_token_rejected(token);
    }
    if !resp.status().is_success() {
        bail!("entitlement failed: {}", resp.status());
    }
    note_token_accepted(token);
    Ok(resp.json().await?)
}

/// `{ apps, trial }` from linggen.dev, with a 60s memory cache. On fetch
/// failure the last known state is served (memory, then the disk snapshot),
/// flagged offline — consumers bound the grace via `current_period_end` /
/// `expires_at`. None = offline with nothing cached.
pub async fn entitlement_cached(token: &str) -> Option<(serde_json::Value, bool)> {
    let account = account_key(token);
    if let Some((value, true)) = entitlement_in_memory(&account) {
        return Some((value, false));
    }
    match fetch_entitlement(token).await {
        Ok(v) => {
            keep_entitlement(&account, &v);
            write_snapshot(&entitlement_snapshot_path(), &account, &v);
            Some((v, false))
        }
        Err(e) => {
            tracing::warn!("entitlement fetch failed, serving last known state: {e:#}");
            if let Some((value, _)) = entitlement_in_memory(&account) {
                return Some((value, true));
            }
            snapshot_of(&entitlement_snapshot_path(), &account).map(|v| (v, true))
        }
    }
}

/// Whether the signed-in account is, as far as this machine knows, entitled to
/// Linggen: the developer, or a subscriber — read from the cached entitlement
/// (any age, memory then the disk snapshot), with NO network call. Cloud voice
/// asks it on paths that must start nothing (a status read, a readiness
/// check); the entitlement is kept fresh by the account page and `/api/uses`.
/// A key the site has refused, or no key, is not entitled.
pub fn subscribed_known() -> bool {
    let Some((token, _)) = resolve_token() else { return false };
    if token_rejected(&token) {
        return false;
    }
    let account = account_key(&token);
    let known = entitlement_in_memory(&account)
        .map(|(value, _)| value)
        .or_else(|| snapshot_of(&entitlement_snapshot_path(), &account));
    known.is_some_and(|ent| entitles_linggen(&ent, unix_secs()))
}

/// The developer account (the site says so on the payload) or any entitling subscription.
fn entitles_linggen(ent: &serde_json::Value, now: i64) -> bool {
    ent.get("developer").and_then(|d| d.as_bool()).unwrap_or(false) || app_entitled(ent, "linggen", now)
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Make sure the entitlement has been asked for, so [`subscribed_known`] has
/// something to read: one cached fetch (60 s memory cache, offline snapshot),
/// given two seconds — a page waiting on a voice must not wait on a dead network.
pub async fn learn_entitlement() {
    if let Some((token, _)) = resolve_token() {
        let _ = tokio::time::timeout(Duration::from_secs(2), entitlement_cached(&token)).await;
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

#[cfg(test)]
mod account_cache_tests {
    use super::*;

    /// An entitlement kept for one account is never read for another — in
    /// memory or from the offline snapshot.
    #[test]
    fn a_cached_entitlement_is_read_only_for_its_account() {
        let _guard = cloud::TEST_LOCK.lock_ok();
        let (a, b) = (account_key("usr_cache_a"), account_key("usr_cache_b"));
        assert_ne!(a, b);
        let value = serde_json::json!({"apps": {"cfo": {"status": "active"}}});
        keep_entitlement(&a, &value);
        assert_eq!(entitlement_in_memory(&a), Some((value.clone(), true)));
        assert_eq!(entitlement_in_memory(&b), None, "another account's entitlement was served");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account-entitlement.json");
        write_snapshot(&path, &a, &value);
        assert_eq!(snapshot_of(&path, &a), Some(value));
        assert_eq!(snapshot_of(&path, &b), None, "another account's snapshot was served");
        invalidate_entitlement_cache();
    }
}

#[cfg(test)]
mod migrate_tests {
    use super::*;

    fn old_file(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("remote.toml");
        std::fs::write(&path, "api_token = \"usr_old\"\n").unwrap();
        path
    }

    /// The old file goes only once the token it holds is written and read
    /// back; a save that failed, or one that does not read back, keeps it.
    #[test]
    fn the_old_file_stays_until_its_token_is_kept() {
        let dir = tempfile::tempdir().unwrap();

        let path = old_file(dir.path());
        migrate_from(&path, || None, |_| Err(anyhow!("disk full")));
        assert!(path.exists(), "a failed save keeps the old file");

        migrate_from(&path, || None, |_| Ok(()));
        assert!(path.exists(), "a save that does not read back keeps the old file");

        let kept = Mutex::new(None::<AccountConfig>);
        migrate_from(
            &path,
            || kept.lock_ok().clone(),
            |a| {
                *kept.lock_ok() = Some(a.clone());
                Ok(())
            },
        );
        assert!(!path.exists(), "kept and read back: the old file goes");
        assert_eq!(kept.lock_ok().as_ref().unwrap().api_token, "usr_old");
    }
}

#[cfg(test)]
mod token_verdict_tests {
    use super::*;

    /// The site's verdict is remembered per key, so a refused key reads as
    /// dead until a fetch accepts it or a new key is saved.
    #[test]
    fn a_refused_key_is_dead_until_accepted_or_replaced() {
        let _guard = TEST_LOCK.lock_ok();
        *TOKEN_REJECTED.lock_ok() = None;
        assert!(!token_rejected("usr_a"));
        note_token_rejected("usr_a");
        assert!(token_rejected("usr_a"));
        assert!(
            !token_rejected("usr_b"),
            "another key is not judged by this one's refusal"
        );
        note_token_accepted("usr_b");
        assert!(
            token_rejected("usr_a"),
            "accepting a different key clears nothing"
        );
        note_token_accepted("usr_a");
        assert!(!token_rejected("usr_a"));
        note_token_rejected("usr_a");
        *TOKEN_REJECTED.lock_ok() = None; // what save_account does
        assert!(!token_rejected("usr_a"));
    }

    static TEST_LOCK: Mutex<()> = Mutex::new(());
}

#[cfg(test)]
mod voice_entitlement_tests {
    use super::*;

    /// Cloud voice is for the developer account and for any entitling subscription; a trial, or nothing, is not.
    #[test]
    fn voice_is_for_the_developer_and_subscribers_only() {
        let now = 1_800_000_000;
        let active = serde_json::json!({"developer": false, "apps": {"suite": {"status": "active", "current_period_end": null}}});
        let lapsed_paid_through = serde_json::json!({"apps": {"suite": {"status": "canceled", "current_period_end": now + 100}}});
        let lapsed = serde_json::json!({"apps": {"suite": {"status": "canceled", "current_period_end": now - 100}}});
        let trial = serde_json::json!({"apps": {}, "trial": {"linggen": {"active": true}}});
        let developer = serde_json::json!({"developer": true, "apps": {}});
        assert!(entitles_linggen(&active, now));
        assert!(entitles_linggen(&lapsed_paid_through, now), "paid through its period");
        assert!(entitles_linggen(&developer, now));
        assert!(!entitles_linggen(&lapsed, now));
        assert!(!entitles_linggen(&trial, now), "a trial does not cover voice");
        assert!(!entitles_linggen(&serde_json::json!({}), now));
    }
}
