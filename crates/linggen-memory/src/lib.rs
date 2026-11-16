//! # ling-mem — library interface
//!
//! Semantic memory store. [`Memory`] is the open store's handle, made by
//! `hosted::open*`; the Linggen engine serves its routes (`http`), and its
//! data verbs run as `ling mem …` (`cli::run_data`).

pub mod cli;
pub mod daemon;
pub mod embed;
pub mod error;
pub mod handle;
mod hash;
pub mod hosted;
pub mod http;
pub mod memory;
pub(crate) mod ops;
pub mod replica;

pub use error::{ErrorCode, MemError};
pub use handle::Memory;
pub use memory::{MemoryType, Origin, Outcome};
