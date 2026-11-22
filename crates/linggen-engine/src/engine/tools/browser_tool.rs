//! The browser tools a session is offered: `browser_open`, `browser_read` and
//! `browser_act`, each with an `op` — the same three, with the same ops and
//! arguments, the MCP door publishes. Their names, schemas and texts are
//! read from the one definition (`browser_def`); what is here is how a
//! session runs a call and words the answer.
//!
//! Each op brokers one `control`-module op over the browser bridge
//! (a [`BrowserBridge`], served by the daemon's `BridgeHub`) to the `linggen-browser` extension, which
//! drives ONE visible controlled tab through CDP. Targeting is
//! reference-first: `browser_read op=page` returns a node tree with per-node
//! `ref`s; click/type resolve by ref. Mutating actions are gated by the
//! extension's own permission prompt (per-origin trust + hard floor).
//!
//! None of these tools is cacheable — the page is live mutable state.

use super::browser_def::{self, BridgeCall, Render};
use super::builtin::Tool;
use super::{ToolCall, ToolResult, Tools};
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use ling_mem::catalog::{Resolved, View};
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// A line of progress an op reports while it waits (e.g. "waiting for your OK").
pub type BridgeProgress = Box<dyn Fn(String) + Send + Sync>;

/// The transport the browser tools broker their ops over. The daemon
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

impl Tools {
    /// Broker one control op over the bridge for the tool `tool`. `Ok(data)`
    /// on success; a model-readable error otherwise (`no_bridge` gets
    /// install guidance).
    pub(crate) async fn browser_call(
        &self,
        tool: &str,
        call: BridgeCall,
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
            let tool = tool.to_string();
            Box::new(move |line: String| {
                let _ = tx.send((tool.clone(), "status".to_string(), line));
            }) as BridgeProgress
        });
        let res = hub
            .call(call.module, call.op, params, timeout_ms, on_progress)
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
                "element_gone: {message}. The page changed — call browser_read op=page again \
                 and use a fresh ref."
            ),
            _ => anyhow::bail!("{code}: {message}"),
        }
    }
}

/// A name a session was offered before the three tools, and the ops that
/// now hold what it did. A declaration (`allowed-tools`, `tools:`,
/// `place.*.tools`) or a call that still names it selects the tools of those
/// ops — through the registry's aliases, which are made from this table.
struct OldName {
    /// As it was offered, then the other spellings it answered to.
    names: &'static [&'static str],
    /// Its ops now: (tool, op).
    held_by: &'static [(&'static str, &'static str)],
}

const OLD_NAMES: &[OldName] = &[
    OldName {
        names: &["Browser_navigate", "browser_navigate"],
        held_by: &[("browser_open", "navigate")],
    },
    OldName {
        names: &["Browser_readPage", "browser_read_page", "Browser_read_page"],
        held_by: &[("browser_read", "page")],
    },
    OldName {
        names: &["Browser_screenshot", "browser_screenshot"],
        held_by: &[("browser_read", "screenshot")],
    },
    OldName {
        names: &["Browser_click", "browser_click"],
        held_by: &[("browser_act", "click")],
    },
    OldName {
        names: &["Browser_type", "browser_type"],
        held_by: &[("browser_act", "type")],
    },
    OldName {
        names: &["Browser_key", "browser_key"],
        held_by: &[("browser_act", "key")],
    },
    OldName {
        names: &["Browser_scroll", "browser_scroll"],
        held_by: &[("browser_act", "scroll")],
    },
    OldName {
        names: &["Browser_wait", "browser_wait"],
        held_by: &[("browser_act", "wait")],
    },
    OldName {
        names: &[
            "Browser_readConsole",
            "browser_read_console",
            "Browser_read_console",
        ],
        held_by: &[("browser_read", "console")],
    },
    OldName {
        names: &["Browser_tabs", "browser_tabs"],
        held_by: &[
            ("browser_read", "tabs"),
            ("browser_open", "tab_open"),
            ("browser_act", "tab_switch"),
            ("browser_act", "tab_close"),
        ],
    },
];

