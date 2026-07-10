//! `Browser_*` engine tools — the engine side of `doc/browser-control-spec.md`.
//!
//! Each tool brokers one `control`-module op over the browser bridge
//! (`server::bridge::BridgeHub`) to the `linggen-browser` extension, which
//! drives ONE visible controlled tab through CDP. Targeting is
//! reference-first: `Browser_readPage` returns a node tree with per-node
//! `ref`s; click/type resolve by ref. The engine caches ref → (role, name)
//! from the last read so the safety gate (`engine/browser_gate.rs`) can
//! recognize hard-floor targets (pay / password / delete / post buttons)
//! without another round trip.
//!
//! None of these tools is cacheable — the page is live mutable state.

use super::builtin::Tool;
use super::{ToolCall, ToolResult, Tools};
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

/// Default broker timeout. Navigation gets longer — a cold page load plus
/// the extension's settle delay can exceed the 20s default. Mutating ops get
/// longer still: the extension's permission prompt waits on a human (up to
/// 120s) before the action even starts.
const CALL_TIMEOUT_MS: u64 = 20_000;
const NAVIGATE_TIMEOUT_MS: u64 = 45_000;
const GATED_TIMEOUT_MS: u64 = 150_000;

/// What the engine remembers about one actionable node from the last
/// `read_page` — enough for the safety gate's hard-floor check and for
/// readable permission prompts.
#[derive(Debug, Clone, Default)]
pub struct BrowserNodeMeta {
    pub role: String,
    pub name: String,
}

impl Tools {
    /// Broker one control op over the bridge. `Ok(data)` on success; a
    /// model-readable error otherwise (`no_bridge` gets install guidance).
    pub(crate) async fn browser_call(&self, op: &str, params: Value, timeout_ms: u64) -> Result<Value> {
        let Some(hub) = &self.browser_bridge else {
            anyhow::bail!(
                "browser control is unavailable in this context (no daemon bridge)"
            );
        };
        let res = hub.call_value("control", op, params, timeout_ms).await;
        if res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(res.get("data").cloned().unwrap_or(Value::Null));
        }
        let code = res.get("code").and_then(Value::as_str).unwrap_or("upstream_error");
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

    /// Origin (`scheme://host`) of the controlled tab right now. The safety
    /// gate calls this live before each mutating action — a cached URL could
    /// be stale after a redirect or an in-page navigation. `None` when no
    /// controlled tab exists yet or the bridge is down.
    pub(crate) async fn browser_current_origin(&self) -> Option<String> {
        let data = self
            .browser_call("tabs", json!({ "action": "list" }), 5_000)
            .await
            .ok()?;
        let url = data.get("tabs")?.as_array()?.first()?.get("url")?.as_str()?;
        origin_of(url)
    }

    /// Ref metadata from the last `Browser_readPage`, if any.
    pub(crate) fn browser_ref_meta(&self, ref_id: &str) -> Option<BrowserNodeMeta> {
        self.browser_refs.lock().ok()?.get(ref_id).cloned()
    }

    /// Replace the ref cache with the nodes of a fresh `read_page`.
    fn browser_store_refs(&self, data: &Value) {
        let Ok(mut map) = self.browser_refs.lock() else { return };
        map.clear();
        let Some(nodes) = data.get("nodes").and_then(Value::as_array) else { return };
        for node in nodes {
            let Some(ref_id) = node.get("ref").and_then(Value::as_str) else { continue };
            map.insert(
                ref_id.to_string(),
                BrowserNodeMeta {
                    role: node.get("role").and_then(Value::as_str).unwrap_or_default().to_string(),
                    name: node.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                },
            );
        }
    }
}

/// `scheme://host` of a URL, lowercased. `None` for anything that isn't a
/// well-formed absolute URL with a host.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() || scheme.is_empty() {
        return None;
    }
    Some(format!("{}://{}", scheme.to_lowercase(), host.to_lowercase()))
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(String::from)
}

fn data_str(data: &Value, key: &str) -> String {
    data.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

pub struct BrowserNavigateTool;
#[async_trait]
impl Tool for BrowserNavigateTool {
    fn name(&self) -> &'static str { "Browser_navigate" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_navigate"] }
    fn description(&self) -> &'static str {
        "Load a URL (or go \"back\"/\"forward\") in the controlled browser tab. The tab is visible to the user. Resolves after the page load settles. Follow with Browser_readPage to see the result."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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
    fn name(&self) -> &'static str { "Browser_readPage" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_read_page", "Browser_read_page"] }
    fn description(&self) -> &'static str {
        "Read the controlled tab as an accessibility tree. Actionable nodes carry a ref like [n42] — pass that ref to Browser_click / Browser_type. Re-read after any action that changes the page; old refs go stale."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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
        let data = tools.browser_call("read_page", params, CALL_TIMEOUT_MS).await?;
        tools.browser_store_refs(&data);
        let truncated = data.get("truncated").and_then(Value::as_bool).unwrap_or(false);
        Ok(ToolResult::Success(format!(
            "{} — \"{}\"{}\n\n{}",
            data_str(&data, "url"),
            data_str(&data, "title"),
            if truncated { " (tree truncated — scroll or filter to see more)" } else { "" },
            data_str(&data, "tree"),
        )))
    }
}

