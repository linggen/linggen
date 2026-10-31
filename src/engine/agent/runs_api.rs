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

use crate::engine::agent::{AgentEvent, AgentManager, AgentRunRecord, AgentRunStatus};
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A top run's life: the run lives, for the run sweeper, while its starter
/// holds this. Dropped with the starter's future, even when it is cancelled.
pub struct RunLife {
    _life: Arc<()>,
}

impl AgentManager {
    /// Tie `run_id`'s life to the returned token.
    pub fn run_life(&self, run_id: &str) -> RunLife {
        let life = Arc::new(());
        self.run_lives
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run_id.to_string(), Arc::downgrade(&life));
        RunLife { _life: life }
    }

    /// Something still holds the run's life.
    pub fn run_lives_on(&self, run_id: &str) -> bool {
        self.run_lives
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(run_id)
            .is_some_and(|life| life.strong_count() > 0)
    }

    /// Close `Running` rows nothing runs any more, with their mailboxes and
    /// lives; lives nothing holds are pruned. Returns the closed ids.
    pub fn reap_runs(&self, ids: &[String], now_secs: u64) -> Vec<String> {
        let reaped = self.run_store.reap(ids, now_secs);
        let mut queues = self.run_messages.lock().unwrap_or_else(|e| e.into_inner());
        let mut lives = self.run_lives.lock().unwrap_or_else(|e| e.into_inner());
        for id in &reaped {
            queues.remove(id);
            lives.remove(id);
        }
        lives.retain(|_, life| life.strong_count() > 0);
        reaped
    }

    pub async fn begin_agent_run(
        &self,
        project_root: &Path,
        session_id: Option<&str>,
        agent_id: &str,
        parent_run_id: Option<String>,
        detail: Option<String>,
    ) -> Result<String> {
        self.begin_delegated_run(project_root, session_id, agent_id, parent_run_id, detail, false).await
    }

    pub async fn begin_delegated_run(
        &self,
        project_root: &Path,
        session_id: Option<&str>,
        agent_id: &str,
        parent_run_id: Option<String>,
        detail: Option<String>,
        background: bool,
    ) -> Result<String> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf());
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
        // Serialize registration with cancellation's tree snapshot. A foreground
        // child cannot be registered after its parent was cancelled or retired.
        let cancelled = self.cancelled_runs.lock().await;
        if !background {
            if let Some(parent) = record.parent_run_id.as_deref() {
                if cancelled.contains(parent) || !self.run_store.get_run(parent)
                    .is_some_and(|run| run.status == AgentRunStatus::Running) {
                    return Err(crate::engine::tools::DelegationCancelled.into());
                }
            }
        }
        self.run_messages.lock().unwrap_or_else(|e| e.into_inner()).insert(run_id.clone(), Vec::new());
        self.run_store.add_run_with_cascade(&record, !background);
        drop(cancelled);
        // A new run is new page state, same as its finish below. Without this
        // the run only reached a page when something else happened to refresh
        // it — a new chat's first-message rename did, a follow-up turn did not
        // — so older chats never learned the run existed and showed no Stop.
        let _ = self.events.send((AgentEvent::StateUpdated, None));
        // Remember the agent's current top-level session so agent_chat can later
        // deliver a message into the chat the user is actually using.
        if record.parent_run_id.is_none() {
            if let Some(sid) = session_id {
                self.record_latest_session(agent_id, sid, &repo_path);
            }
        }
        self.clear_working_place_for_agent(&repo_path, agent_id)
            .await;
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
    ) -> Result<AgentRunStatus> {
        // Invariant: a run the user cancelled can never finish "Completed".
        // Some loop exits surface a cancel as a normal outcome (a checkpoint
        // returning AgentOutcome::None); the flag is the ground truth.
        let mut cancelled = self.cancelled_runs.lock().await;
        let status = if cancelled.contains(run_id) {
            AgentRunStatus::Cancelled
        } else {
            status
        };
        let ended_at = Some(crate::util::now_ts_secs());
        self.run_store
            .update_run(run_id, status, detail.clone(), ended_at);
        cancelled.remove(run_id);
        drop(cancelled);
        self.clear_working_place_for_run(run_id).await;
        let _ = self.events.send((AgentEvent::StateUpdated, None));
        let run_snapshot = self.run_store.get_run(run_id);
        if let Some(ref run) = run_snapshot {
            self.update_agent_activity(&run.repo_path, &run.agent_id)
                .await;
            // A finished top-level run retires its busy status here, at the
            // choke point. Chat and mission turns send Idle from their own
            // paths, but a run that ends anywhere else (agent_chat, watcher
            // heralds) used to leave its last status in `active_statuses`
            // forever — the session list kept spinning on a run that ended
            // days ago. A duplicate Idle is a no-op. Subagent runs are
            // excluded: they share `sid|agent_id` with their parent, and
            // clearing that key mid-run would blank the parent's live status.
            if run.parent_run_id.is_none() {
                let _ = self.events.send((
                    AgentEvent::AgentStatus {
                        agent_id: run.agent_id.clone(),
                        status: "idle".to_string(),
                        detail: None,
                        parent_id: None,
                        run_id: Some(run_id.to_string()),
                        parent_run_id: None,
                    },
                    Some(run.session_id.clone()),
                ));
            }
        }
        self.run_messages.lock().unwrap_or_else(|e| e.into_inner()).remove(run_id);
        self.run_lives.lock().unwrap_or_else(|e| e.into_inner()).remove(run_id);
        self.run_store.remove_run(run_id);
        tracing::info!(
            run_id = %run_id,
            agent_id = run_snapshot.as_ref().map(|r| r.agent_id.as_str()).unwrap_or("?"),
            session_id = run_snapshot.as_ref().map(|r| r.session_id.as_str()).unwrap_or("?"),
            status = ?status,
            detail = ?detail,
            "run/finish"
        );
        Ok(status)
    }

    /// Any top-level (user-visible) run currently executing. Cheap sync
    /// scan; the mission scheduler's quiet gate asks this before starting
    /// background model work.
    pub fn has_active_top_level_runs(&self) -> bool {
        self.run_store
            .list_runs(None)
            .iter()
            .any(|r| r.parent_run_id.is_none() && r.status == AgentRunStatus::Running)
    }

    pub async fn list_agent_runs(
        &self,
        _project_root: &PathBuf,
        session_id: Option<&str>,
    ) -> Result<Vec<AgentRunRecord>> {
        Ok(self.run_store.list_runs(session_id))
    }

    pub async fn is_run_cancelled(&self, run_id: &str) -> bool {
        self.cancelled_runs.lock().await.contains(run_id)
    }

    /// Deliver only between live direct parent and child runs in the same session.
    pub(crate) fn send_run_message(&self, session: &str, sender: &str, target: &str, message: &str) -> Result<()> {
        let mut queues = self.run_messages.lock().unwrap_or_else(|e| e.into_inner());
        let from = self.run_store.get_run(sender).ok_or_else(|| anyhow::anyhow!("Sender is not running"))?;
        let to = self.run_store.get_run(target).ok_or_else(|| anyhow::anyhow!("Recipient is not running"))?;
        if from.session_id != session || to.session_id != session ||
            from.status != AgentRunStatus::Running || to.status != AgentRunStatus::Running ||
            !(from.parent_run_id.as_deref() == Some(target) || to.parent_run_id.as_deref() == Some(sender)) {
            anyhow::bail!("Recipient must be a running direct parent or child in this session");
        }
        let queue = queues.get_mut(target).ok_or_else(|| anyhow::anyhow!("Recipient is not running"))?;
        queue.push(format!("Message from {} (run {}): {}", from.agent_id, sender, message));
        Ok(())
    }

    pub(crate) fn queue_run_notice(&self, run: &str, notice: String) -> bool {
        let mut queues = self.run_messages.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(queue) = queues.get_mut(run) {
            queue.push(notice);
            return true;
        }
        false
    }

    /// Atomically close an empty mailbox, or take pending messages and keep it open.
    pub(crate) fn close_or_drain_run_messages(&self, run: &str) -> Vec<String> {
        let mut queues = self.run_messages.lock().unwrap_or_else(|e| e.into_inner());
        match queues.get_mut(run) {
            Some(queue) if !queue.is_empty() => std::mem::take(queue),
            _ => { queues.remove(run); Vec::new() }
        }
    }

    pub(crate) fn drain_run_messages(&self, run: &str) -> Vec<String> {
        self.run_messages.lock().unwrap_or_else(|e| e.into_inner())
            .get_mut(run).map(std::mem::take).unwrap_or_default()
    }

    pub async fn cancel_run_tree(&self, run_id: &str) -> Result<Vec<AgentRunRecord>> {
        let mut cancelled = self.cancelled_runs.lock().await;
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

        let now = crate::util::now_ts_secs();
        let to_cancel: Vec<AgentRunRecord> = runs
            .into_iter()
            .filter(|run| self.run_store.get_run(&run.run_id)
                .is_some_and(|current| current.status == AgentRunStatus::Running))
            .collect();
        for run in &to_cancel {
            cancelled.insert(run.run_id.clone());
            self.run_store.update_run(
                &run.run_id,
                AgentRunStatus::Cancelled,
                Some("cancelled by user".to_string()),
                Some(now),
            );
        }
        drop(cancelled);
        for run in &to_cancel {
            self.clear_working_place_for_run(&run.run_id).await;
        }
        if !to_cancel.is_empty() {
            let _ = self.events.send((AgentEvent::StateUpdated, None));
        }

        Ok(to_cancel)
    }
}

