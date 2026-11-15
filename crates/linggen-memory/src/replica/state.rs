//! `<data_dir>/memory/.replica.json` — this daemon's role and, as a replica,
//! what it needs to follow its primary. Owner-only: it holds a device token.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Primary,
    Replica,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReplicaFile {
    /// `host:port` of the primary; absent = this daemon IS the primary.
    #[serde(default)]
    pub primary: Option<String>,
    /// Issued by the primary when someone there clicked Allow.
    #[serde(default)]
    pub token: Option<String>,
    /// This machine's stable id when it asks a primary to connect.
    #[serde(default)]
    pub device_id: Option<String>,
    /// The last change sequence applied from the primary.
    #[serde(default)]
    pub cursor: u64,
    #[serde(default)]
    pub last_sync: Option<DateTime<Utc>>,
    /// Did the last call to the primary get an answer?
    #[serde(default)]
    pub online: bool,
    /// Why the last sync did not happen, in a few words.
    #[serde(default)]
    pub error: Option<String>,
    /// A connect request waiting for the click on the primary.
    #[serde(default)]
    pub request_id: Option<String>,
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join(".replica.json")
}

impl ReplicaFile {
    pub fn load(data_dir: &Path) -> Self {
        std::fs::read_to_string(path(data_dir))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let p = path(data_dir);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        restrict(&tmp);
        std::fs::rename(tmp, p)
    }

    pub fn role(&self) -> Role {
        match self.primary {
            Some(_) => Role::Replica,
            None => Role::Primary,
        }
    }

    pub fn upstream_url(&self) -> Option<String> {
        self.primary.as_deref().map(|p| format!("http://{p}"))
    }

    /// What the console, the engine and `/linggen:config` show — facts only,
    /// never the token.
    pub fn view(&self, queued: usize) -> serde_json::Value {
        serde_json::json!({
            "role": self.role(),
            "upstream": self.primary,
            "approved": self.token.is_some(),
            "waiting_for_approval": self.request_id.is_some(),
            "online": self.online,
            "last_sync": self.last_sync,
            "queued": queued,
            "error": self.error,
        })
    }
}

#[cfg(unix)]
fn restrict(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_: &Path) {}

/// `host:port` from what a person typed (`http://` and a trailing `/` allowed).
pub fn normalize_primary(raw: &str) -> Option<String> {
    let s = raw.trim().trim_end_matches('/');
    let s = s.strip_prefix("http://").unwrap_or(s);
    let (host, port) = s.rsplit_once(':')?;
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
    let port_ok = port.parse::<u16>().is_ok_and(|p| p > 0);
    (host_ok && port_ok).then(|| format!("{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_primary_means_this_is_the_primary() {
        let f = ReplicaFile::default();
        assert_eq!(f.role(), Role::Primary);
        let r = ReplicaFile {
            primary: Some("192.168.20.230:9528".into()),
            ..Default::default()
        };
        assert_eq!(r.role(), Role::Replica);
        assert_eq!(r.upstream_url().unwrap(), "http://192.168.20.230:9528");
    }

    #[test]
    fn the_view_never_carries_the_token() {
        let r = ReplicaFile {
            primary: Some("h:1".into()),
            token: Some("secret".into()),
            ..Default::default()
        };
        let v = r.view(2).to_string();
        assert!(!v.contains("secret"));
        assert!(v.contains("\"queued\":2"));
    }

    #[test]
    fn addresses_are_normalized() {
        assert_eq!(
            normalize_primary("192.168.20.230:9528").as_deref(),
            Some("192.168.20.230:9528")
        );
        assert_eq!(
            normalize_primary("http://mac.local:9528/").as_deref(),
            Some("mac.local:9528")
        );
        assert!(normalize_primary("192.168.20.230").is_none());
        assert!(normalize_primary("h:0").is_none());
        assert!(normalize_primary("a b:1").is_none());
    }

    #[test]
    fn the_file_round_trips_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let r = ReplicaFile {
            primary: Some("h:1".into()),
            cursor: 7,
            ..Default::default()
        };
        r.save(dir.path()).unwrap();
        assert_eq!(ReplicaFile::load(dir.path()).cursor, 7);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path(dir.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
