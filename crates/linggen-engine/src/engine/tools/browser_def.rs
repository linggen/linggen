//! The browser tools' one definition: `browser_open`, `browser_read` and
//! `browser_act`, each with an `op` — their names, ops, argument schemas,
//! texts, cross-field rules and time limits, written once.
//!
//! Two doors read it and keep no copy: the MCP door (`/mcp`, for Claude Code
//! and Codex) takes [`TOOLS`] and [`BRANCHES`] into its catalogue, and a
//! Linggen session is offered the same tools from [`catalog`]
//! (`browser_tool`). What differs between them is how the answer is worded
//! to the caller, and that each keeps to its own side.

use ling_mem::catalog::{Branch, Catalog, ToolDoc, Views};
use serde_json::json;
use std::sync::LazyLock;

pub const CALL_TIMEOUT_MS: u64 = 20_000;
pub const NAVIGATE_TIMEOUT_MS: u64 = 45_000;
/// Mutating ops wait on the extension's permission prompt (a human, up to
/// 120s) before acting.
pub const GATED_TIMEOUT_MS: u64 = 150_000;

/// How a bridge answer is shaped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Render {
    Text,
    /// An accessibility tree, headed by the page's URL and title.
    Page,
    /// A PNG image content block.
    Image,
}

/// What runs a branch: one op of the bridge's module, and how its answer is
/// shaped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BridgeCall {
    pub module: &'static str,
    pub op: &'static str,
    pub render: Render,
}

/// The tools' own names and texts.
pub const TOOLS: &[ToolDoc] = &[
    ToolDoc {
        name: "browser_open",
        description: "Load a page in the one visible Chrome tab Linggen controls: op=navigate (an absolute URL, or back/forward) or op=tab_open (an absolute URL, creating the tab if needed). Chrome may ask the user to allow this site; they approve or decline there.",
        implicit_op: None,
    },
    ToolDoc {
        name: "browser_read",
        description: "Read the controlled tab: op=page (accessibility tree with refs like n42 for browser_act; refs go stale after the page changes, so read again), op=screenshot (an image, for what the tree cannot show), op=console (recent messages), op=tabs (its state).",
        implicit_op: None,
    },
    ToolDoc {
        name: "browser_act",
        description: "Act in the controlled tab: click (ref or coordinate), type, key, scroll, wait (load, selector, or ms), tab_switch, tab_close. Read the page again after it changes. Chrome may ask the user to allow this site; they approve or decline there.",
        implicit_op: None,
    }
];

/// The branches: every op of the three tools, in the order the tools'
/// schemas list them.
pub const BRANCHES: &[Branch<BridgeCall>] = &[
    Branch {
        tool: "browser_open",
        op: "navigate",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute URL to load, or \"back\" / \"forward\""}
            },
            "required": ["url"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: Some(rules::navigate),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "navigate", render: Render::Text },
    },
    Branch {
        tool: "browser_read",
        op: "page",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "enum": ["all", "interactive"],
                    "description": "\"interactive\" returns only actionable nodes (smaller); default \"all\""
                }
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "read_page", render: Render::Page },
    },
    Branch {
        tool: "browser_act",
        op: "click",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "ref": {"type": "string", "description": "Node ref from browser_read op=page, e.g. \"n42\""},
                "coordinate": {
                    "type": "array", "items": {"type": "number"},
                    "minItems": 2, "maxItems": 2,
                    "description": "Viewport [x, y] — only when no ref exists for the target"
                },
                "button": {"type": "string", "enum": ["left", "middle", "right"]},
                "double": {"type": "boolean", "description": "Double-click"}
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: Some(rules::click),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "click", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "type",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to type"},
                "ref": {"type": "string", "description": "Field ref from browser_read op=page; omit to use current focus"},
                "clear": {"type": "boolean", "description": "Clear the field before typing"}
            },
            "required": ["text"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "type", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "key",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "keys": {"type": "string", "description": "Key or chord, e.g. \"Enter\", \"Tab\", \"Ctrl+a\""},
                "repeat": {"type": "integer", "description": "Press count (default 1, max 20)"}
            },
            "required": ["keys"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: Some(rules::key),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "key", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "scroll",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                "amount": {"type": "integer", "description": "Pixels (default 600)"},
                "ref": {"type": "string", "description": "Scroll at this node instead of page center"}
            },
            "required": ["direction"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "scroll", render: Render::Text },
    },
    Branch {
        tool: "browser_read",
        op: "screenshot",
        note: "",
        schema: || json!({
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
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "screenshot", render: Render::Image },
    },
    Branch {
        tool: "browser_act",
        op: "wait",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "for": {"type": "string", "enum": ["load", "selector", "ms"]},
                "value": {"type": "string", "description": "CSS selector (for=selector) or milliseconds (for=ms)"}
            },
            "required": ["for"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: Some(rules::wait),
        timeout_ms: NAVIGATE_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "wait", render: Render::Text },
    },
    Branch {
        tool: "browser_open",
        op: "tab_open",
        note: "",
        schema: || json!({"type": "object", "properties": {"url": {"type": "string", "description": "Absolute URL to open in the controlled tab, creating it if needed"}}, "required": ["url"]}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[("action", "open")],
        check: Some(rules::tab_open),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: "browser_read",
        op: "tabs",
        note: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[("action", "list")],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "tab_switch",
        note: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[("action", "switch")],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "tab_close",
        note: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[("action", "close")],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: "browser_read",
        op: "console",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer", "description": "Max messages (default 50)"}
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: BridgeCall { module: "control", op: "read_console", render: Render::Text },
    },
];

