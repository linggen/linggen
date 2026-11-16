//! Golden contract tests of memory's HTTP surface — the safety net for
//! moving memory from "a service the engine hosts" to "a library the engine
//! calls". Every test drives the real router (`build_hosted_router`) over
//! loopback HTTP on a scratch store with the deterministic test embedder,
//! and pins the status and the whole envelope a caller reads.
//!
//! One binary, one module per area (each test binary links all of LanceDB).

mod guards;
mod layers;
mod mcp;
mod person;
mod replica;
mod replica_edges;
mod shipping;
mod support;
mod upkeep;
mod verbs;
