//! `agent_run`'s `grants`: the caller's preset of folders for a new
//! delegated run (`doc/mcp-spec.md` § agent_* group). Each folder is kept
//! only when the caller's own level allows it; the run starts with what
//! survived, and the caller is told what was refused. Programs need no
//! grant: a coding run runs them where it may edit.

use std::path::Path;

use crate::engine::permission::PermissionMode;

/// The argument as the caller sends it.
#[derive(Debug, Default, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallerGrants {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
}

/// What survived, and what was refused with its reason.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sifted {
    pub granted: Granted,
    pub refused: Vec<Refused>,
}

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Granted {
    pub read: Vec<String>,
    pub write: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Refused {
    pub item: String,
    pub reason: String,
}

/// The session param that keeps a run's sifted grants for `agent_status`.
pub const GRANTS_PARAM: &str = "agent_run_grants";

/// Read the argument: `None` when absent.
pub fn parse(value: Option<&serde_json::Value>) -> Result<Option<CallerGrants>, String> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    if value.get("tools").is_some() {
        return Err(
            "grants.tools: a coding session needs no tool grants — a run with \
                    write_dir runs programs where it may edit; grants takes read and write"
                .into(),
        );
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|e| format!("grants: {e} — use {{read: [...], write: [...]}}"))
}

/// A continued session keeps the grants it was made with.
pub fn validate(session: Option<&str>, grants: Option<&CallerGrants>) -> Result<(), String> {
    if session.is_some() && grants.is_some() {
        return Err("grants cannot be used with session_id".into());
    }
    Ok(())
}

/// Keep each folder the caller's level allows. `level` is what the
/// caller's `write_dir` gets (`None`: no write run).
pub fn sift(asked: &CallerGrants, level: Option<PermissionMode>, own_home: &Path) -> Sifted {
    let mut out = Sifted::default();
    for dir in dedup(&asked.read) {
        match folder(dir, own_home) {
            Ok(path) => out.granted.read.push(path),
            Err(reason) => out.refuse(dir, reason),
        }
    }
    for dir in dedup(&asked.write) {
        let checked = match level {
            None => Err("a write needs write_dir with edit or admin".to_string()),
            Some(_) => folder(dir, own_home),
        };
        match checked {
            Ok(path) => out.granted.write.push(path),
            Err(reason) => out.refuse(dir, reason),
        }
    }
    out
}

impl Sifted {
    fn refuse(&mut self, item: &str, reason: String) {
        self.refused.push(Refused {
            item: item.to_string(),
            reason,
        });
    }

    /// The paths this preset adds, in the order they are written: reads
    /// before the run's own folder (a read on a parent, written later,
    /// would prune it), writes after, at `level`.
    pub fn paths(
        &self,
        level: Option<PermissionMode>,
    ) -> (Vec<(String, PermissionMode)>, Vec<(String, PermissionMode)>) {
        let reads = self
            .granted
            .read
            .iter()
            .map(|p| (p.clone(), PermissionMode::Read))
            .collect();
        let writes = level
            .map(|level| {
                self.granted
                    .write
                    .iter()
                    .map(|p| (p.clone(), level))
                    .collect()
            })
            .unwrap_or_default();
        (reads, writes)
    }
}

fn dedup(items: &[String]) -> Vec<&str> {
    let mut seen: Vec<&str> = Vec::new();
    for item in items.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if !seen.contains(&item) {
            seen.push(item);
        }
    }
    seen
}

/// A folder a preset may name: `write_dir`'s rules (an existing absolute
/// folder, never `/` or home), and never Linggen's own folder or one holding it.
fn folder(dir: &str, own_home: &Path) -> Result<String, String> {
    let path =
        super::mcp_agent::write_root(dir).map_err(|e| e.replacen("write_dir", "a folder", 1))?;
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (path_real, home_real) = (real(&path), real(own_home));
    if path_real.starts_with(&home_real) || home_real.starts_with(&path_real) {
        return Err("Linggen's own folder is never granted".into());
    }
    Ok(path.to_string_lossy().to_string())
}

/// The line the caller reads beside the run's reply.
pub fn report_line(sifted: &Sifted) -> String {
    format!(
        "\n\nPreset grants: {}",
        serde_json::to_string(sifted).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(read: &[&str], write: &[&str]) -> CallerGrants {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        CallerGrants {
            read: own(read),
            write: own(write),
        }
    }

    #[test]
    fn refused_items_are_listed_and_the_rest_is_kept() {
        let home = tempfile::tempdir().unwrap();
        let own_home = home.path().join(".linggen");
        std::fs::create_dir_all(own_home.join("config")).unwrap();
        let out = home.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let out = std::fs::canonicalize(out)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let sifted = sift(
            &asked(
                &[&out, "/no/such/dir", &out],
                &[&out, &own_home.join("config").to_string_lossy(), "/"],
            ),
            Some(PermissionMode::Edit),
            &own_home,
        );
        assert_eq!(sifted.granted.read, [out.clone()]);
        assert_eq!(sifted.granted.write, [out.clone()]);
        let refused: Vec<&str> = sifted.refused.iter().map(|r| r.item.as_str()).collect();
        assert_eq!(refused.len(), 3, "{:?}", sifted.refused);
        assert!(refused.contains(&"/no/such/dir") && refused.contains(&"/"));
        assert!(sifted.refused.iter().all(|r| !r.reason.is_empty()));
        // The write folder is granted at the caller's level, never above it.
        let (reads, writes) = sifted.paths(Some(PermissionMode::Edit));
        assert_eq!(reads, [(out.clone(), PermissionMode::Read)]);
        assert_eq!(writes, [(out, PermissionMode::Edit)]);
    }

    #[test]
    fn a_run_with_no_write_level_gets_no_write() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("d");
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.to_string_lossy().to_string();
        let sifted = sift(&asked(&[], &[&dir]), None, &home.path().join(".linggen"));
        assert!(sifted.granted.write.is_empty());
        assert_eq!(sifted.refused.len(), 1);
    }

    #[test]
    fn grants_with_session_id_is_an_error() {
        assert!(validate(Some("s"), Some(&CallerGrants::default())).is_err());
        assert!(validate(Some("s"), None).is_ok());
        assert!(validate(None, Some(&CallerGrants::default())).is_ok());
    }

    #[test]
    fn the_argument_is_read_or_refused_whole() {
        assert_eq!(parse(None).unwrap(), None);
        let v = serde_json::json!({"read": ["/w"]});
        assert_eq!(parse(Some(&v)).unwrap().unwrap().read, ["/w"]);
        assert!(parse(Some(&serde_json::json!({"admin": ["/"]}))).is_err());
        assert!(parse(Some(&serde_json::json!(["flutter"]))).is_err());
    }

    /// A coding run already runs programs where it may edit: a caller that
    /// still names tools is told so, plainly, and the run does not start.
    #[test]
    fn tool_grants_are_refused_with_a_plain_reason() {
        for v in [
            serde_json::json!({"tools": ["flutter"]}),
            serde_json::json!({"tools": [], "read": ["/w"]}),
        ] {
            let err = parse(Some(&v)).unwrap_err();
            assert!(
                err.contains("a coding session needs no tool grants"),
                "{err}"
            );
        }
    }
}
