//! The verb table: every JSON verb memory answers, its route, what a replica
//! does with it, who may run it, and how it runs. `Memory::call` and the HTTP adapters both
//! go through it; the replica's lists (`replica::forward::WRITES`,
//! `PRIMARY_ONLY`) are read off it.
//!
//! Routes that need the connection (the peer address, `require_local`), the
//! sync GETs, health, config and `/mcp` are not verbs: they stay routes.

use crate::error::MemError;
use crate::handle::Memory;
use crate::ops;
use crate::ops::chains::ChainsRequest;
use crate::ops::days::{DaysRequest, HarvestDayRequest, RememberDayRequest, SweepRequest};
use crate::ops::issues::{IssueAddRequest, IssueResolveRequest, IssuesRequest};
use crate::ops::lease::LeaseRequest;
use crate::ops::memory::dto::{
    AddBatchRequest, AddRequest, CountRequest, DeleteRequest, ForgetRequest, GetRequest,
    ListRequest, RestampRequest, SearchRequest, UpdateRequest,
};
use crate::ops::migration::ApplySchemaRequest;
use crate::ops::session::SessionStartRequest;
use futures::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;

/// What a replica does with a verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Served from the local copy.
    Read,
    /// Forwarded to the primary, or applied here and queued.
    Write,
    /// The primary's alone: dreaming, the forget sweep, the schema step.
    PrimaryOnly,
}

/// Who may run a verb (`call::Who`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Kept per person: it reads or changes only the rows of the person the
    /// stamp names, so a person may run it.
    Person,
    /// The owner's alone: it is not kept per person.
    Owner,
    /// This machine's owner alone — not a phone, even the owner's: it crosses
    /// account lines by design.
    ThisMachine,
}

/// A verb's body: the args as given, the op's result.
pub(crate) type Run = fn(Arc<Memory>, Value) -> BoxFuture<'static, Result<Value, MemError>>;

pub(crate) struct Verb {
    pub name: &'static str,
    /// The route it answers at, and the `verb` of a queued write.
    pub path: &'static str,
    pub kind: Kind,
    pub reach: Reach,
    pub run: Run,
    /// The body check its route makes before the verb runs (`None`: the
    /// route never reads a body) — what `http::verb::answer` gives a caller
    /// whose args were never a request.
    pub probe: Option<crate::http::verb::Probe>,
    /// The op it calls, as written (`memory :: write :: add`) — for the
    /// no-bypass check.
    #[cfg(test)]
    pub op: &'static str,
}

/// One row: deserialise the args into the op's request type, call the op.
/// A verb without `$req` takes no arguments and ignores them.
macro_rules! verb {
    ($name:literal, $path:literal, $kind:ident, $reach:ident, $($op:ident)::+, $req:ty) => {
        Verb {
            name: $name,
            path: $path,
            kind: Kind::$kind,
            reach: Reach::$reach,
            run: |state, args| {
                Box::pin(async move {
                    let req: $req = parse($name, args)?;
                    ops::$($op)::+(&state, req).await
                })
            },
            probe: Some(crate::http::verb::probe::<$req>),
            #[cfg(test)]
            op: stringify!($($op)::+),
        }
    };
    ($name:literal, $path:literal, $kind:ident, $reach:ident, $($op:ident)::+) => {
        Verb {
            name: $name,
            path: $path,
            kind: Kind::$kind,
            reach: Reach::$reach,
            run: |state, _args| Box::pin(async move { ops::$($op)::+(&state).await }),
            probe: None,
            #[cfg(test)]
            op: stringify!($($op)::+),
        }
    };
}