pub struct BrowserScreenshotTool;
#[async_trait]
impl Tool for BrowserScreenshotTool {
    fn name(&self) -> &'static str { "Browser_screenshot" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_screenshot"] }
    fn description(&self) -> &'static str {
        "Capture the controlled tab as an image (attached to the conversation). Fallback for visual/canvas content the accessibility tree can't express — prefer Browser_readPage for normal pages."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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
        let data = tools.browser_call("screenshot", params, CALL_TIMEOUT_MS).await?;
        Ok(ToolResult::Screenshot {
            url: data_str(&data, "url"),
            base64: data_str(&data, "base64"),
        })
    }
}

pub struct BrowserClickTool;
#[async_trait]
impl Tool for BrowserClickTool {
    fn name(&self) -> &'static str { "Browser_click" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_click"] }
    fn description(&self) -> &'static str {
        "Click a node by ref (from Browser_readPage) or a viewport coordinate. Prefer refs — coordinates are the screenshot fallback."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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
        let data = tools.browser_call("click", call.args, GATED_TIMEOUT_MS).await?;
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
    fn name(&self) -> &'static str { "Browser_type" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_type"] }
    fn description(&self) -> &'static str {
        "Type text into a field: pass ref to focus it first (clear:true to empty it), or omit ref to type into the currently focused element."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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
        let data = tools.browser_call("type", call.args, GATED_TIMEOUT_MS).await?;
        Ok(ToolResult::Success(format!(
            "typed {} characters",
            data.get("typed").and_then(Value::as_u64).unwrap_or(0),
        )))
    }
}

pub struct BrowserKeyTool;
#[async_trait]
impl Tool for BrowserKeyTool {
    fn name(&self) -> &'static str { "Browser_key" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_key"] }
    fn description(&self) -> &'static str {
        "Press a key or chord in the controlled tab, e.g. \"Enter\", \"Escape\", \"Ctrl+a\", \"Meta+Enter\"."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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
        let data = tools.browser_call("key", call.args, GATED_TIMEOUT_MS).await?;
        Ok(ToolResult::Success(format!("pressed {}", data_str(&data, "pressed"))))
    }
}

pub struct BrowserScrollTool;
#[async_trait]
impl Tool for BrowserScrollTool {
    fn name(&self) -> &'static str { "Browser_scroll" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_scroll"] }
    fn description(&self) -> &'static str {
        "Scroll the page (or the element under ref) in the controlled tab."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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
        let data = tools.browser_call("scroll", call.args, CALL_TIMEOUT_MS).await?;
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
    fn name(&self) -> &'static str { "Browser_wait" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_wait"] }
    fn description(&self) -> &'static str {
        "Wait for the controlled tab to settle before the next read: for \"load\" (page load), \"selector\" (a CSS selector appears, value required), or \"ms\" (fixed delay, max 10000)."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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
        let data = tools.browser_call("wait", call.args, NAVIGATE_TIMEOUT_MS).await?;
        Ok(ToolResult::Success(format!("waited for {}", data_str(&data, "waited"))))
    }
}

pub struct BrowserReadConsoleTool;
#[async_trait]
impl Tool for BrowserReadConsoleTool {
    fn name(&self) -> &'static str { "Browser_readConsole" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_read_console", "Browser_read_console"] }
    fn description(&self) -> &'static str {
        "Read recent console messages from the controlled tab (debugging)."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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
        let data = tools.browser_call("read_console", call.args, CALL_TIMEOUT_MS).await?;
        let messages = data.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
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
    fn name(&self) -> &'static str { "Browser_tabs" }
    fn aliases(&self) -> &'static [&'static str] { &["browser_tabs"] }
    fn description(&self) -> &'static str {
        "Manage the controlled tab: list (current state), open (a URL, creating the tab if needed), switch (bring it to front), close."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn cacheable(&self) -> bool { false }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_of_parses_scheme_and_host() {
        assert_eq!(origin_of("https://x.com/home?a=1"), Some("https://x.com".into()));
        assert_eq!(origin_of("HTTPS://X.com"), Some("https://x.com".into()));
        assert_eq!(
            origin_of("http://localhost:3000/app#x"),
            Some("http://localhost:3000".into())
        );
        assert_eq!(origin_of("about:blank"), None);
        assert_eq!(origin_of("back"), None);
        assert_eq!(origin_of("https://"), None);
    }
}
