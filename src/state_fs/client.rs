//! `~/.linggen/client.json` — where THIS machine goes to find ling-mem (and
//! the engine), and with what token:
//!
//! ```json
//! { "ling": "http://…:9527", "ling_mem": "http://…:9528", "token": "…", "device_id": "…" }
//! ```
//!
//! One file, every writer: the engine's Settings page, `/linggen:config`
//! (`plugins/linggen/scripts/config.sh`), and the plugin hooks read it. It is
//! the authority for the ling-mem address; `[agent].ling_mem_url` in the toml
//! is read once, migrated here, and never written again.
//!
//! Keys this module does not know pass through untouched.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// The header a paired device presents to either daemon.
pub const DEVICE_HEADER: &str = "x-linggen-device";

fn path() -> PathBuf {
    crate::paths::linggen_home().join("client.json")
}

/// The file's object; empty when missing or unreadable.
pub fn read() -> Map<String, Value> {
    read_at(&path())
}

fn read_at(p: &Path) -> Map<String, Value> {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn get(key: &str) -> Option<String> {
    get_at(&path(), key)
}

fn get_at(p: &Path, key: &str) -> Option<String> {
    read_at(p)
        .get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

pub fn ling_mem() -> Option<String> {
    get("ling_mem")
}

pub fn token() -> Option<String> {
    get("token")
}

/// Change some keys, keep the rest; owner-only, temp file + rename.
pub fn update(f: impl FnOnce(&mut Map<String, Value>)) -> std::io::Result<()> {
    update_at(&path(), f)
}

fn update_at(p: &Path, f: impl FnOnce(&mut Map<String, Value>)) -> std::io::Result<()> {
    let mut obj = read_at(p);
    f(&mut obj);
    let bytes = serde_json::to_vec_pretty(&Value::Object(obj)).map_err(std::io::Error::other)?;
    crate::state_fs::devices::write_atomic(p, &bytes)?;
    restrict(p);
    Ok(())
}

#[cfg(unix)]
fn restrict(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_: &Path) {}

/// A stable id for this machine when it asks another to connect.
pub fn device_id() -> std::io::Result<String> {
    if let Some(id) = get("device_id") {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    update(|o| {
        o.insert("device_id".into(), Value::String(id.clone()));
    })?;
    Ok(id)
}

/// The ling-mem address, resolved once at config load. client.json wins; a
/// non-default `[agent].ling_mem_url` from an older toml is copied there the
/// first time (the migration), so the two never disagree again.
pub fn resolve_ling_mem(toml_value: &str, default: &str) -> String {
    resolve_ling_mem_at(&path(), toml_value, default)
}

fn resolve_ling_mem_at(p: &Path, toml_value: &str, default: &str) -> String {
    if let Some(url) = get_at(p, "ling_mem") {
        return url;
    }
    let from_toml = toml_value.trim().trim_end_matches('/');
    if from_toml.is_empty() || from_toml == default {
        return default.to_string();
    }
    if let Err(e) = update_at(p, |o| {
        o.insert("ling_mem".into(), Value::String(from_toml.to_string()));
    }) {
        tracing::warn!("could not move ling_mem_url into client.json: {e}");
    }
    from_toml.to_string()
}

/// Add the device token, when this machine has one, to a request bound for
/// ling-mem. Loopback ignores it; another machine requires it.
pub fn with_token(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match token() {
        Some(t) => req.header(DEVICE_HEADER, t),
        None => req,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = "http://127.0.0.1:9528";

    #[test]
    fn client_json_wins_and_other_keys_survive_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("client.json");
        std::fs::write(
            &p,
            r#"{"ling":"http://127.0.0.1:9527","ling_mem":"http://10.0.0.2:9528/","token":"t"}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_ling_mem_at(&p, "http://127.0.0.1:4100", DEFAULT),
            "http://10.0.0.2:9528"
        );
        update_at(&p, |o| {
            o.insert("token".into(), Value::String("u".into()));
        })
        .unwrap();
        let obj = read_at(&p);
        assert_eq!(obj["ling"], "http://127.0.0.1:9527");
        assert_eq!(obj["token"], "u");
    }

    /// The one-time move: a non-default toml address lands in client.json.
    #[test]
    fn a_custom_toml_address_moves_into_client_json_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("client.json");
        assert_eq!(
            resolve_ling_mem_at(&p, "http://127.0.0.1:4100", DEFAULT),
            "http://127.0.0.1:4100"
        );
        assert_eq!(
            get_at(&p, "ling_mem").as_deref(),
            Some("http://127.0.0.1:4100")
        );
    }

    #[test]
    fn a_default_toml_address_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("client.json");
        assert_eq!(resolve_ling_mem_at(&p, DEFAULT, DEFAULT), DEFAULT);
        assert!(!p.exists());
    }
}
