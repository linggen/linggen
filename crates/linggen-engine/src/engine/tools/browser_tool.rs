//! `Browser_*` engine tools — the engine side of `doc/browser-control-spec.md`.
//!
//! Each tool brokers one `control`-module op over the browser bridge
//! (a [`BrowserBridge`], served by the daemon's `BridgeHub`) to the `linggen-browser` extension, which
//! drives ONE visible controlled tab through CDP. Targeting is
//! reference-first: `Browser_readPage` returns a node tree with per-node
//! `ref`s; click/type resolve by ref. Mutating actions are gated by the
//! extension's own permission prompt (per-origin trust + hard floor).
//!
//! None of these tools is cacheable — the page is live mutable state.

use super::builtin::Tool;
use super::{ToolCall, ToolResult, Tools};
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

/// A line of progress an op reports while it waits (e.g. "waiting for your OK").
pub type BridgeProgress = Box<dyn Fn(String) + Send + Sync>;

/// The transport the `Browser_*` tools broker their ops over. The daemon
/// implements it (its extension bridge); the engine only needs this seam.
/// The reply is the bridge envelope: `{ok:true, data}` or
/// `{ok:false, code, message}`.
#[async_trait]
pub trait BrowserBridge: Send + Sync {
    async fn call(
        &self,
        module: &str,
        op: &str,
        params: Value,
        timeout_ms: u64,
        on_progress: Option<BridgeProgress>,
    ) -> Value;
}

/// Default broker timeout. Navigation gets longer — a cold page load plus
/// the extension's settle delay can exceed the 20s default. Mutating ops get
/// longer still: the extension's permission prompt waits on a human (up to
/// 120s) before the action even starts.
const CALL_TIMEOUT_MS: u64 = 20_000;
const NAVIGATE_TIMEOUT_MS: u64 = 45_000;
const GATED_TIMEOUT_MS: u64 = 150_000;

impl Tools {
    /// Broker one control op over the bridge. `Ok(data)` on success; a
    /// model-readable error otherwise (`no_bridge` gets install guidance).
    pub(crate) async fn browser_call(
        &self,
        op: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value> {
        let Some(hub) = &self.browser_bridge else {
            anyhow::bail!("browser control is unavailable in this context (no daemon bridge)");
        };
        // The extension says when an op waits — on the user's OK in its
        // approval popup, or behind another browser action — and that line
        // rides the running tool, so a two-minute wait reads as one.
        let on_progress = self.progress_tx.clone().map(|tx| {
            let tool = format!("Browser_{op}");
            Box::new(move |line: String| {
                let _ = tx.send((tool.clone(), "status".to_string(), line));
            }) as BridgeProgress
        });
        let res = hub
            .call("control", op, params, timeout_ms, on_progress)
            .await;
        if res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(res.get("data").cloned().unwrap_or(Value::Null));
        }
        let code = res
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("upstream_error");
        let message = res.get("message").and_then(Value::as_str).unwrap_or("");
        match code {
            "no_bridge" | "module_unavailable" => anyhow::bail!(
                "browser not connected — the linggen-browser extension (with the control \
                 module) must be running in Chrome. Ask the user to install or enable it, \
                 then retry."
            ),
            "element_gone" => anyhow::bail!(
                "element_gone: {message}. The page changed — call Browser_readPage again \
                 and use a fresh ref."
            ),
            _ => anyhow::bail!("{code}: {message}"),
        }
    }
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(String::from)
}

fn data_str(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

pub struct BrowserNavigateTool;
#[async_trait]
impl Tool for BrowserNavigateTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_navigate"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_navigate"]
    }
    fn description(&self) -> &'static str {
        "Load a URL (or go \"back\"/\"forward\") in the controlled browser tab. The tab is visible to the user. Resolves after the page load settles. Follow with Browser_readPage to see the result."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute URL to load, or \"back\" / \"forward\""}
            },
            "required": ["url"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_navigate",
            "args": {"url": "string"},
            "returns": "{url,title}",
            "notes": "Load a URL or go back/forward in the controlled browser tab (visible to the user)."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let url = opt_str(&call.args, "url")
            .ok_or_else(|| anyhow::anyhow!("Browser_navigate requires url"))?;
        let data = tools
            .browser_call("navigate", json!({ "url": url }), GATED_TIMEOUT_MS)
            .await?;
        Ok(ToolResult::Success(format!(
            "navigated to {} — \"{}\"",
            data_str(&data, "url"),
            data_str(&data, "title"),
        )))
    }
}

