//! `ling account` — linggen.dev sign-in from a terminal. Writes the one
//! account credential to `account.toml` and registers this machine; the web UI
//! sign-in does exactly the same thing, so the two are interchangeable.

use crate::account;
use anyhow::{bail, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Start a one-shot localhost HTTP server that receives the token via redirect.
/// The browser hands the token back here; it goes straight to account.toml.
pub(super) async fn receive_token_via_callback() -> Result<Option<String>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let callback_url = format!("http://localhost:{port}/callback");
    let csrf_state = uuid::Uuid::new_v4().to_string();

    // Open browser to the link page with callback + CSRF state
    let link_url = format!(
        "{}/auth/link?callback={}&state={}",
        crate::account::site_url(),
        urlencoding::encode(&callback_url),
        urlencoding::encode(&csrf_state),
    );
    println!("  Opening browser for authentication...\n");

    if open::that(&link_url).is_err() {
        println!("  Could not open browser. Please visit:");
        println!("  {link_url}\n");
    }

    println!("  Waiting for authorization...");

    // Wait for the browser to redirect back with the token (timeout 120s)
    let result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);

        // Parse token and state from GET /callback?token=usr_xxx&state=...
        let parsed_url = request
            .lines()
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|path| url::Url::parse(&format!("http://localhost{path}")).ok());

        // Verify CSRF state parameter
        let returned_state = parsed_url.as_ref().and_then(|url| {
            url.query_pairs()
                .find(|(k, _)| k == "state")
                .map(|(_, v)| v.to_string())
        });
        if returned_state.as_deref() != Some(&csrf_state) {
            // Send error page and reject
            let html = "<html><body><h2>❌ Authentication failed</h2><p>Security check failed (state mismatch). Please try again.</p></body></html>";
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(), html
            );
            let _ = stream.write_all(response.as_bytes()).await;
            return Ok(None);
        }

        let token = parsed_url.and_then(|url| {
            url.query_pairs()
                .find(|(k, _)| k == "token")
                .map(|(_, v)| v.to_string())
        });

        // Send a response to close the browser tab
        let html = "<html><body><h2>✅ Authenticated!</h2><p>You can close this tab and return to the terminal.</p><script>window.close()</script></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        // Write response but don't fail if browser closed early — we already have the token
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.flush().await;

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


pub async fn run_login() -> Result<()> {
    println!("\n  🔑 Linggen Account Sign-in\n");
    if let Some(acc) = account::load_account() {
        let who = acc.user_name.map(|n| format!(" as {n}")).unwrap_or_default();
        println!("  Already signed in{who}.");
        println!("  Run `ling account logout` first to switch accounts.\n");
        return Ok(());
    }

    let token = match receive_token_via_callback().await? {
        Some(t) if t.starts_with("usr_") => t,
        _ => bail!("Sign-in did not complete. Please try again."),
    };

    // Best-effort profile fetch; sign-in succeeds even if it fails.
    let me = account::fetch_me(&token).await.unwrap_or_default();
    let config = account::config_from_me(token, &me);
    let who = config
        .user_name
        .clone()
        .map(|n| format!(" as {n}"))
        .unwrap_or_default();
    account::save_account(&config)?;

    // Signed in ⇒ this machine is reachable. Same step the web UI runs.
    match account::register_instance().await {
        Ok(_) => println!("\n  ✅ Signed in{who} — this machine is linked."),
        Err(e) => println!("\n  ✅ Signed in{who}. (Machine link deferred: {e})"),
    }
    println!("  Saved to ~/.linggen/account.toml\n");
    Ok(())
}

pub async fn run_logout() -> Result<()> {
    if account::delete_account()? {
        println!("  Signed out (account.toml removed).");
    } else {
        println!("  Not signed in.");
    }
    Ok(())
}

pub async fn run_status() -> Result<()> {
    let Some((token, _source)) = account::resolve_token() else {
        println!("  Not signed in. Run `ling account login`.");
        return Ok(());
    };
    println!("  Signed in via account.toml");
    if let Some(name) = account::resolved_user_name() {
        println!("  User: {name}");
    }

    let Some((ent, offline)) = account::entitlement_cached(&token).await else {
        println!("  Entitlement: unavailable (offline, nothing cached)");
        return Ok(());
    };
    if offline {
        println!("  (offline — showing last known state)");
    }
    if let Some(apps) = ent.get("apps").and_then(|v| v.as_object()) {
        if apps.is_empty() {
            println!("  Subscriptions: none");
        }
        for (app, row) in apps {
            let status = row.get("status").and_then(|s| s.as_str()).unwrap_or("?");
            println!("  Subscription: {app} — {status}");
        }
    }
    if let Some(trials) = ent.get("trial").and_then(|v| v.as_object()) {
        for (app, t) in trials {
            let used = t.get("tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            let budget = t.get("budget").and_then(|v| v.as_i64()).unwrap_or(0);
            let active = t.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
            let state = if active { "available" } else { "used up" };
            println!("  Trial: {app} — {state} ({used}/{budget} tokens)");
        }
    }
    Ok(())
}
