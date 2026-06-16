//! `ling account` — linggen.dev billing sign-in. Separate from `ling login`
//! (remote access): this writes `account.toml` and registers no instance.

use crate::account;
use anyhow::{bail, Result};

pub async fn run_login() -> Result<()> {
    println!("\n  🔑 Linggen Account Sign-in\n");
    if let Some(acc) = account::load_account() {
        let who = acc.user_name.map(|n| format!(" as {n}")).unwrap_or_default();
        println!("  Already signed in{who}.");
        println!("  Run `ling account logout` first to switch accounts.\n");
        return Ok(());
    }

    let token = match crate::cli::login::receive_token_via_callback().await? {
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

    println!("\n  ✅ Signed in{who}.");
    println!("  Token saved to ~/.linggen/account.toml\n");
    Ok(())
}

pub async fn run_logout() -> Result<()> {
    if account::delete_account()? {
        println!("  Signed out (account.toml removed).");
    } else {
        println!("  Not signed in.");
    }
    if crate::cli::login::load_remote_config().is_some() {
        println!("  Note: the remote access link (remote.toml) is still active");
        println!("  (transport only — it no longer covers billing). `ling logout` unlinks it.");
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
