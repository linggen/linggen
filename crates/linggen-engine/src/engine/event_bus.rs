//! The event bus every surface and watcher listens on, and the seam where an
//! event is stamped. Each event is stamped as it is sent — when it is made —
//! with the app its words are kept to (`turn_place::sight`): the stamp of
//! the run that made it (decided as that run began, so a subagent or a run
//! still streaming after its turn ended keeps it), else that of the turn
//! running in its session, else its own (a background task's entry). A
//! listener judges the stamp it is handed, never the state at the moment it
//! reads the event.

use super::events::ServerEvent;
use super::turn_place::running::RunningPlaces;
use tokio::sync::broadcast;

/// An event as it was made.
#[derive(Debug, Clone)]
pub struct Stamped {
    pub event: ServerEvent,
    /// The app the event's words are kept to; `None`: everyone's.
    pub kept_to: Option<String>,
}

/// The broadcast every event goes out on, stamped as it is sent.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Stamped>,
    places: RunningPlaces,
}

impl EventBus {
    /// A bus holding `capacity` events per listener, stamping from `places`
    /// (the session store's running places).
    pub fn new(capacity: usize, places: RunningPlaces) -> Self {
        Self {
            tx: broadcast::channel(capacity).0,
            places,
        }
    }

    /// A bus of its own, for tests and for a part with no session store.
    pub fn detached(capacity: usize) -> Self {
        Self::new(capacity, RunningPlaces::default())
    }

    /// Send `event`, stamped from whoever made it.
    pub fn send(&self, event: ServerEvent) -> Result<usize, broadcast::error::SendError<Stamped>> {
        let kept_to = self.stamp(&event);
        self.tx.send(Stamped { event, kept_to })
    }

    /// Send `event` stamped as `kept_to` — for an event made outside any turn
    /// that stands for one place's talk (a plan acted on later).
    pub fn send_kept(
        &self,
        event: ServerEvent,
        kept_to: Option<String>,
    ) -> Result<usize, broadcast::error::SendError<Stamped>> {
        self.tx.send(Stamped { event, kept_to })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Stamped> {
        self.tx.subscribe()
    }

    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// The stamp `event` takes now.
    fn stamp(&self, event: &ServerEvent) -> Option<String> {
        if let ServerEvent::BackgroundTask { task, .. } = event {
            return task.kept_to.clone();
        }
        let (session, runs) = origin(event);
        for run in runs.into_iter().flatten() {
            if let Some(stamp) = self.places.run_stamp(run) {
                return stamp;
            }
        }
        self.places.private_app(session?)
    }
}

/// The session an event belongs to, and the runs it names (its own, a
/// parent's).
fn origin(event: &ServerEvent) -> (Option<&str>, [Option<&str>; 2]) {
    use ServerEvent as E;
    fn s(sid: &Option<String>) -> Option<&str> {
        sid.as_deref()
    }
    let r = s;
    match event {
        E::Message {
            session_id, run_id, ..
        }
        | E::ContextUsage {
            session_id, run_id, ..
        }
        | E::Outcome {
            session_id, run_id, ..
        }
        | E::Token {
            session_id, run_id, ..
        }
        | E::TextSegment {
            session_id, run_id, ..
        }
        | E::Followups {
            session_id, run_id, ..
        }
        | E::ModelFallback {
            session_id, run_id, ..
        }
        | E::ToolProgress {
            session_id, run_id, ..
        } => (s(session_id), [r(run_id), None]),
        E::SubagentSpawned {
            session_id,
            subagent_run_id,
            parent_run_id,
            ..
        }
        | E::SubagentResult {
            session_id,
            subagent_run_id,
            parent_run_id,
            ..
        } => (s(session_id), [r(subagent_run_id), r(parent_run_id)]),
        E::AgentStatus {
            session_id,
            run_id,
            parent_run_id,
            ..
        }
        | E::ContentBlockStart {
            session_id,
            run_id,
            parent_run_id,
            ..
        }
        | E::ContentBlockUpdate {
            session_id,
            run_id,
            parent_run_id,
            ..
        }
        | E::TurnComplete {
            session_id,
            run_id,
            parent_run_id,
            ..
        } => (s(session_id), [r(run_id), r(parent_run_id)]),
        E::PlanUpdate { session_id, .. }
        | E::MissionTriggered { session_id, .. }
        | E::AskUser { session_id, .. }
        | E::WidgetResolved { session_id, .. }
        | E::AppLaunched { session_id, .. }
        | E::AllowanceSpent { session_id, .. }
        | E::BackgroundTask { session_id, .. } => (s(session_id), [None, None]),
        E::SessionReset { session_id }
        | E::QueueUpdated { session_id, .. }
        | E::TasksUpdated { session_id, .. }
        | E::WorkingFolderChanged { session_id, .. }
        | E::SessionCreated { session_id, .. } => (Some(session_id.as_str()), [None, None]),
        _ => (None, [None, None]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::turn_place::TurnPlace;

    fn token(session: &str, run: Option<&str>) -> ServerEvent {
        ServerEvent::Token {
            agent_id: "ling".into(),
            token: "t".into(),
            done: false,
            thinking: false,
            session_id: Some(session.into()),
            run_id: run.map(str::to_string),
        }
    }

    fn ledger() -> TurnPlace {
        TurnPlace {
            skill: Some("zz-ledger".into()),
            ..TurnPlace::home()
        }
    }

    /// An event is judged by when it was made: a run begun in a private
    /// turn keeps its stamp after the turn's mark is gone, a subagent's run
    /// takes its parent's, and an event with no run takes the running
    /// turn's.
    #[tokio::test]
    async fn an_event_is_stamped_when_it_is_made() {
        let places = RunningPlaces::default();
        let bus = EventBus::new(16, places.clone());
        let mut rx = bus.subscribe();
        let day = "sess-day-2026-11-16";
        let mark = places.mark(day, &ledger()).unwrap();
        places.keep_talk_private(day);
        places.begin_run("run-1", None, Some(day));
        bus.send(token(day, None)).unwrap();
        drop(mark);
        // The turn is over; its run and a background subagent of it stream on.
        places.begin_run("run-2", Some("run-1"), Some(day));
        bus.send(token(day, Some("run-1"))).unwrap();
        bus.send(token(day, Some("run-2"))).unwrap();
        // Nothing runs now: an event with no run is everyone's.
        bus.send(token(day, None)).unwrap();
        let mut stamps = Vec::new();
        for _ in 0..4 {
            stamps.push(rx.recv().await.unwrap().kept_to);
        }
        let app = Some("zz-ledger".to_string());
        assert_eq!(stamps, [app.clone(), app.clone(), app, None]);
    }

    /// A background task's entry carries its own stamp, decided as it began.
    #[tokio::test]
    async fn a_background_tasks_event_carries_its_own_stamp() {
        let bus = EventBus::detached(4);
        let mut rx = bus.subscribe();
        let mut task = crate::engine::events::BackgroundTask::running("t1", "shell", "ls");
        task.kept_to = Some("zz-ledger".into());
        bus.send(ServerEvent::BackgroundTask {
            session_id: Some("sess-day-2026-11-16".into()),
            agent_id: "ling".into(),
            task,
            notice: None,
        })
        .unwrap();
        assert_eq!(
            rx.recv().await.unwrap().kept_to.as_deref(),
            Some("zz-ledger")
        );
    }
}