/// One of the three tools, as a session is offered it: name, text and
/// schema as the door publishes them.
pub struct BrowserTool {
    name: &'static str,
    description: &'static str,
    schema: Value,
    aliases: &'static [&'static str],
}

/// The three tools, for the registry — one per tool the definition names.
pub(super) fn tools() -> Vec<Arc<dyn Tool>> {
    browser_def::TOOLS
        .iter()
        .map(|doc| Arc::new(BrowserTool::of(doc.name)) as Arc<dyn Tool>)
        .collect()
}

impl BrowserTool {
    fn of(name: &'static str) -> Self {
        // What the door publishes to this machine, word for word.
        let listed = browser_def::catalog().tools_list(View::Local);
        let def = listed
            .iter()
            .find(|t| t["name"] == name)
            .expect("the definition lists every tool it names");
        let aliases: Vec<&'static str> = OLD_NAMES
            .iter()
            .filter(|old| old.held_by.iter().any(|(tool, _)| *tool == name))
            .flat_map(|old| old.names.iter().copied())
            .collect();
        Self {
            name,
            description: Box::leak(def["description"].as_str().unwrap_or_default().into()),
            schema: def["inputSchema"].clone(),
            aliases: Box::leak(aliases.into_boxed_slice()),
        }
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::Stays
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn aliases(&self) -> &'static [&'static str] {
        self.aliases
    }
    fn description(&self) -> &'static str {
        self.description
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        self.schema.clone()
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": self.name,
            "args": short_form(&self.schema),
            "returns": "the page, an image, or the tab's state, by op",
            "notes": self.description,
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let (branch, args) = match browser_def::catalog().resolve(View::Local, self.name, call.args)
        {
            Resolved::Call { branch, args } => (branch, args),
            Resolved::Refused(why) => anyhow::bail!(why),
            Resolved::Unknown => anyhow::bail!("unknown tool: {}", self.name),
        };
        let data = tools
            .browser_call(self.name, branch.dispatch, args, branch.timeout_ms)
            .await?;
        Ok(answer(branch.dispatch, &data))
    }
}

/// The schema's fields as the system prompt's short form writes them:
/// `{field: "type"}`, a `?` after the type of an optional one.
fn short_form(schema: &Value) -> Value {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut out = Map::new();
    for (key, field) in schema["properties"].as_object().into_iter().flatten() {
        let kind = match field["type"].as_str() {
            Some("array") => "array",
            Some("integer") => "number",
            Some(kind) => kind,
            None => "any",
        };
        let optional = if required.contains(&key.as_str()) {
            ""
        } else {
            "?"
        };
        out.insert(key.clone(), json!(format!("{kind}{optional}")));
    }
    Value::Object(out)
}

// ---------------------------------------------------------------------------
// What a session is told of an op's answer
// ---------------------------------------------------------------------------

