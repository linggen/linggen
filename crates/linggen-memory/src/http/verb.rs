//! The HTTP form of a verb: the body read as the verb's route always read
//! it, then [`Memory::call`](crate::Memory::call).

use super::state::SharedState;
use crate::call::Reply;
use crate::error::MemError;
use axum::body::{Body, Bytes};
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::Value;

/// A verb that takes a JSON body of type `T`.
pub(crate) async fn serve<T: DeserializeOwned>(
    state: SharedState,
    verb: &str,
    req: Request,
) -> Response {
    match args::<T>(req).await {
        Ok(args) => respond(state.call(verb, args).await),
        Err(rejection) => rejection.into_response(),
    }
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
