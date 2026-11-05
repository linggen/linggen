//! Foreground subagents (`Task` without `run_in_background`): the turn waits
//! for them, unless the person steers a queued message in. Then each one
//! still running moves to the background as a task of the session — not
//! stopped, its end told as a new turn (doc/background-tasks-spec.md) — and
//! its call returns at once.
use super::types::AgentEngine;
use crate::engine::render::render_tool_result;
use crate::engine::tools::ToolResult;
use anyhow::Result;
use futures_util::future::select_all;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// How often the wait looks at Stop and Steer between subagent ends.
const POLL: Duration = Duration::from_millis(100);

/// One foreground subagent's call, while it runs.
pub(crate) struct ForegroundTask {
    pub idx: usize,
    pub target: String,
    pub task_text: String,
    pub handle: JoinHandle<Result<ToolResult>>,
    /// Its run id, once its run exists.
    pub started: oneshot::Receiver<String>,
}

/// A finished call: its index in the round, target, and result.
pub(crate) type TaskResult = (usize, String, Result<ToolResult>);

impl AgentEngine {
    /// Wait for every foreground subagent of the round. None when the run
    /// was stopped meanwhile.
    pub(crate) async fn await_foreground_tasks(
        &self,
        mut running: Vec<ForegroundTask>,
    ) -> Option<Vec<TaskResult>> {
        let mut results = Vec::new();
        while !running.is_empty() {
            if self.is_cancelled().await {
                return None;
            }
            if self.tools.builtins.steer_pending() {
                for task in running.drain(..) {
                    results.push(self.move_to_background(task).await);
                }
                break;
            }
            let ended = select_all(running.iter_mut().map(|task| &mut task.handle));
            let Ok((joined, at, _)) = tokio::time::timeout(POLL, ended).await else {
                continue;
            };
            let task = running.remove(at);
            let result =
                joined.unwrap_or_else(|e| Err(anyhow::anyhow!("delegation task panicked: {e}")));
            results.push((task.idx, task.target, result));
            if self.is_cancelled().await {
                return None;
            }
        }
        Some(results)
    }

    /// Steer: list the running subagent as a background task, detached from
    /// this turn's run so a later Stop leaves it be, and say so.
    async fn move_to_background(&self, task: ForegroundTask) -> TaskResult {
        let ForegroundTask {
            idx,
            target,
            task_text,
            handle,
            started,
        } = task;
        let (Some(manager), Some(session)) = (
            self.tools.get_manager(),
            self.tools.builtins.session_id.clone(),
        ) else {
            let result = handle.await.unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")));
            return (idx, target, result);
        };
        let caller = self.agent_id.clone().unwrap_or_else(|| "ling".into());
        let (relay, relayed) = oneshot::channel();
        let detaching = manager.clone();
        tokio::spawn(async move {
            if let Ok(run) = started.await {
                detaching.run_store.detach(&run);
                let _ = relay.send(run);
            }
        });
        let run = async move {
            match handle.await {
                Ok(Ok(result)) => Ok(render_tool_result(&result)),
                Ok(Err(error)) => Err(error),
                Err(join) => Err(anyhow::anyhow!("background subagent panicked: {join}")),
            }
        };
        let moved = crate::engine::background::track_subagent(
            &manager,
            &session,
            &caller,
            &target,
            Some(&caller),
            false,
            &task_text,
            relayed,
            run,
        )
        .await;
        let text = format!(
            "Moved to the background as task {}. You will be told in a new turn when it ends; \
             TaskOutput reads it, TaskStop stops it.",
            moved.id
        );
        (idx, target, Ok(ToolResult::Success(text)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::agent::AgentManager;
    use crate::engine::steer::SteerSignal;
    use std::path::Path;
    use std::sync::Arc;

    fn engine_with(manager: Arc<AgentManager>, root: &Path) -> AgentEngine {
        let cfg = crate::engine::EngineConfig::from_app_config(
            &crate::config::Config::default(),
            root.to_path_buf(),
            crate::engine::InterfaceMode::Web,
        );
        let models = Arc::new(crate::provider::models::ModelManager::new_with_credentials(
            Vec::new(),
            &crate::credentials::Credentials::default(),
        ));
        let mut engine =
            AgentEngine::new(cfg, models, "m".into(), crate::engine::AgentRole::Lead).unwrap();
        engine.agent_id = Some("ling".into());
        engine.tools.builtins.set_context(manager, "ling".into());
        engine.tools.builtins.set_session_id(Some("s".into()));
        engine
    }

    /// Steer moves a running foreground subagent to the background: its call
    /// returns at once with the task id, it keeps running as a listed task
    /// detached from the turn's run, and it ends there.
    #[tokio::test]
    async fn steer_hands_a_running_foreground_subagent_to_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let (manager, _events) = AgentManager::new(
            crate::config::Config::default(),
            None,
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        );
        let mut engine = engine_with(manager.clone(), dir.path());
        let steer = Arc::new(SteerSignal::default());
        engine.tools.builtins.set_steer(Some(steer.clone()));
        let turn = manager
            .begin_delegated_run(dir.path(), Some("s"), "ling", None, None, false)
            .await
            .unwrap();
        let child = manager
            .begin_delegated_run(
                dir.path(),
                Some("s"),
                "worker",
                Some(turn.clone()),
                None,
                false,
            )
            .await
            .unwrap();
        assert_eq!(manager.run_store.list_children(&turn).len(), 1);

        let (started_tx, started) = oneshot::channel();
        started_tx.send(child.clone()).unwrap();
        let (end_tx, end_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = end_rx.await;
            Ok(ToolResult::Success("worker done".into()))
        });
        let running = vec![ForegroundTask {
            idx: 0,
            target: "worker".into(),
            task_text: "dig".into(),
            handle,
            started,
        }];
        steer.fire();
        let results = tokio::time::timeout(
            Duration::from_secs(2),
            engine.await_foreground_tasks(running),
        )
        .await
        .unwrap()
        .unwrap();
        let ToolResult::Success(text) = results[0].2.as_ref().unwrap() else {
            panic!("expected the move told")
        };
        assert!(
            text.starts_with("Moved to the background as task ag-"),
            "{text}"
        );

        let tasks = manager.background.list_owned("s", Some("ling"));
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].is_running(), "the subagent was not stopped");
        tokio::time::timeout(Duration::from_secs(2), async {
            while !manager.run_store.list_children(&turn).is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached from the turn's run");

        end_tx.send(()).unwrap();
        let id = tasks[0].id.clone();
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.background.get("s", &id).unwrap().is_running() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            manager.background.get("s", &id).unwrap().status,
            "completed"
        );
    }

    /// Without a Steer the turn waits and gets the subagent's own answer.
    #[tokio::test]
    async fn an_unsteered_subagent_answers_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (manager, _events) = AgentManager::new(
            crate::config::Config::default(),
            None,
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        );
        let mut engine = engine_with(manager.clone(), dir.path());
        engine
            .tools
            .builtins
            .set_steer(Some(Arc::new(SteerSignal::default())));
        let (_tx, started) = oneshot::channel();
        let handle = tokio::spawn(async { Ok(ToolResult::Success("answer".into())) });
        let running = vec![ForegroundTask {
            idx: 0,
            target: "worker".into(),
            task_text: "x".into(),
            handle,
            started,
        }];
        let results = engine.await_foreground_tasks(running).await.unwrap();
        assert!(matches!(results[0].2.as_ref().unwrap(), ToolResult::Success(t) if t == "answer"));
        assert!(manager.background.list_owned("s", Some("ling")).is_empty());
    }
}
