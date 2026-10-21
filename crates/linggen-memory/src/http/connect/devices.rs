//! The files a decision lands in.
//!
//! - `<data_dir>/paired-devices.json` — the SAME list the engine's pairing
//!   writes and `gate.rs` reads. An allowed machine gets a row in the engine's
//!   schema (`id`, `name`, `secret`, `created_at`, `device_id`, `settings`), so
//!   the engine's Devices UI shows it and either daemon accepts its token.
//!   Rows are handled as raw JSON: fields this daemon does not know (a phone's
//!   `account`, `relay_grant`) pass through untouched.
//! - `<data_dir>/memory/connect-devices.json` — which of those rows this
//!   daemon allowed, and from where. Its list and Revoke cover only these; a
//!   phone the engine paired is the engine's to revoke.
//! - `<data_dir>/memory/connect-denied.json` — machines refused for good,
//!   by `device_id`, until someone un-denies them.
//!
//! Both daemons write `paired-devices.json`, so every write here is
//! read-modify-write onto a temp file + rename. A file that does not parse is
//! never written over.

use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Allowed {
    /// The row id in `paired-devices.json`.
    pub id: String,
    pub device_id: String,
    pub name: String,
    pub ip: IpAddr,
    pub allowed_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Denied {
    pub device_id: String,
    pub name: String,
    pub ip: IpAddr,
    pub denied_at: i64,
}

fn paired_path(data_dir: &Path) -> PathBuf {
    crate::http::gate::devices_path(data_dir)
}

fn allowed_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join("connect-devices.json")
}

fn denied_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join("connect-denied.json")
}

/// Missing = empty. Unparseable is an error, so nothing is saved over it.
fn read_list<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Vec<T>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Temp file beside `path`, owner-only, then rename: a reader sees the old
/// file or the new one, never half of either.
fn write_list<T: Serialize>(path: &Path, rows: &[T]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("list"),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(rows).map_err(io::Error::other)?,
    )?;
    restrict(&tmp)?;
    std::fs::rename(&tmp, path)
}

#[cfg(unix)]
fn restrict(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict(_: &Path) -> io::Result<()> {
    Ok(())
}

fn modify<T, R>(path: &Path, f: impl FnOnce(&mut Vec<T>) -> R) -> io::Result<R>
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let mut rows = read_list(path)?;
    let out = f(&mut rows);
    write_list(path, &rows)?;
    Ok(out)
}

/// A 48-hex secret, the engine's token format.
fn new_secret() -> String {
    let mut rng = rand::rng();
    (0..24)
        .map(|_| format!("{:02x}", rng.random::<u8>()))
        .collect()
}

fn row_device_id(row: &Value) -> Option<&str> {
    row.get("device_id").and_then(Value::as_str)
}

fn row_id(row: &Value) -> Option<&str> {
    row.get("id").and_then(Value::as_str)
}

/// Pair a machine: a fresh row (and token) in the shared file, replacing any
/// earlier row for the same `device_id`. Returns the token.
pub fn allow(data_dir: &Path, device_id: &str, name: &str, ip: IpAddr) -> io::Result<String> {
    let secret = new_secret();
    let now = chrono::Utc::now().timestamp();
    let row = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "name": name,
        "secret": secret,
        "created_at": now,
        "device_id": device_id,
        "settings": {},
    });
    let id = row["id"].as_str().unwrap_or_default().to_string();
    modify(&paired_path(data_dir), |rows: &mut Vec<Value>| {
        rows.retain(|r| row_device_id(r) != Some(device_id));
        rows.push(row);
    })?;
    let entry = Allowed {
        id,
        device_id: device_id.to_string(),
        name: name.to_string(),
        ip,
        allowed_at: now,
    };
    modify(&allowed_path(data_dir), |rows: &mut Vec<Allowed>| {
        rows.retain(|r| r.device_id != device_id);
        rows.push(entry);
    })?;
    Ok(secret)
}

/// Machines this daemon allowed whose row is still in the shared file.
pub fn list_allowed(data_dir: &Path) -> io::Result<Vec<Allowed>> {
    let paired: Vec<Value> = read_list(&paired_path(data_dir))?;
    let live = |id: &str| paired.iter().any(|r| row_id(r) == Some(id));
    let allowed: Vec<Allowed> = read_list(&allowed_path(data_dir))?;
    Ok(allowed.into_iter().filter(|a| live(&a.id)).collect())
}

/// Has this daemon allowed this machine before? (It is asking again, so it
/// lost its token — the console says so.)
pub fn is_known(data_dir: &Path, device_id: &str) -> bool {
    list_allowed(data_dir)
        .map(|rows| rows.iter().any(|a| a.device_id == device_id))
        .unwrap_or(false)
}