pub struct BrowserReadPageTool;
#[async_trait]
impl Tool for BrowserReadPageTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_readPage"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_read_page", "Browser_read_page"]
    }
    fn description(&self) -> &'static str {
        "Read the controlled tab as an accessibility tree. Actionable nodes carry a ref like [n42] — pass that ref to Browser_click / Browser_type. Re-read after any action that changes the page; old refs go stale."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "enum": ["all", "interactive"],
                    "description": "\"interactive\" returns only actionable nodes (smaller); default \"all\" includes structure and text"
                }
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_readPage",
            "args": {"filter": "string?"},
            "returns": "{url,title,tree}",
            "notes": "Accessibility tree of the controlled tab; [nN] refs target Browser_click/Browser_type."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let mut params = json!({});
        if let Some(filter) = opt_str(&call.args, "filter") {
            params["filter"] = json!(filter);
        }
        let data = tools
            .browser_call("read_page", params, CALL_TIMEOUT_MS)
            .await?;
        let truncated = data
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(ToolResult::Success(format!(
            "{} — \"{}\"{}\n\n{}",
            data_str(&data, "url"),
            data_str(&data, "title"),
            if truncated {
                " (tree truncated — scroll or filter to see more)"
            } else {
                ""
            },
            data_str(&data, "tree"),
        )))
    }
}

pub struct BrowserScreenshotTool;
#[async_trait]
impl Tool for BrowserScreenshotTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_screenshot"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_screenshot"]
    }
    fn description(&self) -> &'static str {
        "Capture the controlled tab as an image (attached to the conversation). Fallback for visual/canvas content the accessibility tree can't express — prefer Browser_readPage for normal pages."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "region": {
                    "type": "object",
                    "properties": {
                        "x": {"type": "number"}, "y": {"type": "number"},
                        "width": {"type": "number"}, "height": {"type": "number"}
                    },
                    "description": "Optional viewport region to capture; omit for the full viewport"
                }
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_screenshot",
            "args": {"region": "{x,y,width,height}?"},
            "returns": "{url,base64}",
            "notes": "Screenshot of the controlled tab; the image is attached to the conversation."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let mut params = json!({});
        if let Some(region) = call.args.get("region") {
            params["region"] = region.clone();
        }
        let data = tools
            .browser_call("screenshot", params, CALL_TIMEOUT_MS)
            .await?;
        Ok(ToolResult::Screenshot {
            url: data_str(&data, "url"),
            base64: data_str(&data, "base64"),
            id: String::new(),
        })
    }
}

pub struct BrowserClickTool;
#[async_trait]
impl Tool for BrowserClickTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_click"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_click"]
    }
    fn description(&self) -> &'static str {
        "Click a node by ref (from Browser_readPage) or a viewport coordinate. Prefer refs — coordinates are the screenshot fallback."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ref": {"type": "string", "description": "Node ref from Browser_readPage, e.g. \"n42\""},
                "coordinate": {
                    "type": "array", "items": {"type": "number"},
                    "minItems": 2, "maxItems": 2,
                    "description": "Viewport [x, y] — only when no ref exists for the target"
                },
                "button": {"type": "string", "enum": ["left", "middle", "right"]},
                "double": {"type": "boolean", "description": "Double-click"}
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_click",
            "args": {"ref": "string?", "coordinate": "[number,number]?", "button": "string?", "double": "boolean?"},
            "returns": "{clicked,target,url}",
            "notes": "Click by ref (preferred) or coordinate in the controlled tab."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let data = tools
            .browser_call("click", call.args, GATED_TIMEOUT_MS)
            .await?;
        Ok(ToolResult::Success(format!(
            "clicked {} — page is now {}",
            data_str(&data, "target"),
            data_str(&data, "url"),
        )))
    }
}

