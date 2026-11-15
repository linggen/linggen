//! Carrying one JSON-RPC request to a server and bringing the reply back.
//!
//! Two transports, because those are the two the ecosystem actually ships:
//! stdio (most servers) and streamable HTTP (ours). Deliberately not WebRTC —
//! the *client* chooses the transport and we do not own Claude Code's, so a
//! dialect only we speak would defeat the point of being reachable.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

/// Ceiling on one request. A server that has not answered in this long is a
/// server we report as unreachable — never one that owns the turn.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Send one JSON-RPC request and return its `result`, or the error the
    /// server reported.
    async fn request(&self, body: Value) -> Result<Value>;

    /// Send a notification — no `id`, no reply expected.
    async fn notify(&self, body: Value) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Streamable HTTP
// ---------------------------------------------------------------------------

pub struct HttpTransport {
    url: String,
    headers: BTreeMap<String, String>,
    client: reqwest::Client,
    /// Set from the server's `Mcp-Session-Id` on initialize, echoed after.
    /// Stateless servers (ours) never send one, and then this stays empty —
    /// which is why initialize is allowed to be a no-op rather than required.
    session: Mutex<Option<String>>,
}

impl HttpTransport {
    pub fn new(url: String, headers: BTreeMap<String, String>) -> Result<Self> {
        Ok(Self {
            url,
            headers,
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .context("building the MCP http client")?,
            session: Mutex::new(None),
        })
    }

    async fn post(&self, body: &Value) -> Result<reqwest::Response> {
        let mut req = self
            .client
            .post(&self.url)
            .header("content-type", "application/json")
            // Streamable HTTP lets a server answer with either, and a server
            // that sees only one may refuse.
            .header("accept", "application/json, text/event-stream");
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        if let Some(sid) = self.session.lock().await.as_deref() {
            req = req.header("mcp-session-id", sid);
        }
        req.json(body)
            .send()
            .await
            .context("posting to the MCP server")
    }
}

#[async_trait::async_trait]
impl Transport for HttpTransport {
    async fn request(&self, body: Value) -> Result<Value> {
        let res = self.post(&body).await?;
        let status = res.status();
        if let Some(sid) = res
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *self.session.lock().await = Some(sid.to_string());
        }
        let text = res.text().await.context("reading the MCP response")?;
        if !status.is_success() {
            bail!(
                "HTTP {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        unwrap_result(parse_maybe_sse(&text)?)
    }

    async fn notify(&self, body: Value) -> Result<()> {
        // A notification's reply is 202/204 with no body; nothing to read.
        self.post(&body).await?;
        Ok(())
    }
}

/// A streamable-HTTP server may answer a single request as plain JSON or as
/// one `text/event-stream` frame. Accept both rather than making the caller
/// care which a given server chose.
fn parse_maybe_sse(text: &str) -> Result<Value> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).context("parsing the MCP JSON reply");
    }
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            return serde_json::from_str(data).context("parsing the MCP SSE frame");
        }
    }
    bail!(
        "unrecognised MCP reply: {}",
        text.chars().take(200).collect::<String>()
    )
}

// ---------------------------------------------------------------------------
// stdio
// ---------------------------------------------------------------------------

/// A child process speaking line-delimited JSON-RPC on its stdin/stdout.
///
/// One mutex over the whole exchange: stdio has no request ids to demultiplex
/// on in practice, so requests are serialised rather than interleaved. These
/// are tool calls, not a hot loop.
pub struct StdioTransport {
    io: Mutex<StdioPipes>,
    /// Held so the child is killed when the transport is dropped.
    _child: Arc<Mutex<Child>>,
}

struct StdioPipes {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl StdioTransport {
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The server's own logs belong in ours, not interleaved into the
            // protocol stream.
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting MCP server `{command}`"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        Ok(Self {
            io: Mutex::new(StdioPipes {
                stdin,
                stdout: BufReader::new(stdout),
            }),
            _child: Arc::new(Mutex::new(child)),
        })
    }

    async fn write_line(pipes: &mut StdioPipes, body: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(body)?;
        line.push(b'\n');
        pipes
            .stdin
            .write_all(&line)
            .await
            .context("writing to the MCP server")?;
        pipes
            .stdin
            .flush()
            .await
            .context("flushing to the MCP server")?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Transport for StdioTransport {
    async fn request(&self, body: Value) -> Result<Value> {
        let mut pipes = self.io.lock().await;
        Self::write_line(&mut pipes, &body).await?;

        // Skip anything that isn't our answer: servers emit notifications and
        // the occasional stray line, and neither should be mistaken for one.
        let want = body.get("id").cloned();
        let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
        loop {
            let mut line = String::new();
            let read = tokio::time::timeout_at(deadline, pipes.stdout.read_line(&mut line))
                .await
                .map_err(|_| anyhow!("MCP server did not answer in {REQUEST_TIMEOUT:?}"))?
                .context("reading from the MCP server")?;
            if read == 0 {
                bail!("MCP server closed its output");
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(msg) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if msg.get("id") == want.as_ref() {
                return unwrap_result(msg);
            }
        }
    }

    async fn notify(&self, body: Value) -> Result<()> {
        let mut pipes = self.io.lock().await;
        Self::write_line(&mut pipes, &body).await
    }
}

// ---------------------------------------------------------------------------

/// Pull `result` out of a JSON-RPC envelope, turning `error` into ours.
fn unwrap_result(msg: Value) -> Result<Value> {
    if let Some(err) = msg.get("error") {
        let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        bail!("MCP error {code}: {message}");
    }
    Ok(msg.get("result").cloned().unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_json_and_sse_both_parse() {
        let want = json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}});
        assert_eq!(
            parse_maybe_sse(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#).unwrap(),
            want
        );
        assert_eq!(
            parse_maybe_sse(
                "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n"
            )
            .unwrap(),
            want
        );
    }

    #[test]
    fn a_server_error_surfaces_as_an_error_not_a_result() {
        let err = unwrap_result(json!({"error":{"code":-32602,"message":"bad args"}}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("-32602"), "{err}");
        assert!(err.contains("bad args"), "{err}");
    }

    #[test]
    fn a_missing_result_is_null_not_a_failure() {
        // Notifications and empty acks are legal; they must not read as errors.
        assert_eq!(
            unwrap_result(json!({"jsonrpc":"2.0","id":1})).unwrap(),
            Value::Null
        );
    }
}
