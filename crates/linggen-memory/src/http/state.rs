//! The handle as the HTTP handlers hold it: `axum::State<SharedState>`.

pub use crate::handle::Memory;
use std::sync::Arc;

pub type SharedState = Arc<Memory>;

#[cfg(test)]
pub(crate) use crate::handle::scratch_state;