#[cfg(test)]
mod message_tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_serializes_child_registration() {
        let manager = AgentManager::new(
            crate::config::Config::default(), None,
            Arc::new(crate::engine::test_registries::Empty), Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty), crate::engine::InterfaceMode::Web,
        ).0;
        let parent = manager.begin_agent_run(Path::new("."), Some("s"), "root", None, None).await.unwrap();
        let child = manager.begin_agent_run(Path::new("."), Some("s"), "child", Some(parent.clone()), None).await.unwrap();
        manager.cancel_run_tree(&parent).await.unwrap();
        assert!(manager.is_run_cancelled(&child).await);
        assert!(manager.begin_agent_run(Path::new("."), Some("s"), "late", Some(parent.clone()), None).await.is_err());
        assert!(manager.begin_delegated_run(Path::new("."), Some("s"), "background", Some(parent), None, true).await.is_ok());
    }

    #[tokio::test]
    async fn messages_require_live_direct_relatives_in_same_session() {
        let manager = AgentManager::new(
            crate::config::Config::default(), None,
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        ).0;
        let root = manager.begin_agent_run(Path::new("."), Some("s"), "root", None, None).await.unwrap();
        let child = manager.begin_agent_run(Path::new("."), Some("s"), "child", Some(root.clone()), None).await.unwrap();
        let sibling = manager.begin_agent_run(Path::new("."), Some("s"), "sibling", Some(root.clone()), None).await.unwrap();
        let grandchild = manager.begin_agent_run(Path::new("."), Some("s"), "grandchild", Some(child.clone()), None).await.unwrap();
        assert!(manager.send_run_message("s", &root, &child, "hello").is_ok());
        assert!(manager.send_run_message("s", &child, &root, "reply").is_ok());
        assert!(manager.send_run_message("s", &child, &sibling, "no").is_err());
        assert!(manager.send_run_message("s", &root, &grandchild, "no").is_err());
        assert!(manager.send_run_message("other", &root, &child, "no").is_err());
        assert!(manager.drain_run_messages(&child)[0].contains("hello"));
        assert!(manager.drain_run_messages(&child).is_empty());
        assert!(manager.send_run_message("s", &root, &child, "during model call").is_ok());
        assert!(manager.close_or_drain_run_messages(&child)[0].contains("during model call"));
        assert!(manager.close_or_drain_run_messages(&child).is_empty());
        assert!(manager.send_run_message("s", &root, &child, "after close").is_err());
        manager.finish_agent_run(&child, AgentRunStatus::Completed, None).await.unwrap();
        assert!(manager.send_run_message("s", &root, &child, "late").is_err());
    }

    /// A run lives while its life is held; reaping takes its row, mailbox
    /// and life together.
    #[tokio::test]
    async fn a_run_lives_while_held_and_reaping_clears_its_mailbox() {
        let manager = AgentManager::new(
            crate::config::Config::default(), None,
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        ).0;
        let run = manager.begin_agent_run(Path::new("."), Some("s"), "root", None, None).await.unwrap();
        assert!(!manager.run_lives_on(&run), "nothing holds it yet");
        let life = manager.run_life(&run);
        assert!(manager.run_lives_on(&run));
        drop(life);
        assert!(!manager.run_lives_on(&run));
        assert_eq!(manager.reap_runs(&[run.clone()], 0), [run.clone()]);
        assert!(!manager.run_messages.lock().unwrap().contains_key(&run));
        assert!(manager.run_lives.lock().unwrap().is_empty());
    }
}
