//! A host still running an older Linggen plugin: it calls the retired
//! memory door (`/mcp/memory`, answered 410) or a retired tool name
//! (answered "update the plugin"), and its hooks swallow the answer, so the
//! person hears nothing while memory and Linggen's tools stay off there.
//!
//! The engine remembers each such call — which host, when — once per host
//! per day, and the person is told the way problems reach them: `ling
//! status` lists it, and it is a condition Yinyue raises
//! ([`super::conditions`]), said once, with the update commands for that
//! host. Nothing here can update a host's plugin, so the remedy is the
//! reason and where the person can act.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The problem's key: what a skill could declare offers under.
pub const OLD_PLUGIN: &str = "old-plugin";

/// How long a sighting stands for a host still on the old plugin: a host
/// that stopped calling for this long was updated (or closed).
const STANDS_FOR_SECS: i64 = 2 * 24 * 3600;

/// One host's last noted call by an old plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sighting {
    /// The local day it was noted on: at most one note per host per day.
    pub day: String,
    /// When, in seconds since the epoch.
    pub at: i64,
    /// What it called: the retired door or a retired tool name.
    pub called: String,
}

/// The hosts, by the name [`host_of`] gives them.
pub type Sightings = BTreeMap<String, Sighting>;

/// Which host a request came from, by its client's own name (the MCP
/// `clientInfo.name` or the User-Agent): `claude-code`, `codex`, or
/// `unknown` (an old hook calls with curl, which names no host).
pub fn host_of(client: Option<&str>) -> &'static str {
    let said = client.unwrap_or_default().to_ascii_lowercase();
    if said.contains("claude") {
        "claude-code"
    } else if said.contains("codex") {
        "codex"
    } else {
        "unknown"
    }
}

/// The host's name as the person knows it.
pub fn host_name(host: &str) -> &'static str {
    match host {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        _ => "A host on this Mac (Claude Code or Codex)",
    }
}

/// What the person runs to update the plugin in `host`, then restarts it —
/// said where they can act, since nothing on this Mac updates a host's
/// plugin by itself.
pub fn update_commands(host: &str) -> &'static str {
    match host {
        "claude-code" => "nothing on this Mac updates a host's plugin by itself; in a terminal: curl -fsSL https://linggen.dev/install-plugin.sh | bash, then restart Claude Code",
        "codex" => "nothing on this Mac updates a host's plugin by itself; in a terminal: curl -fsSL https://linggen.dev/install-plugin.sh | bash, then restart Codex",
        _ => "nothing on this Mac updates a host's plugin by itself; in a terminal: curl -fsSL https://linggen.dev/install-plugin.sh | bash, then restart Claude Code and Codex",
    }
}

/// A host called `called` with an old plugin: noted, unless it was noted
/// for this host today already.
pub fn note(host: &str, called: &str) {
    let path = path();
    let mut seen = load(&path);
    let now = crate::util::now_ts_secs() as i64;
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    if noted(&mut seen, host, called, now, &day) {
        store(&path, &seen);
    }
}

/// The hosts seen on an old plugin lately, oldest first.
pub fn recent() -> Vec<(String, Sighting)> {
    standing(&load(&path()), crate::util::now_ts_secs() as i64)
}

/// The note, without the disk: `false` when `host` was noted on `day`.
fn noted(seen: &mut Sightings, host: &str, called: &str, now: i64, day: &str) -> bool {
    if seen.get(host).is_some_and(|s| s.day == day) {
        return false;
    }
    let sighting = Sighting {
        day: day.to_string(),
        at: now,
        called: called.to_string(),
    };
    seen.insert(host.to_string(), sighting);
    true
}

/// The sightings that still stand at `now`, oldest first.
fn standing(seen: &Sightings, now: i64) -> Vec<(String, Sighting)> {
    let mut hosts: Vec<(String, Sighting)> = seen
        .iter()
        .filter(|(_, s)| now - s.at < STANDS_FOR_SECS)
        .map(|(h, s)| (h.clone(), s.clone()))
        .collect();
    hosts.sort_by_key(|(_, s)| s.at);
    hosts
}

/// What Yinyue is told, and the person's way to fix it, when a host stands
/// on an old plugin; `None` when none does.
pub fn situation(hosts: &[(String, Sighting)]) -> Option<(String, &'static str)> {
    let (host, last) = hosts.last()?;
    let names: Vec<&str> = hosts.iter().map(|(h, _)| host_name(h)).collect();
    let situation = format!(
        "{} on this Mac still runs an old Linggen plugin — it called the retired {} — so Linggen's memory and tools are off there until the plugin is updated",
        names.join(" and "),
        last.called
    );
    Some((situation, update_commands(host)))
}

fn path() -> PathBuf {
    super::activity::activity_dir().join("old-plugin.json")
}

fn load(path: &Path) -> Sightings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn store(path: &Path, seen: &Sightings) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(body) = serde_json::to_vec_pretty(seen) {
        let _ = std::fs::write(path, body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Once per host per day: a second call the same day changes nothing;
    /// the next day it is noted again, and another host on its own.
    #[test]
    fn a_host_is_noted_once_a_day() {
        let mut seen = Sightings::new();
        assert!(noted(&mut seen, "claude-code", "/mcp/memory", 100, "2026-11-18"));
        assert!(!noted(&mut seen, "claude-code", "memory_list", 200, "2026-11-18"));
        assert_eq!(seen["claude-code"].at, 100);
        assert!(noted(&mut seen, "codex", "memory_list", 300, "2026-11-18"));
        assert!(noted(&mut seen, "claude-code", "memory_list", 90_000, "2026-11-19"));
        assert_eq!(seen["claude-code"].called, "memory_list");
    }

    /// A host that stopped calling two days ago no longer stands.
    #[test]
    fn a_sighting_stands_for_two_days() {
        let mut seen = Sightings::new();
        noted(&mut seen, "codex", "/mcp/memory", 0, "2026-11-16");
        noted(&mut seen, "claude-code", "/mcp/memory", 100_000, "2026-11-17");
        let hosts = standing(&seen, STANDS_FOR_SECS + 10);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].0, "claude-code");
    }

    /// The host is told apart by its client's name, and its update is the
    /// one for that host.
    #[test]
    fn each_host_gets_its_own_update() {
        assert_eq!(host_of(Some("claude-code/2.1.4 (cli)")), "claude-code");
        assert_eq!(host_of(Some("codex_cli_rs/0.167.0")), "codex");
        assert_eq!(host_of(Some("curl/8.7.1")), "unknown");
        let hosts = vec![(
            "codex".to_string(),
            Sighting {
                day: "2026-11-18".into(),
                at: 1,
                called: "memory_list".into(),
            },
        )];
        let (said, fix) = situation(&hosts).unwrap();
        assert!(said.starts_with("Codex on this Mac"), "{said}");
        assert!(fix.contains("install-plugin.sh") && fix.ends_with("restart Codex"), "{fix}");
        assert!(situation(&[]).is_none());
    }
}