/// The three tools as a catalogue of their own — what a session is offered.
pub fn catalog() -> &'static Catalog<BridgeCall> {
    static CATALOG: LazyLock<Catalog<BridgeCall>> = LazyLock::new(|| {
        Catalog::new(TOOLS.to_vec(), BRANCHES.to_vec(), Vec::new())
            .expect("the browser tools are consistent")
    });
    &CATALOG
}

/// The cross-field rules the browser branches check before anything runs.
mod rules {
    use serde_json::{Map, Value};

    fn text<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
        obj.get(key).and_then(Value::as_str)
    }

    /// `https://…`, `file:…`, `about:blank`: a scheme, then the rest.
    fn absolute(url: &str) -> bool {
        let Some((scheme, rest)) = url.split_once(':') else {
            return false;
        };
        let mut chars = scheme.chars();
        let starts = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
        let fits = chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
        starts && fits && !rest.is_empty()
    }

    pub(super) fn navigate(obj: &Map<String, Value>) -> Result<(), String> {
        match text(obj, "url") {
            Some("back" | "forward") => Ok(()),
            Some(url) if absolute(url) => Ok(()),
            _ => Err("`url` must be an absolute URL, or \"back\" / \"forward\"".into()),
        }
    }

    pub(super) fn tab_open(obj: &Map<String, Value>) -> Result<(), String> {
        match text(obj, "url") {
            Some(url) if absolute(url) => Ok(()),
            _ => Err("`url` must be an absolute URL".into()),
        }
    }

    pub(super) fn click(obj: &Map<String, Value>) -> Result<(), String> {
        match (obj.contains_key("ref"), obj.contains_key("coordinate")) {
            (true, false) | (false, true) => Ok(()),
            (true, true) => Err("pass `ref` or `coordinate`, not both".into()),
            (false, false) => Err("needs `ref` (from browser_read op=page) or `coordinate`".into()),
        }
    }

    pub(super) fn key(obj: &Map<String, Value>) -> Result<(), String> {
        match obj.get("repeat").and_then(Value::as_i64) {
            Some(n) if !(1..=20).contains(&n) => Err("`repeat` must be 1–20".into()),
            _ => Ok(()),
        }
    }

    pub(super) fn wait(obj: &Map<String, Value>) -> Result<(), String> {
        let value = text(obj, "value");
        match (text(obj, "for"), value) {
            (Some("load"), None) => Ok(()),
            (Some("load"), Some(_)) => Err("for=load takes no `value`".into()),
            (Some("selector"), Some(_)) => Ok(()),
            (Some("selector"), None) => Err("for=selector needs `value`, a CSS selector".into()),
            (Some("ms"), Some(ms)) => match ms.trim().parse::<u64>() {
                Ok(n) if n <= 10_000 => Ok(()),
                _ => Err("for=ms needs `value`, milliseconds 0–10000 as a string".into()),
            },
            (Some("ms"), None) => Err("for=ms needs `value`, milliseconds 0–10000".into()),
            _ => Ok(()),
        }
    }
}

