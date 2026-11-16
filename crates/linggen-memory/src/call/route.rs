//! Where a verb runs: here, on the primary, or nowhere on this machine.
//! One decision for [`Memory::call`](crate::Memory::call) and the HTTP
//! adapter (`http::verb`), which must read a replica's write body only after
//! it has gone to the primary.

use crate::handle::Memory;
use crate::ops::verbs::{self, Kind, Verb};
use crate::replica::Role;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// Run on this store.
    Here,
    /// A replica's write: to the primary, or here and queued.
    Forward,
    /// The primary's alone; a replica refuses.
    Refuse,
}

pub(crate) fn route(state: &Memory, verb: &Verb) -> Route {
    if state.replica.role() != Role::Replica {
        return Route::Here;
    }
    match verb.kind {
        Kind::Read => Route::Here,
        Kind::Write => Route::Forward,
        Kind::PrimaryOnly => Route::Refuse,
    }
}

/// The route of a verb by name; an unknown one is left to `call` to refuse.
pub(crate) fn route_of(state: &Memory, name: &str) -> Route {
    verbs::find(name).map_or(Route::Here, |v| route(state, v))
}
