//! Memory's operations without HTTP: one file per area, the verbs and the
//! sidecar helpers they share. `http/` only adapts them. Rules for adding one
//! (`stats.rs` and `lease.rs` are the pattern):
//!
//! - A verb of `http/<area>.rs` lives in `ops/<area>.rs` as `pub async fn
//!   <verb>(state: &Memory, req: <Dto>) -> Result<Value, MemError>` holding the
//!   handler's whole body; no body omits `req`. Only a verb whose callee needs
//!   the `Arc` (it spawns, e.g. `replica::link::ask`) takes `&Arc<Memory>`.
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
//! - The handler keeps extraction, gate checks (`require_local` on
//!   `ConnectInfo` + headers), the call and today's wrapping:
//!   `Ok(ok(ops::<area>::<verb>(&state, req).await?))`.
//! - Op and helper unit tests go at the bottom of the ops file; router tests
//!   stay in `http/`. Never edit `tests/contract/`: red there = wrong move.
//! - One handler file per agent. Add your `pub mod` line here in alphabetical
//!   order to keep merges trivial.

pub mod chains;
pub mod config;
pub mod days;
pub mod issues;
pub mod lease;
pub mod network;
pub mod primary;
pub mod session;
pub mod stats;
pub mod sync;
pub mod upkeep;
