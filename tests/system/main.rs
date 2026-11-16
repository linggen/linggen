//! Hermetic system tests (doc/test-design.md § 2):
//!
//!   cargo nextest run --test system        (or ./scripts/check.sh system)
//!
//! Each test builds its own world — the fixture home (tests/fixtures/home)
//! in a fresh root, the fake model and the built `ling` (which hosts its own
//! memory) — and throws it away. Nothing touches ~/.linggen or ports 9527/9528
//! (`support::guard`). A failed test keeps its world and prints where;
//! `LINGGEN_SYSTEM_KEEP=1` keeps every one.

mod assets;
mod background;
mod claude_bridge;
mod claude_host;
mod claude_subagents;
mod drawing;
mod fixtures;
mod met;
mod missions;
mod pictures;
mod plan;
mod presence;
mod prompt;
mod reading;
mod restart;
mod runs;
mod sessions;
mod shared_table;
mod skills;
mod support;
mod workspace;
