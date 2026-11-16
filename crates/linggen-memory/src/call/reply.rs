//! What a verb answers through [`Memory::call`](crate::Memory::call).

use crate::error::{ErrorCode, MemError};
use serde_json::Value;

/// A verb's answer, and where it was decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// Run on this store: the op's result.
    Done(Value),
    /// A replica's write the primary did not answer: applied here and
    /// queued for it (HTTP marks it `x-ling-mem-queued: 1`).
    Queued(Value),
    /// A replica's write, answered by the primary: its status and body as
    /// they came — success or refusal.
    Primary { status: u16, body: Value },
}

impl Reply {
    /// The plain result an in-process caller wants. A primary's success is
    /// its envelope's `data`; its refusal is a [`MemError`] with its code
    /// and words.
    pub fn into_result(self) -> Result<Value, MemError> {
        match self {
            Reply::Done(v) | Reply::Queued(v) => Ok(v),
            Reply::Primary { status, body } if (200..300).contains(&status) => {
                Ok(body.get("data").cloned().unwrap_or(body))
            }
            Reply::Primary { status, body } => Err(refusal(status, &body)),
        }
    }
}

fn refusal(status: u16, body: &Value) -> MemError {
    let message = body["error"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("the primary answered {status}"));
    MemError {
        code: ErrorCode::from_wire(body["code"].as_str(), status),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_primarys_answer_is_its_data_or_its_refusal() {
        let ok = Reply::Primary {
            status: 200,
            body: json!({"ok": true, "data": {"action": "added"}}),
        };
        assert_eq!(ok.into_result().unwrap(), json!({"action": "added"}));

        let refused = Reply::Primary {
            status: 404,
            body: json!({"ok": false, "error": "no fact with id x", "code": "NOT_FOUND"}),
        };
        let e = refused.into_result().unwrap_err();
        assert_eq!(
            (e.code.code(), e.message.as_str()),
            ("NOT_FOUND", "no fact with id x")
        );

        let bare = Reply::Primary {
            status: 422,
            body: Value::Null,
        };
        let e = bare.into_result().unwrap_err();
        assert_eq!(
            (e.code.code(), e.message.as_str()),
            ("BAD_REQUEST", "the primary answered 422")
        );
    }
}
