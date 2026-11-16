//! The HTTP form of a verb: the body read as the verb's route always read
//! it, then [`Memory::call`](crate::Memory::call).
//!
//! A replica's write goes to the primary as its body came, before this
//! store parses it: a body this store cannot read is still the primary's to
//! judge, and is refused here (in axum's words, nothing queued) only when
//! the primary does not answer. `call::route` makes that decision for both.
//!
//! [`answer`] is the same for a caller whose args are JSON already (memory's
//! MCP server, the engine's door): the args fitted to the caller
//! ([`Who::scope`]), the verb's route found by its path, its body checked as
//! the route's `Json` checks it, then the verb — every answer, refusals
//! included, as the route gives it, with no request made.
//!
//! Who a route's call is for: memory's routes are reached only by callers
//! the host has already admitted as this machine or a machine it approved
//! (the engine's `memory_host::routes`), and a body that came through the
//! host's own door is stamped already — so a route's call is the owner's
//! ([`as_route`]).

use super::state::SharedState;
use crate::call::{route_of, Reply, Route, Who};
use crate::error::MemError;
use crate::ops::verbs;
use axum::body::{to_bytes, Body, Bytes};
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::{json, Value};

/// A verb body's check, as its route's `Json` extractor makes it.
pub(crate) type Probe = fn(&[u8]) -> Result<(), JsonRejection>;

/// The [`Probe`] of a verb whose body is a `T`.
pub(crate) fn probe<T: DeserializeOwned>(bytes: &[u8]) -> Result<(), JsonRejection> {
    Json::<T>::from_bytes(bytes).map(drop)
}

/// `/api/memory/<verb>` as the router answers a JSON POST of `args` for
/// `who`: the caller's refusal, an empty 404 for a path the router has no
/// verb at, the body check's rejection, or the verb's answer.
pub async fn answer(state: SharedState, who: Who, verb: &str, mut args: Value) -> Response {
    if let Err(refused) = who.scope(verb, &mut args) {
        return refused.into_response();
    }
    let Some(v) = verbs::at_path(&format!("/api/memory/{verb}")) else {
        state.mark_request();
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(probe) = v.probe else {
        return bare(state, v.name).await;
    };
    match route_of(&state, v.name) {
        Route::Refuse => bare(state, v.name).await,
        Route::Here => match probe(&body_of(&args)) {
            Ok(()) => as_route(state, v.name, args).await,
            Err(rejection) => rejection.into_response(),
        },
        Route::Forward => forward_args(state, v.name, v.path, probe, args).await,
    }
}

/// The bytes a JSON POST of `args` carries.
fn body_of(args: &Value) -> Vec<u8> {
    serde_json::to_vec(args).unwrap_or_default()
}

/// [`forward`] for args that are JSON already.
async fn forward_args(
    state: SharedState,
    verb: &str,
    path: &str,
    probe: Probe,
    args: Value,
) -> Response {
    let rejection = match probe(&body_of(&args)) {
        Ok(()) => return as_route(state, verb, args).await,
        Err(rejection) => rejection,
    };
    match crate::replica::forward::forward_in_order(&state, path, &args).await {
        Some(answer) => answer.into_response(),
        None => rejection.into_response(),
    }
}

/// The verb run for the owner — every route's call (see the module's
/// head), and an [`answer`]'s once its args are fitted to their caller.
async fn as_route(state: SharedState, verb: &str, args: Value) -> Response {
    respond(state.call(&Who::Owner, verb, args).await)
}

/// The most a replica buffers of a write's body before it forwards it.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// A verb that takes a JSON body of type `T`.
pub(crate) async fn serve<T: DeserializeOwned>(
    state: SharedState,
    verb: &str,
    req: Request,
) -> Response {
    match route_of(&state, verb) {
        Route::Forward => forward::<T>(state, verb, req).await,
        // Refused before the body is read.
        Route::Refuse => bare(state, verb).await,
        Route::Here => match args::<T>(req).await {
            Ok(args) => as_route(state, verb, args).await,
            Err(rejection) => rejection.into_response(),
        },
    }
}

/// A replica's write: buffered, then through `call` when this store can
/// read it; otherwise to the primary as sent (a body that is not JSON goes
/// as `null`), and refused here only if the primary does not answer.
async fn forward<T: DeserializeOwned>(state: SharedState, verb: &str, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return too_large();
    };
    let raw: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let rejection = match args::<T>(Request::from_parts(parts, Body::from(bytes))).await {
        Ok(args) => return as_route(state, verb, args).await,
        Err(rejection) => rejection,
    };
    let Some(path) = verbs::find(verb).map(|v| v.path) else {
        return rejection.into_response();
    };
    match crate::replica::forward::forward_in_order(&state, path, &raw).await {
        Some(answer) => answer.into_response(),
        None => rejection.into_response(),
    }
}

fn too_large() -> Response {
    let body = json!({ "ok": false, "error": "request body too large", "code": "REPLICA" });
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}

/// A verb that takes no arguments: its body is never read.
pub(crate) async fn bare(state: SharedState, verb: &str) -> Response {
    as_route(state, verb, Value::Null).await
}

fn respond(answer: Result<Reply, MemError>) -> Response {
    match answer {
        Ok(reply) => reply.into_response(),
        Err(e) => e.into_response(),
    }
}

/// The body as raw JSON — fields `T` does not name kept — refused exactly
/// as `Json<T>` refuses it: the content type first, then the size, then
/// the parse into `T`, in axum's own words.
pub(crate) async fn args<T: DeserializeOwned>(req: Request) -> Result<Value, JsonRejection> {
    let (parts, body) = req.into_parts();
    // `Json` checks the content type before it reads a byte; an empty body
    // asks it that alone.
    let probe = Request::from_parts(parts.clone(), Body::empty());
    if let Err(r @ JsonRejection::MissingJsonContentType(_)) =
        Json::<IgnoredAny>::from_request(probe, &()).await
    {
        return Err(r);
    }
    let bytes = Bytes::from_request(Request::from_parts(parts, body), &()).await?;
    Json::<T>::from_bytes(&bytes).map(drop)?;
    Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
