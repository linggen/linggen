//! A session's thread on disk (`sessions/<id>/messages.jsonl`), read back.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Row {
    pub agent_id: String,
    pub from_id: String,
    pub content: String,
    #[serde(default)]
    pub is_observation: bool,
}

pub fn read(linggen_home: &Path, sid: &str) -> Vec<Row> {
    let path = linggen_home
        .join("sessions")
        .join(sid)
        .join("messages.jsonl");
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad row {l}: {e}")))
        .collect()
}

impl Row {
    /// The tool this row records a call of, when it is one of `agent`'s calls.
    pub fn call_by(&self, agent: &str) -> Option<String> {
        if !self.is_observation || self.from_id != agent {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(&self.content).ok()?;
        (v["type"] == "tool").then(|| v["tool"].as_str().unwrap_or("").to_string())
    }

    /// A line `agent` said (not a tool row).
    pub fn said_by(&self, agent: &str) -> Option<&str> {
        (!self.is_observation && self.from_id == agent).then_some(self.content.as_str())
    }
}

/// Tools `agent` called in `rows`.
pub fn calls_by(rows: &[Row], agent: &str) -> Vec<String> {
    rows.iter().filter_map(|r| r.call_by(agent)).collect()
}

/// `agent`'s last spoken line in `rows`.
pub fn last_said<'a>(rows: &'a [Row], agent: &str) -> Option<&'a str> {
    rows.iter().rev().find_map(|r| r.said_by(agent))
}
