//! Claude Code (CC Max) OAuth authentication.
//!
//! Reuses the OAuth tokens that the `claude` CLI stores, so Linggen can call
//! Anthropic's Messages API under the user's existing CC Max subscription
//! without a separate API key.
//!
//! Storage layout:
//! - macOS: Keychain (generic password), service = `Claude Code-credentials`.
//!   Value is a JSON blob: `{ "claudeAiOauth": { accessToken, refreshToken,
//!   expiresAt (ms), scopes, subscriptionType, rateLimitTier } }`.
//! - Linux / Windows: TODO — `claude` CLI uses OS-native keystores there too.
//!   Fall back to an env var for now.
//!
//! Refresh strategy: we do **not** refresh tokens ourselves. The `claude` CLI
//! keeps them refreshed in the keychain during normal use. We re-read on
//! every request, so whatever the CLI wrote most recently wins. If the token
//! is expired, we surface a clear error asking the user to run `claude` once
//! to trigger a refresh — that's less brittle than guessing at Anthropic's
//! OAuth token endpoint, which isn't publicly documented for the CC Max flow.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Anthropic Messages API base URL (CC Max OAuth goes against the public API).
pub const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";

/// API version header Anthropic requires on every request.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Beta flag that enables OAuth Bearer auth on the Messages endpoint.
/// Without this, Anthropic's public `/v1/messages` rejects OAuth tokens.
pub const ANTHROPIC_OAUTH_BETA: &str = "oauth-2025-04-20";

/// macOS keychain service name that the `claude` CLI writes to.
const KEYCHAIN_SERVICE_MACOS: &str = "Claude Code-credentials";

/// Env var fallback for non-macOS systems (or when keychain access is blocked).
/// Expected to contain just the OAuth access token (starts with `sk-ant-oat01-`).
const ENV_VAR_FALLBACK: &str = "LINGGEN_CLAUDE_OAUTH_TOKEN";

// ---------------------------------------------------------------------------
// Token shape — matches the JSON blob the `claude` CLI writes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAuthTokens {
    #[serde(rename = "accessToken")]
    pub access_token: String,
    #[serde(rename = "refreshToken", default)]
    pub refresh_token: Option<String>,
    /// Epoch milliseconds. 0 means "unknown" — treat as non-expiring.
    #[serde(rename = "expiresAt", default)]
    pub expires_at: i64,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(rename = "subscriptionType", default)]
    pub subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier", default)]
    pub rate_limit_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KeychainPayload {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: ClaudeAuthTokens,
}

impl ClaudeAuthTokens {
    /// Expired if the stored `expires_at` is in the past (with a 60-second
    /// safety margin so we refresh before the server rejects).
    pub fn is_expired(&self) -> bool {
        if self.expires_at == 0 {
            return false; // unknown → assume valid, let the server reject
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        self.expires_at <= now_ms + 60_000
    }

    /// True when the token carries the inference scope required for
    /// `/v1/messages`. Some future Claude Code variants may issue
    /// narrower scopes.
    pub fn can_do_inference(&self) -> bool {
        self.scopes.is_empty() || self.scopes.iter().any(|s| s == "user:inference")
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Read CC Max OAuth tokens from the OS-native store. Freshly re-read on
/// every call so the latest refresh by `claude` CLI is picked up without
/// Linggen restart.
pub fn load() -> Result<ClaudeAuthTokens> {
    // Env var shortcut for CI / headless setups.
    if let Ok(raw) = std::env::var(ENV_VAR_FALLBACK) {
        if !raw.trim().is_empty() {
            return Ok(ClaudeAuthTokens {
                access_token: raw.trim().to_string(),
                refresh_token: None,
                expires_at: 0,
                scopes: vec!["user:inference".to_string()],
                subscription_type: None,
                rate_limit_tier: None,
            });
        }
    }

    #[cfg(target_os = "macos")]
    {
        return load_from_macos_keychain();
    }
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!(
            "Claude Code OAuth tokens aren't wired up for this OS yet. \
             Set {} to an access token (sk-ant-oat01-...) as a workaround.",
            ENV_VAR_FALLBACK
        );
    }
}

#[cfg(target_os = "macos")]
fn load_from_macos_keychain() -> Result<ClaudeAuthTokens> {
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE_MACOS,
            "-w",
        ])
        .output()
        .context("Failed to run `security`. Is the macOS keychain accessible?")?;

    if !output.status.success() {
        anyhow::bail!(
            "Claude Code credentials not found in macOS keychain (service '{}'). \
             Sign in with the `claude` CLI first, or set the {} env var.",
            KEYCHAIN_SERVICE_MACOS,
            ENV_VAR_FALLBACK
        );
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let payload: KeychainPayload = serde_json::from_str(&raw)
        .context("Claude Code keychain entry exists but is not valid JSON")?;
    Ok(payload.claude_ai_oauth)
}
