//! The verb table: every JSON verb memory answers, its route, what a replica
//! does with it, and how it runs. `Memory::call` and the HTTP adapters both
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

/// A verb's body: the args as given, the op's result.
pub(crate) type Run = fn(Arc<Memory>, Value) -> BoxFuture<'static, Result<Value, MemError>>;

pub(crate) struct Verb {
    pub name: &'static str,
    /// The route it answers at, and the `verb` of a queued write.
    pub path: &'static str,
    pub kind: Kind,
    pub run: Run,
    /// The op it calls, as written (`memory::write::add`) — for the
    /// no-bypass check.
    pub op: &'static str,
}

/// One row: deserialise the args into the op's request type, call the op.
/// A verb without `$req` takes no arguments and ignores them.
macro_rules! verb {
    ($name:literal, $path:literal, $kind:ident, $($op:ident)::+, $req:ty) => {
        Verb {
            name: $name,
            path: $path,
            kind: Kind::$kind,
            run: |state, args| {
                Box::pin(async move {
                    let req: $req = parse($name, args)?;
                    ops::$($op)::+(&state, req).await
                })
            },
            op: stringify!($($op)::+),
        }
    };
    ($name:literal, $path:literal, $kind:ident, $($op:ident)::+) => {
        Verb {
            name: $name,
            path: $path,
            kind: Kind::$kind,
            run: |state, _args| Box::pin(async move { ops::$($op)::+(&state).await }),
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
        memory::write::add,
        AddRequest
    ),
    verb!(
        "add_batch",
        "/api/memory/add_batch",
        Write,
        memory::write::add_batch,
        AddBatchRequest
    ),
    verb!(
        "get",
        "/api/memory/get",
        Read,
        memory::read::get,
        GetRequest
    ),
    verb!(
        "search",
        "/api/memory/search",
        Read,
        memory::read::search,
        SearchRequest
    ),
    verb!(
        "list",
        "/api/memory/list",
        Read,
        memory::read::list,
        ListRequest
    ),
    verb!(
        "count",
        "/api/memory/count",
        Read,
        memory::read::count,
        CountRequest
    ),
    verb!(
        "update",
        "/api/memory/update",
        Write,
        memory::update::update,
        UpdateRequest
    ),
    verb!(
        "delete",
        "/api/memory/delete",
        Write,
        memory::admin::delete,
        DeleteRequest
    ),
    verb!(
        "forget",
        "/api/memory/forget",
        Write,
        memory::admin::forget,
        ForgetRequest
    ),
    verb!(
        "restamp",
        "/api/memory/restamp",
        Write,
        memory::admin::restamp,
        RestampRequest
    ),
    verb!(
        "accounts",
        "/api/memory/accounts",
        Read,
        memory::admin::accounts
    ),
    verb!(
        "issues",
        "/api/memory/issues",
        Read,
        issues::issues,
        IssuesRequest
    ),
    verb!(
        "issue_add",
        "/api/memory/issue_add",
        Write,
        issues::issue_add,
        IssueAddRequest
    ),
    verb!(
        "issue_resolve",
        "/api/memory/issue_resolve",
        Write,
        issues::issue_resolve,
        IssueResolveRequest
    ),
    verb!("days", "/api/memory/days", Read, days::days, DaysRequest),
    verb!(
        "remember_day",
        "/api/memory/remember_day",
        PrimaryOnly,
        days::remember_day,
        RememberDayRequest
    ),
    verb!(
        "harvest_day",
        "/api/memory/harvest_day",
        Write,
        days::harvest_day,
        HarvestDayRequest
    ),
    verb!(
        "sweep",
        "/api/memory/sweep",
        PrimaryOnly,
        days::sweep,
        SweepRequest
    ),
    verb!(
        "dream_lease",
        "/api/memory/dream_lease",
        PrimaryOnly,
        lease::dream_lease,
        LeaseRequest
    ),
    verb!(
        "apply_schema",
        "/api/schema/apply",
        PrimaryOnly,
        migration::apply_schema,
        ApplySchemaRequest
    ),
    verb!(
        "chains",
        "/api/memory/chains",
        Read,
        chains::chains,
        ChainsRequest
    ),
    verb!("stats", "/api/memory/stats", Read, stats::stats),
    verb!(
        "session_start",
        "/api/memory/session_start",
        Read,
        session::session_start,
        SessionStartRequest
    ),
    verb!("role", "/api/memory/role", Read, primary::role),
];

pub(crate) fn find(name: &str) -> Option<&'static Verb> {
    VERBS.iter().find(|v| v.name == name)
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
mod tests {
    use super::*;

    #[test]
    fn the_derived_lists_are_the_replicas_lists() {
        assert_eq!(&WRITE_PATHS[..], crate::replica::forward::WRITES);
        assert_eq!(
            &PRIMARY_ONLY_PATHS[..],
            crate::replica::forward::PRIMARY_ONLY
        );
    }

    #[test]
    fn names_and_paths_are_unique() {
        for (i, v) in VERBS.iter().enumerate() {
            for w in &VERBS[i + 1..] {
                assert_ne!(v.name, w.name);
                assert_ne!(v.path, w.path);
            }
        }
    }

    /// Every verb's route is a route of the HTTP surface, read from the
    /// routers' source (axum cannot list a router's routes).
    #[test]
    fn every_verb_is_routed_at_its_path() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/http");
        let text: String = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .collect();
        for v in VERBS {
            let line = format!(".route(\"{}\",", v.path);
            assert!(
                text.contains(&line),
                "{} is not routed at {}",
                v.name,
                v.path
            );
        }
    }
}