/// Revoke one machine this daemon allowed. False if it is not one of ours.
pub fn revoke(data_dir: &Path, id: &str) -> io::Result<bool> {
    let ours = read_list::<Allowed>(&allowed_path(data_dir))?
        .iter()
        .any(|a| a.id == id);
    if !ours {
        return Ok(false);
    }
    modify(&paired_path(data_dir), |rows: &mut Vec<Value>| {
        rows.retain(|r| row_id(r) != Some(id));
    })?;
    modify(&allowed_path(data_dir), |rows: &mut Vec<Allowed>| {
        rows.retain(|a| a.id != id);
    })?;
    Ok(true)
}

pub fn list_denied(data_dir: &Path) -> io::Result<Vec<Denied>> {
    read_list(&denied_path(data_dir))
}

pub fn is_denied(data_dir: &Path, device_id: &str) -> bool {
    list_denied(data_dir)
        .map(|rows| rows.iter().any(|d| d.device_id == device_id))
        .unwrap_or(false)
}

pub fn deny(data_dir: &Path, device_id: &str, name: &str, ip: IpAddr) -> io::Result<()> {
    let entry = Denied {
        device_id: device_id.to_string(),
        name: name.to_string(),
        ip,
        denied_at: chrono::Utc::now().timestamp(),
    };
    modify(&denied_path(data_dir), |rows: &mut Vec<Denied>| {
        rows.retain(|d| d.device_id != device_id);
        rows.push(entry);
    })
}

/// False if it was not on the list.
pub fn undeny(data_dir: &Path, device_id: &str) -> io::Result<bool> {
    modify(&denied_path(data_dir), |rows: &mut Vec<Denied>| {
        let before = rows.len();
        rows.retain(|d| d.device_id != device_id);
        rows.len() != before
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        "192.168.20.31".parse().unwrap()
    }

    fn paired(dir: &Path) -> Vec<Value> {
        read_list(&paired_path(dir)).unwrap()
    }

    /// The row the engine's `PairedDevice` reads, beside a phone's row whose
    /// extra fields must survive untouched.
    #[test]
    fn allow_writes_an_engine_row_and_keeps_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let phone = r#"[{"id":"p1","name":"Phone","secret":"aa","created_at":1,
            "account":{"id":"acc"},"device_id":"phone-1","settings":{"models":["x"]},
            "relay_grant":"g1"}]"#;
        std::fs::write(paired_path(dir.path()), phone).unwrap();

        let token = allow(dir.path(), "dev-1", "DS242", ip()).unwrap();
        assert_eq!(token.len(), 48);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));

        let rows = paired(dir.path());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["relay_grant"], "g1");
        assert_eq!(rows[0]["settings"]["models"][0], "x");
        let row = &rows[1];
        for key in [
            "id",
            "name",
            "secret",
            "created_at",
            "device_id",
            "settings",
        ] {
            assert!(row.get(key).is_some(), "missing {key}");
        }
        assert_eq!(row["secret"], token);
        assert!(row["created_at"].is_i64());
    }

    #[test]
    fn allowing_again_replaces_the_machines_row() {
        let dir = tempfile::tempdir().unwrap();
        let first = allow(dir.path(), "dev-1", "DS242", ip()).unwrap();
        let second = allow(dir.path(), "dev-1", "DS242", ip()).unwrap();
        assert_ne!(first, second);
        let rows = paired(dir.path());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["secret"], second);
        assert_eq!(list_allowed(dir.path()).unwrap().len(), 1);
        assert!(is_known(dir.path(), "dev-1"));
    }

    #[test]
    fn revoke_removes_only_rows_this_daemon_allowed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            paired_path(dir.path()),
            r#"[{"id":"p1","name":"Phone","secret":"aa","created_at":1}]"#,
        )
        .unwrap();
        allow(dir.path(), "dev-1", "DS242", ip()).unwrap();
        assert!(
            !revoke(dir.path(), "p1").unwrap(),
            "the engine's phone is not ours"
        );
        let id = list_allowed(dir.path()).unwrap()[0].id.clone();
        assert!(revoke(dir.path(), &id).unwrap());
        let rows = paired(dir.path());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "p1");
        assert!(list_allowed(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn an_unreadable_device_file_is_never_written_over() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(paired_path(dir.path()), "{ torn").unwrap();
        assert!(allow(dir.path(), "dev-1", "DS242", ip()).is_err());
        assert_eq!(
            std::fs::read_to_string(paired_path(dir.path())).unwrap(),
            "{ torn"
        );
    }

    #[test]
    fn deny_is_remembered_until_undone() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_denied(dir.path(), "dev-1"));
        deny(dir.path(), "dev-1", "DS242", ip()).unwrap();
        deny(dir.path(), "dev-1", "DS242", ip()).unwrap();
        assert!(is_denied(dir.path(), "dev-1"));
        assert_eq!(list_denied(dir.path()).unwrap().len(), 1);
        assert!(undeny(dir.path(), "dev-1").unwrap());
        assert!(!undeny(dir.path(), "dev-1").unwrap());
        assert!(!is_denied(dir.path(), "dev-1"));
    }
}
