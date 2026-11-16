//! Standard response envelope — every handler returns `{ok, data}` on
//! success or `{ok:false, error, code}` on failure.
//!
//! The shape is the spec's HTTP dispatch contract (see
//! `../../linggen/doc/memory-spec.md`, § HTTP dispatch contract). Keeping
//! it uniform lets Linggen's dispatcher parse every provider the same way.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};

use crate::call::Reply;
pub use crate::error::{ErrorCode, MemError};
use axum::http::HeaderValue;

/// The header a replica's queued write answers with.
pub const QUEUED_HEADER: &str = "x-ling-mem-queued";

/// Emit a success envelope `{ok:true, data}` with a given status code.
pub fn ok<T: Serialize>(data: T) -> Response {
    (StatusCode::OK, Json(json!({"ok": true, "data": data}))).into_response()
}

/// The error type every handler returns — the library's [`MemError`]
/// under its HTTP-side name.
pub type ApiError = MemError;

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body: Value = json!({
            "ok": false,
            "error": self.message,
            "code": self.code.code(),
        });
        let status = StatusCode::from_u16(self.code.http_status())
            .expect("every error code maps to a valid status");
        (status, Json(body)).into_response()
    }
}

/// A verb's answer as its route gives it: the envelope (marked when the
/// write was queued), or the primary's status and body as they came.
impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        match self {
            Reply::Done(data) => ok(data),
            Reply::Queued(data) => {
                let mut res = ok(data);
                res.headers_mut()
                    .insert(QUEUED_HEADER, HeaderValue::from_static("1"));
                res
            }
            Reply::Primary { status, body } => {
                let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
                (status, Json(body)).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The statuses the envelope answered with before the error moved to
    /// the crate root.
    #[test]
    fn every_code_keeps_its_status() {
        let pairs = [
            (ErrorCode::BadRequest, StatusCode::BAD_REQUEST),
            (ErrorCode::NotFound, StatusCode::NOT_FOUND),
            (ErrorCode::Unauthorized, StatusCode::UNAUTHORIZED),
            (ErrorCode::Forbidden, StatusCode::FORBIDDEN),
            (ErrorCode::TooMany, StatusCode::TOO_MANY_REQUESTS),
            (ErrorCode::Conflict, StatusCode::CONFLICT),
            (ErrorCode::Internal, StatusCode::INTERNAL_SERVER_ERROR),
        ];
        for (code, status) in pairs {
            let res = MemError {
                code,
                message: String::new(),
            }
            .into_response();
            assert_eq!(res.status(), status, "{}", code.code());
        }
    }
}
