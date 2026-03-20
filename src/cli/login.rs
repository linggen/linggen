//! `ling login` — link this machine to a linggen.dev account for remote access.
//!
//! Flow:
//! 1. Open browser to linggen.dev/app/settings (user copies API token)
//! 2. User pastes token into terminal
//! 3. Verify token with linggen.dev API
//! 4. Save to ~/.linggen/remote.toml
//! 5. Generate instance_id if not exists

use anyhow::{bail, Context, Result};
use std::io::{self, Write};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEFAULT_RELAY_URL: &str = "https://linggen.dev";

/// Path to the remote config file.
fn remote_config_path() -> PathBuf {
    crate::paths::linggen_home().join("remote.toml")
}

/// Path to the instance ID file.
fn instance_id_path() -> PathBuf {
    crate::paths::linggen_home().join("instance_id")
}

/// Load existing remote config, if any.
pub fn load_remote_config() -> Option<RemoteConfig> {
    let path = remote_config_path();
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}

/// Get or generate a stable instance ID.
pub fn get_or_create_instance_id() -> Result<String> {
    let path = instance_id_path();
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim().to_string();
        if !id.is_empty() {
            return Ok(id);
        }
    }
    // Generate a new instance ID
    let id = format!(
        "inst-{}",
        uuid::Uuid::new_v4().to_string().split('-').take(3).collect::<Vec<_>>().join("")
    );
    std::fs::write(&path, &id).context("Failed to write instance_id")?;
    println!("  Generated instance ID: {id}");
    Ok(id)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RemoteConfig {
    pub relay_url: String,
    pub api_token: String,
    pub instance_name: String,
    pub instance_id: String,
}

/// Start a one-shot localhost HTTP server that receives the token via redirect.
async fn receive_token_via_callback() -> Result<Option<String>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let callback_url = format!("http://localhost:{port}/callback");

    // Open browser to the link page with callback
    let link_url = format!("{}/auth/link?callback={}", DEFAULT_RELAY_URL, urlencoding::encode(&callback_url));
    println!("  Opening browser for authentication...\n");

    if open::that(&link_url).is_err() {
        println!("  Could not open browser. Please visit:");
        println!("  {link_url}\n");
    }

    println!("  Waiting for authorization...");

    // Wait for the browser to redirect back with the token (timeout 120s)
    let result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);

        // Parse token from GET /callback?token=usr_xxx
        let token = request
            .lines()
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|path| url::Url::parse(&format!("http://localhost{path}")).ok())
            .and_then(|url| {
                url.query_pairs()
                    .find(|(k, _)| k == "token")
                    .map(|(_, v)| v.to_string())
            });

        // Send a response to close the browser tab
        let html = "<html><body><h2>✅ Authenticated!</h2><p>You can close this tab and return to the terminal.</p><script>window.close()</script></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        stream.write_all(response.as_bytes()).await?;
        stream.flush().await?;

        Ok::<_, anyhow::Error>(token)
    })
    .await;

    match result {
        Ok(Ok(token)) => Ok(token),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            println!("  Timed out waiting for browser callback.");
            Ok(None)
        }
    }
}

/// Fallback: manual token paste.
fn read_token_manually() -> Result<String> {
    let settings_url = format!("{}/app/settings", DEFAULT_RELAY_URL);
    println!("\n  Please visit: {settings_url}");
    println!("  Generate an API token and paste it below.\n");
    print!("  API token (usr_...): ");
    io::stdout().flush()?;
    let mut token = String::new();
    io::stdin().read_line(&mut token)?;
    Ok(token.trim().to_string())
}

pub async fn run() -> Result<()> {
    println!("\n  🌐 Linggen Remote Login\n");
    println!("  This links your machine to your linggen.dev account");
    println!("  for remote access via WebRTC.\n");

    // Try automated browser flow, fall back to manual paste
    let token = match receive_token_via_callback().await {
        Ok(Some(t)) if t.starts_with("usr_") => {
            println!("  ✓ Token received from browser.\n");
            t
        }
        _ => {
            let t = read_token_manually()?;
            if !t.starts_with("usr_") {
                bail!("Invalid token format. Expected token starting with 'usr_'.");
            }
            t
        }
    };

    // Step 3: Ask for instance name
    let hostname = gethostname::gethostname()
        .to_string_lossy()
        .to_string();
    print!("  Instance name [{}]: ", hostname);
    io::stdout().flush()?;
    let mut name = String::new();
    io::stdin().read_line(&mut name)?;
    let name = name.trim().to_string();
    let instance_name = if name.is_empty() { hostname } else { name };

    // Step 4: Get or create instance ID
    let instance_id = get_or_create_instance_id()?;

    // Step 5: Verify token by registering with the relay
    println!("\n  Verifying token and registering instance...");
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/instances", DEFAULT_RELAY_URL))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "instance_id": instance_id,
            "name": instance_name,
        }))
        .send()
        .await
        .context("Failed to connect to linggen.dev")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Registration failed ({status}): {body}");
    }

    // Step 6: Save config
    let config = RemoteConfig {
        relay_url: DEFAULT_RELAY_URL.to_string(),
        api_token: token,
        instance_name: instance_name.clone(),
        instance_id: instance_id.clone(),
    };

    let toml_str = toml::to_string_pretty(&config).context("Failed to serialize config")?;
    let path = remote_config_path();
    std::fs::write(&path, &toml_str).context("Failed to write remote.toml")?;

    // Set file permissions to 0o600 (owner read/write only) on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    println!("\n  ✅ Remote access configured!");
    println!("  Instance: {instance_name} ({instance_id})");
    println!("  Config saved to: {}", path.display());
    println!("\n  Your linggen instance will register with linggen.dev on next startup.");
    println!("  Access it from anywhere at: {}/app\n", DEFAULT_RELAY_URL);

    Ok(())
}

pub async fn run_logout() -> Result<()> {
    let path = remote_config_path();
    if path.exists() {
        std::fs::remove_file(&path).context("Failed to remove remote.toml")?;
        println!("  Remote access configuration removed.");
    } else {
        println!("  No remote configuration found.");
    }
    Ok(())
}

pub async fn run_status() -> Result<()> {
    match load_remote_config() {
        Some(config) => {
            println!("  Remote access: Configured");
            println!("  Relay: {}", config.relay_url);
            println!("  Instance: {} ({})", config.instance_name, config.instance_id);
            println!("  Token: {}...{}", &config.api_token[..8], &config.api_token[config.api_token.len()-4..]);
        }
        None => {
            println!("  Remote access: Not configured");
            println!("  Run `ling login` to set up remote access.");
        }
    }
    Ok(())
}
