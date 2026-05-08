use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentRunRecord {
    pub run_id: String,
    pub repo_path: String,
    pub session_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub agent_kind: Option<String>,
    pub parent_run_id: Option<String>,
    pub status: AgentRunStatus,
    pub detail: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
}

/// In-memory run store. No file persistence — runs are ephemeral process records
/// that exist only while the server is running.
pub struct RunStore {
    runs: Mutex<HashMap<String, AgentRunRecord>>,
    /// Parent run_id → list of child run_ids, for cancel cascade.
    children: Mutex<HashMap<String, Vec<String>>>,
}

impl RunStore {
    pub fn new() -> Self {
        Self {
            runs: Mutex::new(HashMap::new()),
            children: Mutex::new(HashMap::new()),
        }
    }

    pub fn add_run(&self, record: &AgentRunRecord) {
        if let Some(ref parent_id) = record.parent_run_id {
            self.children
                .lock()
                .unwrap()
                .entry(parent_id.clone())
                .or_default()
                .push(record.run_id.clone());
        }
        self.runs
            .lock()
            .unwrap()
            .insert(record.run_id.clone(), record.clone());
    }

    pub fn update_run(
        &self,
        run_id: &str,
        status: AgentRunStatus,
        detail: Option<String>,
        ended_at: Option<u64>,
    ) {
        let mut runs = self.runs.lock().unwrap();
        if let Some(run) = runs.get_mut(run_id) {
            run.status = status;
            if detail.is_some() {
                run.detail = detail;
            }
            if ended_at.is_some() {
                run.ended_at = ended_at;
            }
        }
    }

    pub fn get_run(&self, run_id: &str) -> Option<AgentRunRecord> {
        self.runs.lock().unwrap().get(run_id).cloned()
    }

    pub fn remove_run(&self, run_id: &str) {
        let mut runs = self.runs.lock().unwrap();
        if let Some(run) = runs.remove(run_id) {
            // Also clean up the children index.
            if let Some(ref parent_id) = run.parent_run_id {
                let mut children = self.children.lock().unwrap();
                if let Some(siblings) = children.get_mut(parent_id) {
                    siblings.retain(|id| id != run_id);
                    if siblings.is_empty() {
                        children.remove(parent_id);
                    }
                }
            }
            // Remove any children entries for this run.
            self.children.lock().unwrap().remove(run_id);
        }
    }

    pub fn list_runs(&self, session_id: Option<&str>) -> Vec<AgentRunRecord> {
        let runs = self.runs.lock().unwrap();
        let mut result: Vec<AgentRunRecord> = runs
            .values()
            .filter(|r| {
                if let Some(sid) = session_id {
                    r.session_id == sid
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        result
    }

    pub fn list_children(&self, parent_run_id: &str) -> Vec<AgentRunRecord> {
        let child_ids = self
            .children
            .lock()
            .unwrap()
            .get(parent_run_id)
            .cloned()
            .unwrap_or_default();
        let runs = self.runs.lock().unwrap();
        let mut result: Vec<AgentRunRecord> = child_ids
            .iter()
            .filter_map(|id| runs.get(id).cloned())
            .collect();
        result.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_run(
        run_id: &str,
        session_id: &str,
        parent: Option<&str>,
        started_at: u64,
    ) -> AgentRunRecord {
        AgentRunRecord {
            run_id: run_id.into(),
            repo_path: "/tmp/p".into(),
            session_id: session_id.into(),
            agent_id: "ling".into(),
            agent_kind: None,
            parent_run_id: parent.map(|s| s.into()),
            status: AgentRunStatus::Running,
            detail: None,
            started_at,
            ended_at: None,
        }
    }

    #[test]
    fn test_add_and_get_run() {
        let store = RunStore::new();
        let run = make_run("r1", "s1", None, 1000);
        store.add_run(&run);

        let fetched = store.get_run("r1").unwrap();
        assert_eq!(fetched.status, AgentRunStatus::Running);
        assert_eq!(fetched.run_id, "r1");
    }

    #[test]
    fn test_update_run() {
        let store = RunStore::new();
        let run = make_run("r1", "s1", None, 1000);
        store.add_run(&run);

        store.update_run("r1", AgentRunStatus::Completed, Some("done".into()), Some(2000));
        let fetched = store.get_run("r1").unwrap();
        assert_eq!(fetched.status, AgentRunStatus::Completed);
        assert_eq!(fetched.detail.as_deref(), Some("done"));
        assert_eq!(fetched.ended_at, Some(2000));
    }

    #[test]
    fn test_list_runs() {
        let store = RunStore::new();
        store.add_run(&make_run("r1", "s1", None, 1000));
        store.add_run(&make_run("r2", "s1", None, 2000));
        store.add_run(&make_run("r3", "s2", None, 3000));

        let all = store.list_runs(None);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].run_id, "r3");

        let s1_runs = store.list_runs(Some("s1"));
        assert_eq!(s1_runs.len(), 2);

        let s2_runs = store.list_runs(Some("s2"));
        assert_eq!(s2_runs.len(), 1);

        let empty = store.list_runs(Some("nonexistent"));
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn test_list_children() {
        let store = RunStore::new();
        store.add_run(&make_run("parent", "s1", None, 1000));
        store.add_run(&make_run("child1", "s1", Some("parent"), 1001));
        store.add_run(&make_run("child2", "s1", Some("parent"), 1002));
        store.add_run(&make_run("other", "s1", None, 1003));

        let children = store.list_children("parent");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].run_id, "child1");
        assert_eq!(children[1].run_id, "child2");
    }

    #[test]
    fn test_get_nonexistent_run() {
        let store = RunStore::new();
        assert!(store.get_run("nonexistent").is_none());
    }

    #[test]
    fn test_remove_run() {
        let store = RunStore::new();
        store.add_run(&make_run("parent", "s1", None, 1000));
        store.add_run(&make_run("child1", "s1", Some("parent"), 1001));
        assert_eq!(store.list_children("parent").len(), 1);

        store.remove_run("child1");
        assert!(store.get_run("child1").is_none());
        assert_eq!(store.list_children("parent").len(), 0);
    }
}