fn data_str(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn answer(call: BridgeCall, data: &Value) -> ToolResult {
    match call.render {
        Render::Page => ToolResult::Success(page_text(data)),
        Render::Image => ToolResult::Screenshot {
            url: data_str(data, "url"),
            base64: data_str(data, "base64"),
            id: String::new(),
        },
        Render::Text => ToolResult::Success(op_text(call.op, data)),
    }
}

/// The page's URL and title, then its tree.
fn page_text(data: &Value) -> String {
    let truncated = data
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    format!(
        "{} — \"{}\"{}\n\n{}",
        data_str(data, "url"),
        data_str(data, "title"),
        if truncated {
            " (tree truncated — scroll or filter to see more)"
        } else {
            ""
        },
        data_str(data, "tree"),
    )
}

/// One sentence for an op that acted, by its bridge op.
fn op_text(bridge_op: &str, data: &Value) -> String {
    match bridge_op {
        "navigate" => format!(
            "navigated to {} — \"{}\"",
            data_str(data, "url"),
            data_str(data, "title"),
        ),
        "click" => format!(
            "clicked {} — page is now {}",
            data_str(data, "target"),
            data_str(data, "url"),
        ),
        "type" => format!(
            "typed {} characters",
            data.get("typed").and_then(Value::as_u64).unwrap_or(0),
        ),
        "key" => format!("pressed {}", data_str(data, "pressed")),
        "scroll" => format!(
            "scrolled {} by {}px",
            data_str(data, "scrolled"),
            data.get("amount").and_then(Value::as_u64).unwrap_or(0),
        ),
        "wait" => format!("waited for {}", data_str(data, "waited")),
        "read_console" => console_text(data),
        _ => data.to_string(),
    }
}

fn console_text(data: &Value) -> String {
    let messages = data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if messages.is_empty() {
        return "console is empty".to_string();
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
    lines.join("\n")
}

/// What the three tools do — the ten old tools' behaviour pinned before the
/// fold, kept behind the three: each op's bridge op, params and time limit,
/// the answer the model reads, how it fails, and what the engine knows of
/// each tool around the call (tier, reach, cache, ceiling, status line,
/// progress label). Where a line differs from the ten's, it says why.
#[cfg(test)]
mod pins {
    use super::*;
    use crate::engine::permission::PermissionMode;
    use crate::engine::tools::PlaceReach;
    use std::sync::Mutex;

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

    /// One call: the tool and its args (with `op`), the bridge op and params
    /// it sends, the limit it waits, the bridge's data and the model's text.
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
                tool: "browser_open",
                args: json!({"op": "navigate", "url": "https://a.b/"}),
                op: "navigate",
                sent: json!({"url": "https://a.b/"}),
                timeout_ms: 150_000,
                data: json!({"url": "https://a.b/x", "title": "T"}),
                text: "navigated to https://a.b/x — \"T\"",
            },
            Case {
                tool: "browser_read",
                args: json!({"op": "page"}),
                op: "read_page",
                sent: json!({}),
                timeout_ms: 20_000,
                data: page(),
                text: "https://a.b/x — \"T\"\n\n[n1] link",
            },
            Case {
                tool: "browser_read",
                args: json!({"op": "page", "filter": "interactive"}),
                op: "read_page",
                sent: json!({"filter": "interactive"}),
                timeout_ms: 20_000,
                data: json!({"url": "u", "title": "T", "tree": "t", "truncated": true}),
                text: "u — \"T\" (tree truncated — scroll or filter to see more)\n\nt",
            },
            Case {
                tool: "browser_act",
                args: json!({"op": "click", "ref": "n42", "button": "right", "double": true}),
                op: "click",
                sent: json!({"ref": "n42", "button": "right", "double": true}),
                timeout_ms: 150_000,
                data: json!({"target": "button Go", "url": "https://a.b/y"}),
                text: "clicked button Go — page is now https://a.b/y",
            },
            Case {
                tool: "browser_act",
                args: json!({"op": "type", "text": "hello", "ref": "n2", "clear": true}),
                op: "type",
                sent: json!({"text": "hello", "ref": "n2", "clear": true}),
                timeout_ms: 150_000,
                data: json!({"typed": 5}),
                text: "typed 5 characters",
            },
            Case {
                tool: "browser_act",
                args: json!({"op": "key", "keys": "Enter", "repeat": 3}),
                op: "key",
                sent: json!({"keys": "Enter", "repeat": 3}),
                timeout_ms: 150_000,
                data: json!({"pressed": "Enter"}),
                text: "pressed Enter",
            },
            Case {
                tool: "browser_act",
                args: json!({"op": "scroll", "direction": "down", "amount": 300, "ref": "n3"}),
                op: "scroll",
                sent: json!({"direction": "down", "amount": 300, "ref": "n3"}),
                timeout_ms: 20_000,
                data: json!({"scrolled": "down", "amount": 300}),
                text: "scrolled down by 300px",
            },
            Case {
                tool: "browser_act",
                args: json!({"op": "wait", "for": "ms", "value": "500"}),
                op: "wait",
                sent: json!({"for": "ms", "value": "500"}),
                timeout_ms: 45_000,
                data: json!({"waited": "ms"}),
                text: "waited for ms",
            },
            Case {
                tool: "browser_read",
                args: json!({"op": "console", "limit": 5}),
                op: "read_console",
                sent: json!({"limit": 5}),
                timeout_ms: 20_000,
                data: json!({"messages": [{"level": "error", "text": "boom"}, {"text": "hi"}]}),
                text: "[error] boom\n[log] hi",
            },
            Case {
                tool: "browser_read",
                args: json!({"op": "console"}),
                op: "read_console",
                sent: json!({}),
                timeout_ms: 20_000,
                data: json!({"messages": []}),
                text: "console is empty",
            },
            // The door waits 150s on the three tab ops that never ask the
            // person; a session waited 20s. One definition, one limit: the door's.
            Case {
                tool: "browser_read",
                args: json!({"op": "tabs"}),
                op: "tabs",
                sent: json!({"action": "list"}),
                timeout_ms: 150_000,
                data: json!({"tabs": []}),
                text: "{\"tabs\":[]}",
            },
            Case {
                tool: "browser_open",
                args: json!({"op": "tab_open", "url": "https://a.b/"}),
                op: "tabs",
                sent: json!({"action": "open", "url": "https://a.b/"}),
                timeout_ms: 150_000,
                data: json!({"url": "https://a.b/", "title": "T"}),
                text: "{\"url\":\"https://a.b/\",\"title\":\"T\"}",
            },
            Case {
                tool: "browser_act",
                args: json!({"op": "tab_switch"}),
                op: "tabs",
                sent: json!({"action": "switch"}),
                timeout_ms: 150_000,
                data: json!({"url": "u", "title": "T"}),
                text: "{\"url\":\"u\",\"title\":\"T\"}",
            },
            Case {
                tool: "browser_act",
                args: json!({"op": "tab_close"}),
                op: "tabs",
                sent: json!({"action": "close"}),
                timeout_ms: 150_000,
                data: json!({"closed": true}),
                text: "{\"closed\":true}",
            },
        ]
    }

    #[tokio::test]
    async fn each_op_asks_the_bridge_for_its_op_and_tells_the_model_what_came_back() {
        for case in cases() {
            let (tools, bridge) = tools_with(ok(case.data.clone()), None);
            let text = said(run(&tools, case.tool, case.args.clone()).await);
            let seen = bridge.seen.lock().unwrap().clone();
            let want = vec![(
                "control".to_string(),
                case.op.to_string(),
                case.sent.clone(),
                case.timeout_ms,
            )];
            assert_eq!(seen, want, "{} {}", case.tool, case.args);
            assert_eq!(text, case.text, "{} {}", case.tool, case.args);
        }
    }

    /// Every op of the definition is pinned: in the table above, or (an
    /// image, not a sentence) by the screenshot test below.
    #[test]
    fn every_op_of_the_definition_is_pinned() {
        let mut pinned: Vec<(String, String)> = cases()
            .iter()
            .map(|c| {
                (
                    c.tool.to_string(),
                    c.args["op"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        pinned.push(("browser_read".into(), "screenshot".into()));
        for b in browser_def::BRANCHES {
            assert!(
                pinned.contains(&(b.tool.to_string(), b.op.to_string())),
                "{} {}",
                b.tool,
                b.op
            );
        }
    }

    #[tokio::test]
    async fn a_screenshot_is_an_image_of_the_page_with_the_region_it_was_asked() {
        let reply = ok(json!({"url": "https://a.b/", "base64": "AAAA"}));
        let (tools, bridge) = tools_with(reply.clone(), None);
        let region = json!({"x": 1, "y": 2, "width": 3, "height": 4});
        let shot = run(
            &tools,
            "browser_read",
            json!({"op": "screenshot", "region": region}),
        )
        .await;
        match shot.expect("the call ran") {
            ToolResult::Screenshot { url, base64, id } => {
                assert_eq!(
                    (url.as_str(), base64.as_str(), id.as_str()),
                    ("https://a.b/", "AAAA", "")
                );
            }
            other => panic!("not a screenshot: {other:?}"),
        }
        let seen = bridge.seen.lock().unwrap().clone();
        let want = vec![(
            "control".to_string(),
            "screenshot".to_string(),
            json!({"region": region}),
            20_000,
        )];
        assert_eq!(seen, want);
        // No region: none is sent.
        let (tools, bridge) = tools_with(reply, None);
        run(&tools, "browser_read", json!({"op": "screenshot"}))
            .await
            .unwrap();
        assert_eq!(bridge.seen.lock().unwrap()[0].2, json!({}));
    }

    #[tokio::test]
    async fn a_failure_reads_the_way_it_does_today() {
        let fail =
            |code: &str, message: &str| json!({"ok": false, "code": code, "message": message});
        let said_err = |reply: Value, tool: &'static str, args: Value| async move {
            let (tools, _) = tools_with(reply, None);
            run(&tools, tool, args).await.unwrap_err().to_string()
        };
        let not_connected =
            "browser not connected — the linggen-browser extension (with the control \
             module) must be running in Chrome. Ask the user to install or enable it, \
             then retry.";
        for code in ["no_bridge", "module_unavailable"] {
            let got = said_err(fail(code, "x"), "browser_read", json!({"op": "page"})).await;
            assert_eq!(got, not_connected, "{code}");
        }
        let click = json!({"op": "click", "ref": "n9"});
        let got = said_err(fail("element_gone", "n9"), "browser_act", click).await;
        assert_eq!(
            got,
            "element_gone: n9. The page changed — call browser_read op=page again and use a fresh ref."
        );
        let got = said_err(
            fail("not_permitted", "no"),
            "browser_act",
            json!({"op": "type", "text": "a"}),
        )
        .await;
        assert_eq!(got, "not_permitted: no");
        let got = said_err(
            json!({"ok": false}),
            "browser_act",
            json!({"op": "key", "keys": "a"}),
        )
        .await;
        assert_eq!(got, "upstream_error: ");
        // No bridge wired (CLI, evals).
        let tools = Tools::new(std::path::PathBuf::from(".")).unwrap();
        let got = run(&tools, "browser_read", json!({"op": "tabs"}))
            .await
            .unwrap_err();
        assert_eq!(
            got.to_string(),
            "browser control is unavailable in this context (no daemon bridge)"
        );
    }

    /// The door's checks now run in a session too, before the bridge is
    /// asked: a call the door would refuse is refused in the words the door
    /// uses, and nothing is sent. (The ten passed these on to the extension.)
    #[tokio::test]
    async fn a_call_the_door_would_refuse_is_refused_before_the_bridge_is_asked() {
        let refused = [
            ("browser_open", json!({"op": "navigate"}), "needs `url`"),
            (
                "browser_open",
                json!({"op": "navigate", "url": "example.com"}),
                "absolute URL",
            ),
            ("browser_open", json!({"op": "tab_open"}), "needs `url`"),
            ("browser_act", json!({"op": "click"}), "needs `ref`"),
            (
                "browser_act",
                json!({"op": "click", "ref": "n1", "coordinate": [1, 2]}),
                "not both",
            ),
            ("browser_act", json!({"click": true}), "needs op"),
            (
                "browser_read",
                json!({"op": "page", "filter": "some"}),
                "`filter` must be one of",
            ),
        ];
        for (tool, args, why) in refused {
            let (tools, bridge) = tools_with(ok(json!({})), None);
            let got = run(&tools, tool, args.clone())
                .await
                .unwrap_err()
                .to_string();
            assert!(got.contains(why), "{tool} {args}: {got}");
            assert!(bridge.seen.lock().unwrap().is_empty(), "{tool} {args}");
        }
    }

    /// The bridge's progress line rides the running tool, under the name of
    /// the tool the call came in by.
    #[tokio::test]
    async fn progress_rides_the_running_tool_under_its_name() {
        for case in cases() {
            let (mut tools, _) = tools_with(ok(case.data.clone()), Some("waiting for your OK"));
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            tools.set_progress_tx(tx);
            run(&tools, case.tool, case.args.clone()).await.unwrap();
            let got = rx.try_recv().expect("a progress line");
            assert_eq!(
                got,
                (
                    case.tool.to_string(),
                    "status".to_string(),
                    "waiting for your OK".to_string()
                )
            );
        }
    }

    /// What the engine knows of each tool around the call.
    #[test]
    fn each_tool_is_read_tier_stays_in_place_and_is_never_cached() {
        for doc in browser_def::TOOLS {
            let name = doc.name;
            let tool = super::super::builtin::lookup(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(tool.name(), name);
            assert_eq!(tool.tier(), PermissionMode::Read, "{name}");
            assert_eq!(tool.place_reach(), PlaceReach::Stays, "{name}");
            assert!(!tool.cacheable(), "{name}");
            assert!(tool.model_facing() && tool.offered().is_ok(), "{name}");
            assert_eq!(
                tool.max_duration(),
                Some(std::time::Duration::from_secs(300)),
                "{name}"
            );
            assert_eq!(
                crate::engine::tools::builtin_tier(name),
                Some(PermissionMode::Read),
                "{name}"
            );
        }
    }

    /// Each of the ten old names is a name of the tool that holds its op, and
    /// that op asks the bridge for what the old tool asked it.
    #[test]
    fn each_old_name_selects_the_tool_that_holds_what_it_did() {
        let bridge_op = |old: &'static str| -> &'static str {
            match old {
                "Browser_readPage" => "read_page",
                "Browser_readConsole" => "read_console",
                other => other.strip_prefix("Browser_").unwrap(),
            }
        };
        assert_eq!(OLD_NAMES.len(), 10);
        for old in OLD_NAMES {
            let first = old.names[0];
            for (tool, op) in old.held_by {
                let branch = browser_def::BRANCHES
                    .iter()
                    .find(|b| b.tool == *tool && b.op == *op)
                    .unwrap_or_else(|| panic!("{first} is held by {tool} op={op}, which is no op"));
                assert_eq!(branch.dispatch.op, bridge_op(first), "{first}");
                let holder = super::super::builtin::lookup(tool).unwrap();
                for name in old.names {
                    assert!(
                        holder.aliases().contains(name),
                        "{name} is not an alias of {tool}"
                    );
                }
            }
        }
    }

    /// An op none of the ten had is held by no old name; every other op is.
    #[test]
    fn every_op_the_ten_had_is_held_by_an_old_name() {
        for b in browser_def::BRANCHES {
            let held = OLD_NAMES.iter().any(|old| {
                old.held_by
                    .iter()
                    .any(|(tool, op)| *tool == b.tool && *op == b.op)
            });
            assert!(held, "{} {} is held by no old name", b.tool, b.op);
        }
    }

    /// The session is offered the three, and none of the ten.
    #[test]
    fn a_session_is_offered_the_three_and_none_of_the_ten() {
        let offered: Vec<String> = super::super::json_schema::oai_tool_definitions(None)
            .iter()
            .filter_map(|d| d["function"]["name"].as_str().map(String::from))
            .filter(|n| n.to_lowercase().starts_with("browser_"))
            .collect();
        assert_eq!(offered, ["browser_open", "browser_read", "browser_act"]);
    }

    /// The line a session's chat shows for each op, before, during and after.
    #[test]
    fn each_op_has_its_status_line() {
        use crate::engine::tool_render::{tool_status_line, ToolStatusPhase::*};
        let rows: [(&str, Value, [&str; 3]); 14] = [
            (
                "browser_open",
                json!({"op": "navigate", "url": "https://a.b/"}),
                [
                    "Browsing to: https://a.b/",
                    "Browsed to: https://a.b/",
                    "Navigation failed: https://a.b/",
                ],
            ),
            (
                "browser_read",
                json!({"op": "page"}),
                ["Reading page", "Read page", "Page read failed"],
            ),
            (
                "browser_read",
                json!({"op": "screenshot"}),
                ["Capturing page", "Captured page", "Capture failed"],
            ),
            (
                "browser_act",
                json!({"op": "click", "ref": "n4"}),
                ["Clicking: n4", "Clicked: n4", "Click failed: n4"],
            ),
            (
                "browser_act",
                json!({"op": "type", "text": "hi"}),
                ["Typing: hi", "Typed: hi", "Typing failed: hi"],
            ),
            (
                "browser_act",
                json!({"op": "key", "keys": "Enter"}),
                [
                    "Pressing: Enter",
                    "Pressed: Enter",
                    "Key press failed: Enter",
                ],
            ),
            (
                "browser_act",
                json!({"op": "scroll", "direction": "down"}),
                ["Scrolling: down", "Scrolled: down", "Scroll failed: down"],
            ),
            (
                "browser_act",
                json!({"op": "wait", "for": "load"}),
                [
                    "Waiting for page: load",
                    "Waited for page: load",
                    "Wait failed: load",
                ],
            ),
            (
                "browser_read",
                json!({"op": "console"}),
                ["Reading console", "Read console", "Console read failed"],
            ),
            (
                "browser_read",
                json!({"op": "tabs"}),
                [
                    "Browser tab: list",
                    "Browser tab: list",
                    "Browser tab action failed: list",
                ],
            ),
            (
                "browser_open",
                json!({"op": "tab_open", "url": "https://a.b/"}),
                [
                    "Browser tab: https://a.b/",
                    "Browser tab: https://a.b/",
                    "Browser tab action failed: https://a.b/",
                ],
            ),
            (
                "browser_act",
                json!({"op": "tab_switch"}),
                [
                    "Browser tab: switch",
                    "Browser tab: switch",
                    "Browser tab action failed: switch",
                ],
            ),
            (
                "browser_act",
                json!({"op": "tab_close"}),
                [
                    "Browser tab: close",
                    "Browser tab: close",
                    "Browser tab action failed: close",
                ],
            ),
            (
                "browser_act",
                json!({}),
                ["Browser action", "Browser action", "Browser action failed"],
            ),
        ];
        for (tool, args, [start, done, failed]) in rows {
            assert_eq!(
                tool_status_line(tool, Some(&args), Start),
                start,
                "{tool} {args}"
            );
            assert_eq!(
                tool_status_line(tool, Some(&args), Done),
                done,
                "{tool} {args}"
            );
            assert_eq!(
                tool_status_line(tool, Some(&args), Failed),
                failed,
                "{tool} {args}"
            );
        }
        // No op of the definition falls to the generic line.
        for b in browser_def::BRANCHES {
            let args = json!({"op": b.op});
            let line = tool_status_line(b.tool, Some(&args), Start);
            assert_ne!(line, "Browser action", "{} {}", b.tool, b.op);
        }
    }

    /// The delegate's fixed list holds the three tools and none of the ten.
    #[test]
    fn the_delegate_holds_the_three_and_none_of_the_ten() {
        for write in [false, true] {
            let scope = crate::engine::session_spec::delegate::tool_scope(write).unwrap();
            for doc in browser_def::TOOLS {
                assert!(scope.contains(doc.name), "{}", doc.name);
            }
            let ten: Vec<&&str> = OLD_NAMES
                .iter()
                .filter_map(|old| old.names.first())
                .collect();
            assert!(ten.iter().all(|name| !scope.contains(**name)), "{ten:?}");
        }
    }
}
