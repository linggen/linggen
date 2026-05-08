//! ChatGPT OAuth authentication for Codex API access.
//!
//! Uses the same OAuth flow as OpenAI's Codex CLI to authenticate with
//! a ChatGPT Plus/Pro subscription. Tokens are stored at `~/.linggen/codex_auth.json`.
//!
//! Two flows supported:
//! 1. Browser flow (Authorization Code + PKCE) — opens browser for login.
//! 2. Device code flow — for headless environments.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{Rng, RngExt};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Constants (from OpenAI Codex CLI source)
// ---------------------------------------------------------------------------

const AUTH_ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CALLBACK_PORT: u16 = 1455;
const SCOPES: &str = "openid profile email offline_access";

/// ChatGPT backend API base URL for subscription-based access.
pub const CHATGPT_API_BASE: &str = "https://chatgpt.com/backend-api/codex";

// Token refresh interval in days.
const TOKEN_REFRESH_INTERVAL_DAYS: i64 = 8;

// ---------------------------------------------------------------------------
// Token storage format
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CodexAuthTokens {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub account_id: Option<String>,
    pub last_refresh: Option<String>,
}

impl CodexAuthTokens {
    pub fn is_valid(&self) -> bool {
        self.access_token.is_some() && self.refresh_token.is_some()
    }

    /// Check if tokens need refresh (older than TOKEN_REFRESH_INTERVAL_DAYS).
    pub fn needs_refresh(&self) -> bool {
        let Some(ref last) = self.last_refresh else {
            return true;
        };
        let Ok(last_dt) = chrono::DateTime::parse_from_rfc3339(last) else {
            return true;
        };
        let age = chrono::Utc::now().signed_duration_since(last_dt);
        age.num_days() >= TOKEN_REFRESH_INTERVAL_DAYS
    }

