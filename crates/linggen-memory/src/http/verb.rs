//! The HTTP form of a verb: the body read as the verb's route always read
//! it, then [`Memory::call`](crate::Memory::call).
//!
//! A replica's write goes to the primary as its body came, before this
//! store parses it: a body this store cannot read is still the primary's to
//! judge, and is refused here (in axum's words, nothing queued) only when
//! the primary does not answer. `call::route` makes that decision for both.

use super::state::SharedState;
use crate::call::{route_of, Reply, Route};
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
            Ok(args) => respond(state.call(verb, args).await),
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
        Ok(args) => return respond(state.call(verb, args).await),
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
    respond(state.call(verb, Value::Null).await)
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
