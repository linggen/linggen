//! The event bus every surface and watcher listens on: events go out by
//! [`EventBus::send`], and [`EventBus::listen`] is the one way to listen.

use super::events::ServerEvent;
use tokio::sync::broadcast;

/// The broadcast every event goes out on.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<ServerEvent>,
}

impl EventBus {
    /// A bus holding `capacity` events per listener.
    pub fn new(capacity: usize) -> Self {
        Self {
            tx: broadcast::channel(capacity).0,
        }
    }

    /// Send `event` to every listener.
    pub fn send(&self, event: ServerEvent) -> Result<usize, broadcast::error::SendError<ServerEvent>> {
        self.tx.send(event)
    }

    /// The one way to listen on the bus.
    pub fn listen(&self) -> broadcast::Receiver<ServerEvent> {
        self.tx.subscribe()
    }

    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}
