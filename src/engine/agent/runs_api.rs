//! `AgentManager` methods for the run-record lifecycle: begin a run,
//! mark it finished, list/get records, cancel a run tree.
//!
//! Distinct from `engine::agent::runs`, which owns the underlying
//! `RunStore` data structure (in-memory, process-lifetime only).
//! This module sits on top of it: it wires the store into the rest
//! of `AgentManager` (cancellation set, working-place clearing,
//! activity timestamps, the StateUpdated event channel).
//!
//! Method bodies were moved verbatim from `agent/mod.rs`; the only
//! change is the surrounding `impl AgentManager` block.

use crate::engine::agent::{
    AgentEvent, AgentManager, AgentRunRecord, AgentRunStatus,
};
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;

impl AgentManager {
    pub async fn begin_agent_run(
        &self,
        project_root: &PathBuf,
        session_id: Option<&str>,
        agent_id: &str,
        parent_run_id: Option<String>,
        detail: Option<String>,
    ) -> Result<String> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let run_id = self.make_run_id(agent_id);
        let started_at = crate::util::now_ts_secs();
        let repo_path = project_root.to_string_lossy().to_string();

        let record = AgentRunRecord {
            run_id: run_id.clone(),
            repo_path: repo_path.clone(),
            session_id: session_id.unwrap_or("default").to_string(),
            agent_id: agent_id.to_string(),
            agent_kind: None,
            parent_run_id,
            status: AgentRunStatus::Running,
            detail,
            started_at,
            ended_at: None,
        };
        self.run_store.add_run(&record);
        // Remember the agent's current top-level session so agent_chat can later
        // deliver a message into the chat the user is actually using.
        if record.parent_run_id.is_none() {
            if let Some(sid) = session_id {
                self.record_latest_session(agent_id, sid, &repo_path);
            }
        }
        self.clear_working_place_for_agent(&repo_path, agent_id)
            .await;
        self.cancelled_runs.lock().await.remove(&run_id);
        tracing::info!(
            run_id = %run_id,
            agent_id = %agent_id,
            session_id = %record.session_id,
            parent_run_id = ?record.parent_run_id,
            "run/begin"
        );
        Ok(run_id)
    }

    pub async fn finish_agent_run(
        &self,
        run_id: &str,
        status: AgentRunStatus,
        detail: Option<String>,
    ) -> Result<()> {
        let ended_at = Some(crate::util::now_ts_secs());
        self.run_store.update_run(run_id, status, detail.clone(), ended_at);
        self.clear_working_place_for_run(run_id).await;
        let _ = self.events.send((AgentEvent::StateUpdated, None));
        self.cancelled_runs.lock().await.remove(run_id);
        let run_snapshot = self.run_store.get_run(run_id);
        if let Some(ref run) = run_snapshot {
            self.update_agent_activity(&run.repo_path, &run.agent_id).await;
        }
        self.run_store.remove_run(run_id);
        tracing::info!(
            run_id = %run_id,
            agent_id = run_snapshot.as_ref().map(|r| r.agent_id.as_str()).unwrap_or("?"),
            session_id = run_snapshot.as_ref().map(|r| r.session_id.as_str()).unwrap_or("?"),
            status = ?status,
            detail = ?detail,
            "run/finish"
        );
        Ok(())
    }

    pub async fn list_agent_runs(
        &self,
        _project_root: &PathBuf,
        session_id: Option<&str>,
    ) -> Result<Vec<AgentRunRecord>> {
        Ok(self.run_store.list_runs(session_id))
    }

    pub async fn get_agent_run(
        &self,
        run_id: &str,
        _project_root: Option<&str>,
    ) -> Result<Option<AgentRunRecord>> {
        Ok(self.run_store.get_run(run_id))
    }

    pub async fn is_run_cancelled(&self, run_id: &str) -> bool {
        self.cancelled_runs.lock().await.contains(run_id)
    }

    pub async fn cancel_run_tree(
        &self,
        run_id: &str,
    ) -> Result<Vec<AgentRunRecord>> {
        let mut stack = vec![run_id.to_string()];
        let mut seen = HashSet::new();
        let mut runs = Vec::new();

        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(run) = self.run_store.get_run(&id) else {
                continue;
            };
            for child in self.run_store.list_children(&id) {
                stack.push(child.run_id.clone());
            }
            runs.push(run);
        }

        let to_cancel: Vec<AgentRunRecord> = runs
            .into_iter()
            .filter(|run| run.status == AgentRunStatus::Running)
            .collect();

        let now = crate::util::now_ts_secs();
        {
            let mut cancelled = self.cancelled_runs.lock().await;
            for run in &to_cancel {
                cancelled.insert(run.run_id.clone());
            }
        }
        for run in &to_cancel {
            self.run_store.update_run(
                &run.run_id,
                AgentRunStatus::Cancelled,
                Some("cancelled by user".to_string()),
                Some(now),
            );
            self.clear_working_place_for_run(&run.run_id).await;
        }
        if !to_cancel.is_empty() {
            let _ = self.events.send((AgentEvent::StateUpdated, None));
        }

        Ok(to_cancel)
    }
}
