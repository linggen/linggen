//! One picture through the Responses API's hosted `image_generation` tool —
//! how Codex CLI draws on a ChatGPT sign-in. The stream carries the picture
//! as base64 in an `image_generation_call` item; a refusal (a usage limit,
//! a declined prompt) comes back as one plain line, never retried elsewhere.

use super::{sse_data, sse_lines, OpenAiClient};
use anyhow::{anyhow, Result};
use base64::Engine as _;
use futures_util::StreamExt;
use serde_json::{json, Value};

/// What to send.
pub struct PictureRequest<'a> {
    pub model: &'a str,
    pub prompt: &'a str,
    /// `1024x1024`, `1536x1024`, …
    pub size: &'a str,
    /// A picture to keep the pose and shape of, as PNG/JPEG bytes.
    pub reference: Option<(&'a [u8], &'a str)>,
}

/// The drawn picture.
#[derive(Debug)]
pub struct DrawnPicture {
    pub png: Vec<u8>,
}

const INSTRUCTIONS: &str = "Draw the picture the user describes with the image_generation tool, \
    exactly once. Do not answer in words.";

impl OpenAiClient {
    /// Draw one picture. Errors are one line, fit for a tool result.
    pub async fn draw_picture(&self, req: &PictureRequest<'_>) -> Result<DrawnPicture> {
        let url = format!("{}/responses", self.base_url);
        let rb = self
            .http
            .post(url)
            .header("Accept", "text/event-stream")
            .json(&request_body(req));
        let resp = self
            .send_with_oauth_retry(rb)
            .await
            .map_err(|e| anyhow!("ChatGPT could not be reached: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(refusal_line(status.as_u16(), &text)));
        }
        let mut collect = Collected::default();
        let mut lines = sse_lines(resp);
        while let Some(line) = lines.next().await {
            let line = line.map_err(|e| anyhow!("ChatGPT's picture stream broke: {e}"))?;
            let Some(event) = sse_data(&line).and_then(|d| serde_json::from_str(d).ok()) else {
                continue;
            };
            collect.take(&event);
        }
        collect.finish()
    }
}

fn request_body(req: &PictureRequest<'_>) -> Value {
    let mut content = vec![json!({"type": "input_text", "text": req.prompt})];
    if let Some((bytes, mime)) = req.reference {
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        content
            .push(json!({"type": "input_image", "image_url": format!("data:{mime};base64,{b64}")}));
    }
    json!({
        "model": req.model,
        "instructions": INSTRUCTIONS,
        "input": [{"type": "message", "role": "user", "content": content}],
        "tools": [{"type": "image_generation", "output_format": "png", "size": req.size}],
        "tool_choice": "auto",
        "stream": true,
        "store": false,
    })
}

/// A non-success answer as one line: a usage limit says so, with ChatGPT's
/// own words when it gave any.
pub(super) fn refusal_line(status: u16, body: &str) -> String {
    if status == 401 {
        return "ChatGPT sign-in expired; sign in again in Settings → Models.".to_string();
    }
    let said = error_words(body);
    let what = match status {
        429 => "ChatGPT's usage limit is reached",
        402 => "ChatGPT has no picture credits left",
        _ => "ChatGPT refused the picture",
    };
    match said {
        Some(said) => format!("{what} ({status}): {said}"),
        None => format!("{what} ({status})."),
    }
}

/// The message inside an error body: `error.message`, `detail`, or the raw
/// text cut to one short line.
fn error_words(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let said = v["error"]["message"]
        .as_str()
        .or(v["detail"].as_str())
        .or(v["detail"]["message"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| body.trim().to_string());
    let line: String = said
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    (!line.is_empty()).then_some(line)
}

/// What a picture stream said, event by event.
#[derive(Default)]
pub(super) struct Collected {
    image: Option<String>,
    failure: Option<String>,
    words: String,
}

impl Collected {
    pub(super) fn take(&mut self, event: &Value) {
        match event["type"].as_str().unwrap_or("") {
            "response.output_item.done" => self.take_item(&event["item"]),
            "response.output_text.delta" => {
                self.words.push_str(event["delta"].as_str().unwrap_or(""))
            }
            "response.failed" => self.fail(&event["response"]["error"]["message"]),
            "error" => self.fail(event.get("message").unwrap_or(&event["error"]["message"])),
            "response.incomplete" => self.fail(&event["response"]["incomplete_details"]["reason"]),
            _ => {}
        }
    }

    fn take_item(&mut self, item: &Value) {
        if item["type"] != "image_generation_call" {
            return;
        }
        match item["result"].as_str().filter(|r| !r.is_empty()) {
            Some(b64) => self.image = Some(b64.to_string()),
            None => self.fail(&item["status"]),
        }
    }

    fn fail(&mut self, said: &Value) {
        let said = said.as_str().unwrap_or("no reason given");
        self.failure
            .get_or_insert_with(|| said.chars().take(200).collect());
    }

    pub(super) fn finish(self) -> Result<DrawnPicture> {
        if let Some(b64) = self.image {
            let png = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|_| anyhow!("ChatGPT's picture did not decode."))?;
            return Ok(DrawnPicture { png });
        }
        if let Some(why) = self.failure {
            return Err(anyhow!("ChatGPT did not draw it: {why}"));
        }
        let words: String = self.words.trim().chars().take(200).collect();
        if words.is_empty() {
            return Err(anyhow!("ChatGPT answered without a picture."));
        }
        Err(anyhow!("ChatGPT answered without a picture: {words}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_b64() -> String {
        base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nfake")
    }

    #[test]
    fn a_drawn_item_is_the_picture() {
        let mut c = Collected::default();
        c.take(&json!({"type": "response.created"}));
        c.take(&json!({"type": "response.output_item.done", "item": {"type": "message"}}));
        c.take(&json!({"type": "response.output_item.done",
            "item": {"type": "image_generation_call", "status": "completed", "result": png_b64()}}));
        assert!(c.finish().unwrap().png.starts_with(b"\x89PNG"));
    }

    #[test]
    fn a_failed_stream_says_why_in_one_line() {
        let mut c = Collected::default();
        c.take(&json!({"type": "response.failed",
            "response": {"error": {"message": "You've hit your image limit."}}}));
        let err = c.finish().unwrap_err().to_string();
        assert_eq!(err, "ChatGPT did not draw it: You've hit your image limit.");
    }

    #[test]
    fn words_without_a_picture_are_reported() {
        let mut c = Collected::default();
        c.take(&json!({"type": "response.output_text.delta", "delta": "I can't draw that."}));
        let err = c.finish().unwrap_err().to_string();
        assert!(
            err.contains("without a picture: I can't draw that."),
            "{err}"
        );
    }

    #[test]
    fn refusals_are_one_plain_line() {
        let limit = r#"{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached"}}"#;
        assert_eq!(
            refusal_line(429, limit),
            "ChatGPT's usage limit is reached (429): The usage limit has been reached"
        );
        assert_eq!(
            refusal_line(400, r#"{"detail":"Unsupported model"}"#),
            "ChatGPT refused the picture (400): Unsupported model"
        );
        assert!(refusal_line(401, "").contains("sign in again"));
        assert_eq!(refusal_line(503, ""), "ChatGPT refused the picture (503).");
    }

    #[test]
    fn the_request_asks_for_one_png_at_the_size() {
        let body = request_body(&PictureRequest {
            model: "gpt-6-luna",
            prompt: "a coffee machine",
            size: "1536x1024",
            reference: Some((b"x", "image/png")),
        });
        assert_eq!(body["tools"][0]["type"], "image_generation");
        assert_eq!(body["tools"][0]["size"], "1536x1024");
        assert_eq!(body["stream"], true);
        let content = &body["input"][0]["content"];
        assert_eq!(content[0]["text"], "a coffee machine");
        assert!(content[1]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }
}
