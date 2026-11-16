//! Memory's operations without HTTP: one file per area, the verbs and the
//! sidecar helpers they share. `http/` only adapts them. Rules for adding one
//! (`stats.rs` and `lease.rs` are the pattern):
//!
//! - A verb of `http/<area>.rs` lives in `ops/<area>.rs` as `pub(in
//!   crate::ops) async fn <verb>(state: &Memory, req: <Dto>) -> Result<Value,
//!   MemError>` (a verb without arguments takes `state` alone), and has a row
//!   in `verbs.rs`, the verb table. The table is the only way in: it is what
//!   `Memory::call` runs, and `call` is where a replica forwards its writes
//!   — a direct call would skip that (`verbs_tests.rs` locks it). Routes
//!   that are not verbs (connect, sync, config, primary, compact) keep `pub`
//!   ops their handlers call. Only an op whose callee needs the `Arc` (it
//!   spawns, e.g. `replica::link::ask`) takes `&Arc<Memory>`.
//! - No axum types in `ops/`: no `State`, `Json`, `Query`, `Response`,
//!   `StatusCode`, `ok(..)`. Return what the handler wrapped in `ok(..)`; fail
//!   with `MemError` where it said `ApiError` (same constructor, same words).
//!   Keep the parameter named `state` so a body moves unchanged. Private
//!   helpers the body calls move with it and take `&Memory`.
//! - The handle is `Memory` (`use crate::handle::Memory`); a file that also
//!   needs the row type imports it as `use crate::memory::Memory as Row`.
//! - The DTO (`Json` or `Query` body) moves with its op, serde attributes byte
//!   for byte, made `pub`; if another file names its old path, leave
//!   `pub(crate) use crate::ops::<area>::<Dto>;` in the handler file.
//! - A verb's handler is `http::verb::serve::<Dto>(state, "<verb>", req)`
//!   (`bare` without arguments); its `.route("/api/memory/<verb>", …)` line
//!   stays literal (the engine's tests read it). Any other handler keeps its
//!   extraction, gate checks (`require_local` on `ConnectInfo` + headers),
//!   the call and the wrapping: `Ok(ok(ops::<area>::<op>(&state, req).await?))`.
//! - Op and helper unit tests go at the bottom of the ops file; router tests
//!   stay in `http/`. Never edit `tests/contract/`: red there = wrong move.
//! - One handler file per agent. Add your `pub mod` line here in alphabetical
//!   order to keep merges trivial.

pub mod chains;
pub mod compact;
pub mod config;
pub mod connect;
pub mod days;
pub mod issues;
pub mod lease;
pub mod memory;
pub mod migration;
pub mod network;
pub mod primary;
pub mod session;
pub mod stats;
pub mod sync;
pub mod upkeep;
pub(crate) mod verbs;