/// Every verb. The order is the replica lists' order.
pub(crate) const VERBS: &[Verb] = &[
    verb!(
        "add",
        "/api/memory/add",
        Write,
        Person,
        memory::write::add,
        AddRequest
    ),
    verb!(
        "add_batch",
        "/api/memory/add_batch",
        Write,
        Person,
        memory::write::add_batch,
        AddBatchRequest
    ),
    verb!(
        "get",
        "/api/memory/get",
        Read,
        Person,
        memory::read::get,
        GetRequest
    ),
    verb!(
        "search",
        "/api/memory/search",
        Read,
        Person,
        memory::read::search,
        SearchRequest
    ),
    verb!(
        "list",
        "/api/memory/list",
        Read,
        Person,
        memory::read::list,
        ListRequest
    ),
    verb!(
        "count",
        "/api/memory/count",
        Read,
        Person,
        memory::read::count,
        CountRequest
    ),
    verb!(
        "update",
        "/api/memory/update",
        Write,
        Person,
        memory::update::update,
        UpdateRequest
    ),
    verb!(
        "delete",
        "/api/memory/delete",
        Write,
        Person,
        memory::admin::delete,
        DeleteRequest
    ),
    verb!(
        "forget",
        "/api/memory/forget",
        Write,
        Person,
        memory::admin::forget,
        ForgetRequest
    ),
    verb!(
        "restamp",
        "/api/memory/restamp",
        Write,
        ThisMachine,
        memory::admin::restamp,
        RestampRequest
    ),
    verb!(
        "accounts",
        "/api/memory/accounts",
        Read,
        ThisMachine,
        memory::admin::accounts
    ),
    verb!(
        "issues",
        "/api/memory/issues",
        Read,
        Owner,
        issues::issues,
        IssuesRequest
    ),
    verb!(
        "issue_add",
        "/api/memory/issue_add",
        Write,
        Owner,
        issues::issue_add,
        IssueAddRequest
    ),
    verb!(
        "issue_resolve",
        "/api/memory/issue_resolve",
        Write,
        Owner,
        issues::issue_resolve,
        IssueResolveRequest
    ),
    verb!(
        "days",
        "/api/memory/days",
        Read,
        Owner,
        days::days,
        DaysRequest
    ),
    verb!(
        "remember_day",
        "/api/memory/remember_day",
        PrimaryOnly,
        Owner,
        days::remember_day,
        RememberDayRequest
    ),
    verb!(
        "harvest_day",
        "/api/memory/harvest_day",
        Write,
        Owner,
        days::harvest_day,
        HarvestDayRequest
    ),
    verb!(
        "sweep",
        "/api/memory/sweep",
        PrimaryOnly,
        Owner,
        days::sweep,
        SweepRequest
    ),
    verb!(
        "dream_lease",
        "/api/memory/dream_lease",
        PrimaryOnly,
        Owner,
        lease::dream_lease,
        LeaseRequest
    ),
    verb!(
        "apply_schema",
        "/api/schema/apply",
        PrimaryOnly,
        Owner,
        migration::apply_schema,
        ApplySchemaRequest
    ),
    verb!(
        "chains",
        "/api/memory/chains",
        Read,
        Owner,
        chains::chains,
        ChainsRequest
    ),
    verb!("stats", "/api/memory/stats", Read, Owner, stats::stats),
    verb!(
        "session_start",
        "/api/memory/session_start",
        Read,
        Person,
        session::session_start,
        SessionStartRequest
    ),
    verb!("role", "/api/memory/role", Read, Owner, primary::role),
];

pub(crate) fn find(name: &str) -> Option<&'static Verb> {
    VERBS.iter().find(|v| v.name == name)
}

/// The verb routed at `path`, as the router finds it.
pub(crate) fn at_path(path: &str) -> Option<&'static Verb> {
    VERBS.iter().find(|v| v.path == path)
}

/// The args as the op's request type. The words are serde's, behind the
/// verb's name.
fn parse<T: DeserializeOwned>(verb: &str, args: Value) -> Result<T, MemError> {
    serde_json::from_value(args).map_err(|e| MemError::bad_request(format!("{verb}: {e}")))
}

const fn count(kind: Kind) -> usize {
    let (mut i, mut n) = (0, 0);
    while i < VERBS.len() {
        if VERBS[i].kind as u8 == kind as u8 {
            n += 1;
        }
        i += 1;
    }
    n
}

const fn paths<const N: usize>(kind: Kind) -> [&'static str; N] {
    let mut out = [""; N];
    let (mut i, mut n) = (0, 0);
    while i < VERBS.len() {
        if VERBS[i].kind as u8 == kind as u8 {
            out[n] = VERBS[i].path;
            n += 1;
        }
        i += 1;
    }
    out
}

/// The routes of the writes a replica forwards, in table order.
pub(crate) const WRITE_PATHS: [&str; count(Kind::Write)] = paths(Kind::Write);
/// The routes a replica refuses, in table order.
pub(crate) const PRIMARY_ONLY_PATHS: [&str; count(Kind::PrimaryOnly)] = paths(Kind::PrimaryOnly);

#[cfg(test)]
#[path = "verbs_tests.rs"]
mod tests;
