//! # ling-mem — library interface
//!
//! Semantic memory store, hosted by the Linggen engine (`hosted`); its data
//! verbs run as `ling mem …` (`cli::run_data`).

pub mod cli;
pub mod daemon;
pub mod embed;
mod hash;
pub mod hosted;
pub mod http;
pub mod memory;
pub mod replica;
pub mod telemetry;

pub use memory::{Memory, MemoryType, Origin, Outcome};