pub struct BrowserTypeTool;
#[async_trait]
impl Tool for BrowserTypeTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_type"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_type"]
    }
    fn description(&self) -> &'static str {
        "Type text into a field: pass ref to focus it first (clear:true to empty it), or omit ref to type into the currently focused element."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to type"},
                "ref": {"type": "string", "description": "Field ref from Browser_readPage; omit to use current focus"},
                "clear": {"type": "boolean", "description": "Clear the field before typing"}
            },
            "required": ["text"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_type",
            "args": {"text": "string", "ref": "string?", "clear": "boolean?"},
            "returns": "{typed}",
            "notes": "Type into the referenced field (or current focus) in the controlled tab."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let data = tools
            .browser_call("type", call.args, GATED_TIMEOUT_MS)
            .await?;
        Ok(ToolResult::Success(format!(
            "typed {} characters",
            data.get("typed").and_then(Value::as_u64).unwrap_or(0),
        )))
    }
}

pub struct BrowserKeyTool;
#[async_trait]
impl Tool for BrowserKeyTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_key"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_key"]
    }
    fn description(&self) -> &'static str {
        "Press a key or chord in the controlled tab, e.g. \"Enter\", \"Escape\", \"Ctrl+a\", \"Meta+Enter\"."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "keys": {"type": "string", "description": "Key or chord, e.g. \"Enter\", \"Tab\", \"Ctrl+a\""},
                "repeat": {"type": "integer", "description": "Press count (default 1, max 20)"}
            },
            "required": ["keys"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_key",
            "args": {"keys": "string", "repeat": "number?"},
            "returns": "{pressed}",
            "notes": "Press a key or chord (Enter, Escape, Ctrl+a, ...) in the controlled tab."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let data = tools
            .browser_call("key", call.args, GATED_TIMEOUT_MS)
            .await?;
        Ok(ToolResult::Success(format!(
            "pressed {}",
            data_str(&data, "pressed")
        )))
    }
}

pub struct BrowserScrollTool;
#[async_trait]
impl Tool for BrowserScrollTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_scroll"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_scroll"]
    }
    fn description(&self) -> &'static str {
        "Scroll the page (or the element under ref) in the controlled tab."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                "amount": {"type": "integer", "description": "Pixels (default 600)"},
                "ref": {"type": "string", "description": "Scroll at this node instead of page center"}
            },
            "required": ["direction"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_scroll",
            "args": {"direction": "string", "amount": "number?", "ref": "string?"},
            "returns": "{scrolled,amount}",
            "notes": "Scroll the controlled tab (or the element at ref)."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let data = tools
            .browser_call("scroll", call.args, CALL_TIMEOUT_MS)
            .await?;
        Ok(ToolResult::Success(format!(
            "scrolled {} by {}px",
            data_str(&data, "scrolled"),
            data.get("amount").and_then(Value::as_u64).unwrap_or(0),
        )))
    }
}

pub struct BrowserWaitTool;
#[async_trait]
impl Tool for BrowserWaitTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_wait"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_wait"]
    }
    fn description(&self) -> &'static str {
        "Wait for the controlled tab to settle before the next read: for \"load\" (page load), \"selector\" (a CSS selector appears, value required), or \"ms\" (fixed delay, max 10000)."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "for": {"type": "string", "enum": ["load", "selector", "ms"]},
                "value": {"type": "string", "description": "CSS selector (for=selector) or milliseconds (for=ms)"}
            },
            "required": ["for"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_wait",
            "args": {"for": "string", "value": "string?"},
            "returns": "{waited}",
            "notes": "Wait for load / a CSS selector / a fixed delay in the controlled tab."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let data = tools
            .browser_call("wait", call.args, NAVIGATE_TIMEOUT_MS)
            .await?;
        Ok(ToolResult::Success(format!(
            "waited for {}",
            data_str(&data, "waited")
        )))
    }
}

pub struct BrowserReadConsoleTool;
#[async_trait]
impl Tool for BrowserReadConsoleTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_readConsole"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_read_console", "Browser_read_console"]
    }
    fn description(&self) -> &'static str {
        "Read recent console messages from the controlled tab (debugging)."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer", "description": "Max messages (default 50)"}
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_readConsole",
            "args": {"limit": "number?"},
            "returns": "{messages:[{level,text}]}",
            "notes": "Recent console output of the controlled tab."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let data = tools
            .browser_call("read_console", call.args, CALL_TIMEOUT_MS)
            .await?;
        let messages = data
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if messages.is_empty() {
            return Ok(ToolResult::Success("console is empty".to_string()));
        }
        let lines: Vec<String> = messages
            .iter()
            .map(|m| {
                format!(
                    "[{}] {}",
                    m.get("level").and_then(Value::as_str).unwrap_or("log"),
                    m.get("text").and_then(Value::as_str).unwrap_or(""),
                )
            })
            .collect();
        Ok(ToolResult::Success(lines.join("\n")))
    }
}

