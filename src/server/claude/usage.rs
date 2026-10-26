//! The pinned SDK restores cumulative session costs on resume. Persist its
//! latest total, never sum result messages or individual host turns.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Deserialize, Serialize)]
pub(crate) struct Usage {
    pub estimated_cost_usd: f64,
    #[serde(skip_serializing)]
    session_id: String,
}
#[derive(Deserialize, Serialize)]
struct Snapshot {
    session_id: String,
    estimated_cost_usd: f64,
}

pub(crate) fn read(dir: &Path) -> Option<Usage> {
    let snapshot: Snapshot =
        serde_json::from_slice(&std::fs::read(dir.join("claude-usage.json")).ok()?).ok()?;
    Some(Usage {
        estimated_cost_usd: snapshot.estimated_cost_usd,
        session_id: snapshot.session_id,
    })
}
pub(crate) fn record(dir: &Path, event: &Value) -> Result<()> {
    let Some(cost) = event["cost"]
        .as_f64()
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
    else {
        return Ok(());
    };
    let Some(session_id) = event["session_id"].as_str().filter(|id| !id.is_empty()) else {
        return Ok(());
    };
    let previous = read(dir)
        .filter(|usage| usage.session_id == session_id)
        .map(|usage| usage.estimated_cost_usd)
        .unwrap_or(0.0);
    let snapshot = Snapshot {
        session_id: session_id.into(),
        estimated_cost_usd: previous.max(cost),
    };
    let path = dir.join("claude-usage.json");
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(&snapshot)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
pub(crate) fn clear(dir: &Path) -> Result<()> {
    match std::fs::remove_file(dir.join("claude-usage.json")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn resumed_cost_is_not_added_again_and_clear_resets() {
        let dir = tempfile::tempdir().unwrap();
        for cost in [0.12, 0.12, 0.15, 0.20, -1.0] {
            record(
                dir.path(),
                &json!({"session_id":"sdk-session", "cost":cost}),
            )
            .unwrap();
        }
        assert_eq!(read(dir.path()).unwrap().estimated_cost_usd, 0.20);
        record(
            dir.path(),
            &json!({"session_id":"new-session", "cost":0.03}),
        )
        .unwrap();
        assert_eq!(read(dir.path()).unwrap().estimated_cost_usd, 0.03);
        clear(dir.path()).unwrap();
        assert!(read(dir.path()).is_none());
        clear(dir.path()).unwrap();
    }
}
