//! Hermetic system tests (doc/test-design.md § 2):
//!
//!   cargo nextest run --features test-support --test system
//!   (or ./scripts/check.sh system)
//!
//! Each test builds its own world — the fixture home (tests/fixtures/home)
//! in a fresh root, the fake model and the built `ling` (which hosts its own
//! memory) — and throws it away. Nothing touches ~/.linggen or ports 9527/9528
//! (`support::guard`). A failed test keeps its world and prints where;
//! `LINGGEN_SYSTEM_KEEP=1` keeps every one.

mod app_door;
mod app_index;
mod assets;
mod background;
mod browser_tools;
mod claude_rules;
mod day_places;
mod day_session;
mod delegate;
mod door;
mod event_speech;
mod event_speech_people;
mod drawing;
mod fixtures;
mod heralds;
mod hearing;
mod mac_tools;
mod memory_ops;
mod met;
mod model_kinds;
mod leftovers;
mod live_token;
mod missions;
mod notes;
mod person_lan;
mod pictures;
mod place_rules;
mod plan;
mod presence;
mod private_app;
mod prompt;
mod reading;
mod restart;
mod rooms;
mod runs;
mod sessions;
mod shared_table;
mod step_limit;
mod turn_entrance;
mod skills;
mod spoken_line;
mod spend;
mod support;
mod unprompted;
mod voice_seen;
mod workspace;
