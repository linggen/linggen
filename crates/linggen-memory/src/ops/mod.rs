//! Memory's operations without HTTP: one file per area, the verbs and the
//! sidecar helpers they share. `http/` only adapts them. Rules for adding one
//! (`stats.rs` and `lease.rs` are the pattern):
//!
//! - A verb of `http/<area>.rs` lives in `ops/<area>.rs` as
//!   `pub async fn <verb>(state: &Memory, req: <Dto>) -> Result<Value, MemError>`
//!   holding the handler's whole body; a verb with no body omits `req`.
//! - No axum types in `ops/`: no `State`, `Json`, `Response`, `StatusCode`,
//!   no `http::envelope::ok`. Return the data the handler wrapped in `ok(..)`,
//!   fail with `MemError` where the body said `ApiError` (same constructor,
//!   same words). Keep the parameter named `state` so a body moves unchanged.
//! - The handle is `Memory` (`use crate::handle::Memory`); a file that also
//!   needs the row type imports it as `use crate::memory::Memory as Row`.
//! - The request DTO moves with its op into `ops/<area>.rs`, serde attributes
//!   byte for byte, the type made `pub`. If another file names it by its old
//!   path, leave `pub(crate) use crate::ops::<area>::<Dto>;` in the handler file.
//! - The handler keeps extraction, the call and today's wrapping, nothing else:
//!   `Ok(ok(ops::<area>::<verb>(&state, req).await?))`. Statuses, codes, error
//!   text, ordering and log lines stay exactly as they are.
//! - Unit tests of an op or helper go in `mod tests` at the bottom of its ops
//!   file; tests that drive a router stay in `http/`. Never edit
//!   `tests/contract/`: a red contract test means the move is wrong.
//! - One handler file per agent. Add your `pub mod` line here in alphabetical
//!   order to keep merges trivial.

pub mod config;
pub mod days;
pub mod issues;
pub mod lease;
pub mod network;
pub mod stats;
pub mod sync;
pub mod upkeep;
