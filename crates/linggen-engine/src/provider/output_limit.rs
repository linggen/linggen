//! The output limit (`max_tokens`) an Anthropic request carries.
//!
//! One fixed 8,192 cut healthy coding runs: a large edit was cut, written
//! again, cut again. The Models API says what each model can write
//! (`GET /v1/models/{id}` → `max_tokens`); this module turns that, a
//! per-model config value and a conservative fallback into the one number a
//! request sends — the only place that decides it.
//!
//! **What is sent.** A request that carries tools (a coding run's turn) gets
//! the model's cap, or [`WORKING_OUTPUT_TOKENS`] when the cap is higher: the
//! limit is also counted against the context window (input + `max_tokens`
//! must fit), and the provider counts it toward the output rate limit when a
//! request begins, so 128,000 would be asked for by every turn that writes
//! one line. 64,000 is eight times the old limit — about 250 KB of source in
//! one call. A text-only request (a summary, a one-line companion turn,
//! a title) keeps [`FALLBACK_OUTPUT_TOKENS`]: it never needed more.
//! A configured value replaces both, clamped to the cap when that is known.
//! A model the API cannot describe gets [`FALLBACK_OUTPUT_TOKENS`].

use serde_json::Value;

/// The limit sent for a model that carries tools, when its cap is higher.
pub const WORKING_OUTPUT_TOKENS: u32 = 64_000;

/// The limit sent when nothing says more: the number used before the Models
/// API was asked, and still for a text-only request.
pub const FALLBACK_OUTPUT_TOKENS: u32 = 8_192;

/// The least room worth retrying a request into when input + limit were over
/// the context window; below it the original error stands.
const LEAST_ROOM: u32 = 1_024;

/// The `max_tokens` to send: the configured value, else the working limit
/// for a request with tools, else the fallback; never above the model's
/// `cap` when that is known.
pub fn tokens_to_send(configured: Option<u32>, cap: Option<u32>, with_tools: bool) -> u32 {
    let wanted = configured.unwrap_or(if with_tools && cap.is_some() {
        WORKING_OUTPUT_TOKENS
    } else {
        FALLBACK_OUTPUT_TOKENS
    });
    cap.map_or(wanted, |cap| wanted.min(cap)).max(1)
}

/// The output cap a Models API answer states (`max_tokens`), when it states
/// one.
pub fn cap_in(model: &Value) -> Option<u32> {
    let cap = model.get("max_tokens")?.as_u64()?;
    u32::try_from(cap).ok().filter(|cap| *cap > 0)
}

/// The room left for output after a "input length and `max_tokens` exceed
/// context limit: 190000 + 64000 > 200000" refusal: the window less the
/// input it counted, when that is worth a second request.
pub fn room_after_overflow(error_body: &str) -> Option<u32> {
    let at = error_body.find("exceed context limit:")? + "exceed context limit:".len();
    let numbers: Vec<u64> = error_body[at..]
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(3)
        .filter_map(|part| part.parse().ok())
        .collect();
    let [input, _asked, window] = numbers[..] else {
        return None;
    };
    let room = u32::try_from(window.checked_sub(input)?).ok()?;
    (room >= LEAST_ROOM).then_some(room)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tools_get_the_working_limit_below_a_high_cap_and_the_cap_below_it() {
        assert_eq!(tokens_to_send(None, Some(128_000), true), WORKING_OUTPUT_TOKENS);
        assert_eq!(tokens_to_send(None, Some(32_000), true), 32_000);
        assert_eq!(tokens_to_send(None, Some(4_096), true), 4_096);
    }

    #[test]
    fn a_text_only_request_or_an_unknown_model_keeps_the_fallback() {
        assert_eq!(tokens_to_send(None, Some(128_000), false), FALLBACK_OUTPUT_TOKENS);
        assert_eq!(tokens_to_send(None, None, true), FALLBACK_OUTPUT_TOKENS);
        assert_eq!(tokens_to_send(None, Some(4_096), false), 4_096);
    }

    #[test]
    fn a_configured_value_wins_but_never_passes_the_cap() {
        assert_eq!(tokens_to_send(Some(20_000), Some(128_000), true), 20_000);
        assert_eq!(tokens_to_send(Some(20_000), Some(128_000), false), 20_000);
        assert_eq!(tokens_to_send(Some(200_000), Some(128_000), true), 128_000);
        assert_eq!(tokens_to_send(Some(200_000), None, true), 200_000);
    }

    #[test]
    fn the_cap_is_read_from_the_models_api_answer() {
        let answer = json!({"id": "m", "max_input_tokens": 1_000_000, "max_tokens": 128_000});
        assert_eq!(cap_in(&answer), Some(128_000));
        assert_eq!(cap_in(&json!({"id": "m"})), None);
        assert_eq!(cap_in(&json!({"max_tokens": 0})), None);
        assert_eq!(cap_in(&json!({"max_tokens": "big"})), None);
    }

    #[test]
    fn the_room_left_is_read_from_the_overflow_refusal() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"input length and `max_tokens` exceed context limit: 190000 + 64000 > 200000, decrease input length or `max_tokens` and try again"}}"#;
        assert_eq!(room_after_overflow(body), Some(10_000));
        // Input alone over the window: nothing to retry into.
        let full = body.replace("190000", "199900");
        assert_eq!(room_after_overflow(&full), None);
        assert_eq!(room_after_overflow("context length exceeded"), None);
    }
}