pub struct BrowserTabsTool;
#[async_trait]
impl Tool for BrowserTabsTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        "Browser_tabs"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["browser_tabs"]
    }
    fn description(&self) -> &'static str {
        "Manage the controlled tab: list (current state), open (a URL, creating the tab if needed), switch (bring it to front), close."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "open", "switch", "close"]},
                "url": {"type": "string", "description": "URL for action=open"}
            },
            "required": ["action"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Browser_tabs",
            "args": {"action": "string", "url": "string?"},
            "returns": "{tabs}|{url,title}|{closed}",
            "notes": "list / open / switch / close the controlled browser tab."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let timeout = if call.args.get("action").and_then(Value::as_str) == Some("open") {
            GATED_TIMEOUT_MS
        } else {
            CALL_TIMEOUT_MS
        };
        let data = tools.browser_call("tabs", call.args, timeout).await?;
        Ok(ToolResult::Success(data.to_string()))
    }
}

/// Today's behaviour of the ten tools, pinned before they move: what each
/// asks the bridge (op, params, time limit), what it answers the model, how
/// it fails, and what the engine knows of it around the call (tier, reach,
/// cache, time ceiling, status line, progress label).
#[cfg(test)]
mod pins {
    use super::*;
    use crate::engine::permission::PermissionMode;
    use crate::engine::tools::PlaceReach;
    use std::sync::{Arc, Mutex};

    /// Every call the tools made over the bridge, and a reply to give.
    struct FakeBridge {
        seen: Mutex<Vec<(String, String, Value, u64)>>,
        reply: Value,
        progress: Option<&'static str>,
    }

    #[async_trait]
    impl BrowserBridge for FakeBridge {
        async fn call(
            &self,
            module: &str,
            op: &str,
            params: Value,
            timeout_ms: u64,
            on_progress: Option<BridgeProgress>,
        ) -> Value {
            self.seen
                .lock()
                .unwrap()
                .push((module.into(), op.into(), params, timeout_ms));
            if let (Some(line), Some(report)) = (self.progress, on_progress) {
                report(line.to_string());
            }
            self.reply.clone()
        }
    }

    fn tools_with(reply: Value, progress: Option<&'static str>) -> (Tools, Arc<FakeBridge>) {
        let bridge = Arc::new(FakeBridge {
            seen: Mutex::new(Vec::new()),
            reply,
            progress,
        });
        let mut tools = Tools::new(std::path::PathBuf::from(".")).unwrap();
        tools.set_browser_bridge(bridge.clone());
        (tools, bridge)
    }

    fn ok(data: Value) -> Value {
        json!({"ok": true, "data": data})
    }

    async fn run(tools: &Tools, tool: &str, args: Value) -> Result<ToolResult> {
        tools
            .execute(ToolCall {
                tool: tool.into(),
                args,
                block_id: None,
            })
            .await
    }

    fn said(result: Result<ToolResult>) -> String {
        match result.expect("the call ran") {
            ToolResult::Success(text) => text,
            other => panic!("not a text answer: {other:?}"),
        }
    }

    /// One call of one old tool: its args, the bridge op and params it
    /// sends, the limit it waits, the bridge's data and the model's text.
    struct Case {
        tool: &'static str,
        args: Value,
        op: &'static str,
        sent: Value,
        timeout_ms: u64,
        data: Value,
        text: &'static str,
    }

