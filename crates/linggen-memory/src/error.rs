//! The error every memory verb returns: a short machine code plus the
//! message. `http::envelope` turns it into the `{ok:false, error, code}`
//! envelope with the matching HTTP status.

/// Machine-readable error codes. Kept short and uppercase so the
/// dispatcher can surface them in `provider error [CODE]: MSG`.
#[derive(Debug, Clone, Copy)]
pub enum ErrorCode {
    BadRequest,
    NotFound,
    Unauthorized,
    Forbidden,
    TooMany,
    Conflict,
    Internal,
}

impl ErrorCode {
    /// The HTTP status this code answers with.
    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::BadRequest => 400,
            ErrorCode::NotFound => 404,
            ErrorCode::Unauthorized => 401,
            ErrorCode::Forbidden => 403,
            ErrorCode::TooMany => 429,
            ErrorCode::Conflict => 409,
            ErrorCode::Internal => 500,
        }
    }

    /// The code as the envelope spells it.
    pub fn code(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "BAD_REQUEST",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Unauthorized => "UNAUTHORIZED",
            ErrorCode::Forbidden => "FORBIDDEN",
            ErrorCode::TooMany => "TOO_MANY_REQUESTS",
            ErrorCode::Conflict => "CONFLICT",
            ErrorCode::Internal => "INTERNAL",
        }
    }
}

/// A memory verb's failure: what kind, and the words a caller sees.
#[derive(Debug)]
pub struct MemError {
    pub code: ErrorCode,
    pub message: String,
}

impl MemError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::BadRequest,
            message: msg.into(),
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::NotFound,
            message: msg.into(),
        }
    }

    /// The LAN gate's refusal — a caller off this machine without a paired
    /// device token. Distinct from `bad_request` so a client can tell "you may
    /// not" from "you asked wrong".
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Unauthorized,
            message: msg.into(),
        }
    }

    /// A caller the gate let in that still may not do this — a paired device
    /// asking to change how this machine listens. Only this machine may.
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Forbidden,
            message: msg.into(),
        }
    }

    /// An open route asked too often from one address.
    pub fn too_many(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::TooMany,
            message: msg.into(),
        }
    }

    /// Someone else holds what this asked for (a dream lease on that day).
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Conflict,
            message: msg.into(),
        }
    }

    pub fn internal(err: anyhow::Error) -> Self {
        Self {
            code: ErrorCode::Internal,
            message: format!("{err:#}"),
        }
    }
}

/// Map an `anyhow::Error` into an internal error. Used for store errors
/// that don't have a narrower classification.
impl From<anyhow::Error> for MemError {
    fn from(err: anyhow::Error) -> Self {
        MemError::internal(err)
    }
}
