//! Hermetic system tests (doc/test-design.md § 2):
//!
//!   cargo nextest run --test system        (or ./scripts/check.sh system)
//!
//! Each test builds its own world — the fixture home (tests/fixtures/home)
//! in a fresh root, the fake model, a scratch ling-mem and the built `ling`
//! — and throws it away. Nothing touches ~/.linggen or ports 9527/9528
//! (`support::guard`). A failed test keeps its world and prints where;
//! `LINGGEN_SYSTEM_KEEP=1` keeps every one. `LING_MEM_BIN` picks the ling-mem.

mod fixtures;
mod met;
mod presence;
mod prompt;
mod sessions;
mod shared_table;
mod skills;
mod support;