    fn cases() -> Vec<Case> {
        let page = || json!({"url": "https://a.b/x", "title": "T", "tree": "[n1] link"});
        vec![
            Case {
                tool: "Browser_navigate",
                args: json!({"url": "https://a.b/"}),
                op: "navigate",
                sent: json!({"url": "https://a.b/"}),
                timeout_ms: 150_000,
                data: json!({"url": "https://a.b/x", "title": "T"}),
                text: "navigated to https://a.b/x — \"T\"",
            },
            Case {
                tool: "Browser_readPage",
                args: json!({}),
                op: "read_page",
                sent: json!({}),
                timeout_ms: 20_000,
                data: page(),
                text: "https://a.b/x — \"T\"\n\n[n1] link",
            },
            Case {
                tool: "Browser_readPage",
                args: json!({"filter": "interactive"}),
                op: "read_page",
                sent: json!({"filter": "interactive"}),
                timeout_ms: 20_000,
                data: json!({"url": "u", "title": "T", "tree": "t", "truncated": true}),
                text: "u — \"T\" (tree truncated — scroll or filter to see more)\n\nt",
            },
            Case {
                tool: "Browser_click",
                args: json!({"ref": "n42", "button": "right", "double": true}),
                op: "click",
                sent: json!({"ref": "n42", "button": "right", "double": true}),
                timeout_ms: 150_000,
                data: json!({"target": "button Go", "url": "https://a.b/y"}),
                text: "clicked button Go — page is now https://a.b/y",
            },
            Case {
                tool: "Browser_type",
                args: json!({"text": "hello", "ref": "n2", "clear": true}),
                op: "type",
                sent: json!({"text": "hello", "ref": "n2", "clear": true}),
                timeout_ms: 150_000,
                data: json!({"typed": 5}),
                text: "typed 5 characters",
            },
            Case {
                tool: "Browser_key",
                args: json!({"keys": "Enter", "repeat": 3}),
                op: "key",
                sent: json!({"keys": "Enter", "repeat": 3}),
                timeout_ms: 150_000,
                data: json!({"pressed": "Enter"}),
                text: "pressed Enter",
            },
            Case {
                tool: "Browser_scroll",
                args: json!({"direction": "down", "amount": 300, "ref": "n3"}),
                op: "scroll",
                sent: json!({"direction": "down", "amount": 300, "ref": "n3"}),
                timeout_ms: 20_000,
                data: json!({"scrolled": "down", "amount": 300}),
                text: "scrolled down by 300px",
            },
            Case {
                tool: "Browser_wait",
                args: json!({"for": "ms", "value": "500"}),
                op: "wait",
                sent: json!({"for": "ms", "value": "500"}),
                timeout_ms: 45_000,
                data: json!({"waited": "ms"}),
                text: "waited for ms",
            },
            Case {
                tool: "Browser_readConsole",
                args: json!({"limit": 5}),
                op: "read_console",
                sent: json!({"limit": 5}),
                timeout_ms: 20_000,
                data: json!({"messages": [{"level": "error", "text": "boom"}, {"text": "hi"}]}),
                text: "[error] boom\n[log] hi",
            },
            Case {
                tool: "Browser_readConsole",
                args: json!({}),
                op: "read_console",
                sent: json!({}),
                timeout_ms: 20_000,
                data: json!({"messages": []}),
                text: "console is empty",
            },
            Case {
                tool: "Browser_tabs",
                args: json!({"action": "list"}),
                op: "tabs",
                sent: json!({"action": "list"}),
                timeout_ms: 20_000,
                data: json!({"tabs": []}),
                text: "{\"tabs\":[]}",
            },
            Case {
                tool: "Browser_tabs",
                args: json!({"action": "open", "url": "https://a.b/"}),
                op: "tabs",
                sent: json!({"action": "open", "url": "https://a.b/"}),
                timeout_ms: 150_000,
                data: json!({"url": "https://a.b/", "title": "T"}),
                text: "{\"url\":\"https://a.b/\",\"title\":\"T\"}",
            },
            Case {
                tool: "Browser_tabs",
                args: json!({"action": "switch"}),
                op: "tabs",
                sent: json!({"action": "switch"}),
                timeout_ms: 20_000,
                data: json!({"url": "u", "title": "T"}),
                text: "{\"url\":\"u\",\"title\":\"T\"}",
            },
            Case {
                tool: "Browser_tabs",
                args: json!({"action": "close"}),
                op: "tabs",
                sent: json!({"action": "close"}),
                timeout_ms: 20_000,
                data: json!({"closed": true}),
                text: "{\"closed\":true}",
            },
        ]
    }

    #[tokio::test]
    async fn each_tool_asks_the_bridge_for_its_op_and_tells_the_model_what_came_back() {
        for case in cases() {
            let (tools, bridge) = tools_with(ok(case.data.clone()), None);
            let text = said(run(&tools, case.tool, case.args.clone()).await);
            let seen = bridge.seen.lock().unwrap().clone();
            let want = vec![("control".to_string(), case.op.to_string(), case.sent.clone(), case.timeout_ms)];
            assert_eq!(seen, want, "{} {}", case.tool, case.args);
            assert_eq!(text, case.text, "{} {}", case.tool, case.args);
        }
    }