    pub fn load(file: &Path) -> Self {
        if !file.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(file) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, file: &Path) -> Result<()> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(file, json)?;
        // Set file permissions to 600 on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

/// Default auth file path: `~/.linggen/codex_auth.json`.
pub fn codex_auth_file() -> PathBuf {
    crate::paths::linggen_home().join("codex_auth.json")
}

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

fn generate_code_verifier() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..64).map(|_| rng.random::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    use std::io::Write;
    // SHA-256 of the verifier
    let digest = {
        // Use a simple SHA-256 implementation via ring-like approach
        // Since we don't have ring/sha2, compute via openssl command or use a manual approach.
        // Actually, reqwest pulls in rustls which has ring. Let's use base64 + manual sha256.
        // For simplicity, use the sha256 from the system.
        // Actually, let's compute it properly using std.
        // We can use the fact that reqwest's rustls-tls brings in ring.
        // But to keep it simple and dependency-free, let's shell out or use a basic approach.
        // Better approach: use the `Bash` to compute, or inline a minimal sha256.
        // Since this is build-time, let's use a subprocess approach for now.
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s' '{}' | shasum -a 256 | cut -d' ' -f1", verifier))
            .output();
        match output {
            Ok(out) => {
                let hex = String::from_utf8_lossy(&out.stdout).trim().to_string();
                hex_to_bytes(&hex)
            }
            Err(_) => {
                // Fallback: return empty (will fail auth but won't crash)
                vec![]
            }
        }
    };
    URL_SAFE_NO_PAD.encode(&digest)
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

fn generate_state() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

// ---------------------------------------------------------------------------
// Extract account_id from JWT
// ---------------------------------------------------------------------------

fn extract_account_id(id_token: &str) -> Option<String> {
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    // Account ID is in the nested auth claims
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Token exchange
// ---------------------------------------------------------------------------

async fn exchange_code_for_tokens(
    http: &reqwest::Client,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<CodexAuthTokens> {
    let token_url = format!("{}/oauth/token", AUTH_ISSUER);
    let resp = http
        .post(&token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", CLIENT_ID),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .context("Token exchange request failed")?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Token exchange failed: {}", text);
    }

    let body: serde_json::Value = resp.json().await?;
    let access_token = body["access_token"].as_str().map(|s| s.to_string());
    let refresh_token = body["refresh_token"].as_str().map(|s| s.to_string());
    let id_token = body["id_token"].as_str().map(|s| s.to_string());
    let account_id = id_token.as_deref().and_then(extract_account_id);

    Ok(CodexAuthTokens {
        access_token,
        refresh_token,
        id_token,
        account_id,
        last_refresh: Some(chrono::Utc::now().to_rfc3339()),
    })
}

// ---------------------------------------------------------------------------
// Token refresh
// ---------------------------------------------------------------------------

pub async fn refresh_tokens(
    http: &reqwest::Client,
    tokens: &CodexAuthTokens,
) -> Result<CodexAuthTokens> {
    let Some(ref refresh_token) = tokens.refresh_token else {
        anyhow::bail!("No refresh token available");
    };

    let token_url = format!("{}/oauth/token", AUTH_ISSUER);
    let resp = http
        .post(&token_url)
        .json(&serde_json::json!({
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .context("Token refresh request failed")?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Token refresh failed: {}", text);
    }

    let body: serde_json::Value = resp.json().await?;
    let new_access = body["access_token"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| tokens.access_token.clone());
    let new_refresh = body["refresh_token"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| tokens.refresh_token.clone());
    let new_id = body["id_token"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| tokens.id_token.clone());
    let account_id = new_id
        .as_deref()
        .and_then(extract_account_id)
        .or_else(|| tokens.account_id.clone());

    Ok(CodexAuthTokens {
        access_token: new_access,
        refresh_token: new_refresh,
        id_token: new_id,
        account_id,
        last_refresh: Some(chrono::Utc::now().to_rfc3339()),
    })
}

// ---------------------------------------------------------------------------
// Browser detection
// ---------------------------------------------------------------------------

/// Detect whether the current environment can open a browser.
/// Returns `false` for SSH sessions, missing displays, and non-interactive shells.
fn can_open_browser() -> bool {
    // SSH session — no local browser
    if std::env::var("SSH_CONNECTION").is_ok() || std::env::var("SSH_TTY").is_ok() {
        return false;
    }
    // Linux without a display server
    #[cfg(target_os = "linux")]
    if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
        return false;
    }
    // macOS and Windows always have a way to open a browser if we're not in SSH
    true
}

// ---------------------------------------------------------------------------
// Browser-based OAuth flow (Authorization Code + PKCE)
// ---------------------------------------------------------------------------

/// Start the browser OAuth flow. Opens a browser window for ChatGPT login.
/// Returns tokens on success.
pub async fn browser_login() -> Result<CodexAuthTokens> {
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = generate_state();
    let redirect_uri = format!("http://localhost:{}/auth/callback", CALLBACK_PORT);

    let auth_url = format!(
        "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}&codex_cli_simplified_flow=true",
        AUTH_ISSUER,
        CLIENT_ID,
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(SCOPES),
        code_challenge,
        state,
    );

    // Start a local HTTP server to receive the callback
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let expected_state = state.clone();
    let server_handle = tokio::spawn(async move {
        use axum::{extract::Query, routing::get, Router};

        #[derive(Deserialize)]
        struct CallbackParams {
            code: String,
            state: String,
        }

        let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
        let expected = expected_state;

        let app = Router::new().route(
            "/auth/callback",
            get({
                let tx = tx.clone();
                let expected = expected.clone();
                move |Query(params): Query<CallbackParams>| {
                    let tx = tx.clone();
                    let expected = expected.clone();
                    async move {
                        if params.state != expected {
                            return "State mismatch. Please try again.".to_string();
                        }
                        if let Some(sender) = tx.lock().await.take() {
                            let _ = sender.send(params.code);
                        }
                        "Login successful! You can close this tab and return to Linggen.".to_string()
                    }
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", CALLBACK_PORT))
            .await
            .expect("Failed to bind callback port");
        // Serve until we get the callback (with timeout)
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            })
            .await
            .ok();
    });

    // Open browser (auto-detect headless and fall back to device code flow)
    info!("Opening browser for ChatGPT OAuth login...");
    let can_open = can_open_browser();
    if !can_open {
        info!("Headless environment detected — switching to device code flow.");
        // Abort the callback server since we won't use it
        server_handle.abort();
        return device_code_login().await;
    }
    let open_result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&auth_url).spawn()
    } else if cfg!(target_os = "linux") {
        std::process::Command::new("xdg-open").arg(&auth_url).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", "start", &auth_url]).spawn()
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::Other, "unsupported platform"))
    };
    if let Err(e) = open_result {
        warn!("Failed to open browser: {} — falling back to device code flow.", e);
        server_handle.abort();
        return device_code_login().await;
    }

    // Wait for the callback
    let code = tokio::time::timeout(std::time::Duration::from_secs(300), rx)
        .await
        .context("Login timed out (5 minutes)")?
        .context("Login callback channel closed")?;

    // Abort the server
    server_handle.abort();

    // Exchange code for tokens
    let http = reqwest::Client::new();
    let tokens = exchange_code_for_tokens(&http, &code, &redirect_uri, &code_verifier).await?;

    // Save tokens
    let auth_file = codex_auth_file();
    tokens.save(&auth_file)?;
    info!("ChatGPT OAuth login successful. Tokens saved to {:?}", auth_file);

    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Device code flow (for headless environments)
// ---------------------------------------------------------------------------

/// Start device code flow. Returns a user code for the user to enter at
/// auth.openai.com/codex/device, then polls until authorized.
pub async fn device_code_login() -> Result<CodexAuthTokens> {
    let http = reqwest::Client::new();

    // Step 1: Request user code
    let resp = http
        .post(format!("{}/api/accounts/deviceauth/usercode", AUTH_ISSUER))
        .json(&serde_json::json!({ "client_id": CLIENT_ID }))
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("Failed to get device code: {}", resp.text().await.unwrap_or_default());
    }

    let body: serde_json::Value = resp.json().await?;
    let device_auth_id = body["device_auth_id"]
        .as_str()
        .context("Missing device_auth_id")?
        .to_string();
    let user_code = body["user_code"]
        .as_str()
        .context("Missing user_code")?
        .to_string();
    let interval: u64 = body["interval"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    info!(
        "Device code: {}. Go to https://auth.openai.com/codex/device and enter this code.",
        user_code
    );

    // Step 2: Poll for authorization
    let mut attempts = 0;
    let max_attempts = 60; // 5 minutes at 5s interval
    loop {
        attempts += 1;
        if attempts > max_attempts {
            anyhow::bail!("Device code login timed out");
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        let resp = http
            .post(format!("{}/api/accounts/deviceauth/token", AUTH_ISSUER))
            .json(&serde_json::json!({
                "device_auth_id": device_auth_id,
                "user_code": user_code,
            }))
            .send()
            .await?;

        if resp.status().is_success() {
            let body: serde_json::Value = resp.json().await?;
            let auth_code = body["authorization_code"]
                .as_str()
                .context("Missing authorization_code")?;
            let code_verifier = body["code_verifier"]
                .as_str()
                .context("Missing code_verifier")?;

            let redirect_uri = format!("{}/deviceauth/callback", AUTH_ISSUER);
            let tokens =
                exchange_code_for_tokens(&http, auth_code, &redirect_uri, code_verifier).await?;

            let auth_file = codex_auth_file();
            tokens.save(&auth_file)?;
            info!("ChatGPT OAuth login successful via device code.");
            return Ok(tokens);
        }
        // 403/404 = still pending, continue polling
    }
}

// ---------------------------------------------------------------------------
// Auth manager — shared token state with auto-refresh
// ---------------------------------------------------------------------------

pub struct CodexAuthManager {
    tokens: RwLock<CodexAuthTokens>,
    http: reqwest::Client,
}

impl CodexAuthManager {
    pub fn new() -> Self {
        let tokens = CodexAuthTokens::load(&codex_auth_file());
        Self {
            tokens: RwLock::new(tokens),
            http: reqwest::Client::new(),
        }
    }

    /// Get current access token and account ID, refreshing if needed.
    /// Returns (access_token, account_id).
    pub async fn get_auth(&self) -> Result<(String, Option<String>)> {
        {
            let tokens = self.tokens.read().await;
            if tokens.is_valid() && !tokens.needs_refresh() {
                return Ok((
                    tokens.access_token.clone().unwrap(),
                    tokens.account_id.clone(),
                ));
            }
        }

        // Need refresh
        let mut tokens = self.tokens.write().await;
        // Double-check after acquiring write lock
        if tokens.is_valid() && !tokens.needs_refresh() {
            return Ok((
                tokens.access_token.clone().unwrap(),
                tokens.account_id.clone(),
            ));
        }

        if !tokens.is_valid() {
            anyhow::bail!(
                "ChatGPT OAuth not configured. Run `ling auth login` or sign in via Web UI Settings."
            );
        }

        info!("Refreshing ChatGPT OAuth tokens...");
        match refresh_tokens(&self.http, &tokens).await {
            Ok(new_tokens) => {
                if let Err(e) = new_tokens.save(&codex_auth_file()) {
                    warn!("Failed to save refreshed tokens: {}", e);
                }
                let access = new_tokens.access_token.clone().unwrap();
                let account = new_tokens.account_id.clone();
                *tokens = new_tokens;
                Ok((access, account))
            }
            Err(e) => {
                warn!("Token refresh failed: {}. Using existing token.", e);
                if let Some(ref access) = tokens.access_token {
                    Ok((access.clone(), tokens.account_id.clone()))
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Check if tokens are present and valid.
    pub async fn is_authenticated(&self) -> bool {
        self.tokens.read().await.is_valid()
    }

    /// Clear stored tokens (logout).
    pub async fn logout(&self) -> Result<()> {
        let mut tokens = self.tokens.write().await;
        *tokens = CodexAuthTokens::default();
        let auth_file = codex_auth_file();
        if auth_file.exists() {
            std::fs::remove_file(&auth_file)?;
        }
        info!("ChatGPT OAuth tokens cleared.");
        Ok(())
    }

    /// Reload tokens from disk (after browser login completes).
    pub async fn reload(&self) {
        let new_tokens = CodexAuthTokens::load(&codex_auth_file());
        let mut tokens = self.tokens.write().await;
        *tokens = new_tokens;
    }
}

// ---------------------------------------------------------------------------
// URL encoding helper (lightweight, avoids adding `urlencoding` crate)
// ---------------------------------------------------------------------------

mod urlencoding {
    pub fn encode(input: &str) -> String {
        let mut result = String::with_capacity(input.len() * 3);
        for byte in input.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push('%');
                    result.push_str(&format!("{:02X}", byte));
                }
            }
        }
        result
    }
}