    #[tokio::test]
    async fn a_screenshot_is_an_image_of_the_page_with_the_region_it_was_asked() {
        let reply = ok(json!({"url": "https://a.b/", "base64": "AAAA"}));
        let (tools, bridge) = tools_with(reply.clone(), None);
        let region = json!({"x": 1, "y": 2, "width": 3, "height": 4});
        let shot = run(&tools, "Browser_screenshot", json!({"region": region})).await;
        match shot.expect("the call ran") {
            ToolResult::Screenshot { url, base64, id } => {
                assert_eq!((url.as_str(), base64.as_str(), id.as_str()), ("https://a.b/", "AAAA", ""));
            }
            other => panic!("not a screenshot: {other:?}"),
        }
        let seen = bridge.seen.lock().unwrap().clone();
        let want = vec![("control".to_string(), "screenshot".to_string(), json!({"region": region}), 20_000)];
        assert_eq!(seen, want);
        // No region: none is sent.
        let (tools, bridge) = tools_with(reply, None);
        run(&tools, "Browser_screenshot", json!({})).await.unwrap();
        assert_eq!(bridge.seen.lock().unwrap()[0].2, json!({}));
    }

    #[tokio::test]
    async fn a_failure_reads_the_way_it_does_today() {
        let fail = |code: &str, message: &str| json!({"ok": false, "code": code, "message": message});
        let said_err = |reply: Value, tool: &'static str, args: Value| async move {
            let (tools, _) = tools_with(reply, None);
            run(&tools, tool, args).await.unwrap_err().to_string()
        };
        let not_connected = "browser not connected — the linggen-browser extension (with the control \
             module) must be running in Chrome. Ask the user to install or enable it, \
             then retry.";
        for code in ["no_bridge", "module_unavailable"] {
            let got = said_err(fail(code, "x"), "Browser_readPage", json!({})).await;
            assert_eq!(got, not_connected, "{code}");
        }
        let got = said_err(fail("element_gone", "n9"), "Browser_click", json!({"ref": "n9"})).await;
        assert_eq!(
            got,
            "element_gone: n9. The page changed — call Browser_readPage again and use a fresh ref."
        );
        let got = said_err(fail("not_permitted", "no"), "Browser_type", json!({"text": "a"})).await;
        assert_eq!(got, "not_permitted: no");
        let got = said_err(json!({"ok": false}), "Browser_key", json!({"keys": "a"})).await;
        assert_eq!(got, "upstream_error: ");
        // No bridge wired (CLI, evals).
        let mut tools = Tools::new(std::path::PathBuf::from(".")).unwrap();
        tools.browser_bridge = None;
        let got = run(&tools, "Browser_tabs", json!({"action": "list"})).await.unwrap_err();
        assert_eq!(got.to_string(), "browser control is unavailable in this context (no daemon bridge)");
        // A navigate with no url is refused before the bridge is asked.
        let (tools, bridge) = tools_with(ok(json!({})), None);
        let got = run(&tools, "Browser_navigate", json!({})).await.unwrap_err();
        assert_eq!(got.to_string(), "Browser_navigate requires url");
        assert!(bridge.seen.lock().unwrap().is_empty());
    }

    /// The bridge's progress line rides the running tool, under the label
    /// `Browser_<bridge op>`.
    #[tokio::test]
    async fn progress_rides_the_running_tool_under_its_bridge_op() {
        for case in cases() {
            let (mut tools, _) = tools_with(ok(case.data.clone()), Some("waiting for your OK"));
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            tools.set_progress_tx(tx);
            run(&tools, case.tool, case.args.clone()).await.unwrap();
            let got = rx.try_recv().expect("a progress line");
            let label = format!("Browser_{}", case.op);
            assert_eq!(got, (label, "status".to_string(), "waiting for your OK".to_string()));
        }
    }

    /// What the engine knows of each tool around the call.
    #[test]
    fn each_tool_is_read_tier_stays_in_place_and_is_never_cached() {
        for name in OLD_NAMES {
            let tool = super::super::builtin::lookup(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(tool.name(), name);
            assert_eq!(tool.tier(), PermissionMode::Read, "{name}");
            assert_eq!(tool.place_reach(), PlaceReach::Stays, "{name}");
            assert!(!tool.cacheable(), "{name}");
            assert!(tool.model_facing() && tool.offered().is_ok(), "{name}");
            assert_eq!(tool.max_duration(), Some(std::time::Duration::from_secs(300)), "{name}");
            assert_eq!(crate::engine::tools::builtin_tier(name), Some(PermissionMode::Read), "{name}");
        }
    }

    const OLD_NAMES: [&str; 10] = [
        "Browser_navigate",
        "Browser_readPage",
        "Browser_screenshot",
        "Browser_click",
        "Browser_type",
        "Browser_key",
        "Browser_scroll",
        "Browser_wait",
        "Browser_readConsole",
        "Browser_tabs",
    ];

    /// The session is sent the ten, in this order, each with a schema.
    #[test]
    fn a_session_is_offered_the_ten_in_this_order() {
        let offered: Vec<String> = super::super::json_schema::oai_tool_definitions(None)
            .iter()
            .filter_map(|d| d["function"]["name"].as_str().map(String::from))
            .filter(|n| n.starts_with("Browser_"))
            .collect();
        assert_eq!(offered, OLD_NAMES);
    }

    /// The names a declaration or a stale call may use, and the delegate's
    /// fixed list: the ten, each its own tool.
    #[test]
    fn the_ten_names_resolve_and_the_delegate_holds_them() {
        let aliases = [
            ("browser_navigate", "Browser_navigate"),
            ("browser_read_page", "Browser_readPage"),
            ("Browser_read_page", "Browser_readPage"),
            ("browser_screenshot", "Browser_screenshot"),
            ("browser_click", "Browser_click"),
            ("browser_type", "Browser_type"),
            ("browser_key", "Browser_key"),
            ("browser_scroll", "Browser_scroll"),
            ("browser_wait", "Browser_wait"),
            ("browser_read_console", "Browser_readConsole"),
            ("Browser_read_console", "Browser_readConsole"),
            ("browser_tabs", "Browser_tabs"),
        ];
        for (alias, name) in aliases {
            assert_eq!(crate::engine::tools::canonical_tool_name(alias), Some(name), "{alias}");
        }
        let delegate = crate::engine::session_spec::delegate::tool_scope(false).unwrap();
        for name in OLD_NAMES {
            assert!(delegate.contains(name), "{name}");
        }
        let scope = crate::engine::tool_scope::compute_tool_scope(&["Browser_click".to_string()]);
        assert_eq!(scope.unwrap().into_iter().collect::<Vec<_>>(), ["Browser_click"]);
    }

    /// The line a session's chat shows for each, before, during and after.
    #[test]
    fn each_tool_has_its_status_line() {
        use crate::engine::tool_render::{tool_status_line, ToolStatusPhase::*};
        let rows: [(&str, Value, [&str; 3]); 10] = [
            ("Browser_navigate", json!({"url": "https://a.b/"}), ["Browsing to: https://a.b/", "Browsed to: https://a.b/", "Navigation failed: https://a.b/"]),
            ("Browser_readPage", json!({}), ["Reading page", "Read page", "Page read failed"]),
            ("Browser_screenshot", json!({}), ["Capturing page", "Captured page", "Capture failed"]),
            ("Browser_click", json!({"ref": "n4"}), ["Clicking: n4", "Clicked: n4", "Click failed: n4"]),
            ("Browser_type", json!({"text": "hi"}), ["Typing: hi", "Typed: hi", "Typing failed: hi"]),
            ("Browser_key", json!({"keys": "Enter"}), ["Pressing: Enter", "Pressed: Enter", "Key press failed: Enter"]),
            ("Browser_scroll", json!({"direction": "down"}), ["Scrolling: down", "Scrolled: down", "Scroll failed: down"]),
            ("Browser_wait", json!({"for": "load"}), ["Waiting for page: load", "Waited for page: load", "Wait failed: load"]),
            ("Browser_readConsole", json!({}), ["Reading console", "Read console", "Console read failed"]),
            ("Browser_tabs", json!({"action": "list"}), ["Browser tab: list", "Browser tab: list", "Browser tab action failed: list"]),
        ];
        for (tool, args, [start, done, failed]) in rows {
            assert_eq!(tool_status_line(tool, Some(&args), Start), start, "{tool}");
            assert_eq!(tool_status_line(tool, Some(&args), Done), done, "{tool}");
            assert_eq!(tool_status_line(tool, Some(&args), Failed), failed, "{tool}");
        }
    }
}
