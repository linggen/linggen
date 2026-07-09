//! Site-trust gate for mutating browser actions — the runtime half of
//! `doc/browser-control-spec.md` §"Permission and safety".
//!
//! Read-class `Browser_*` ops run free. Mutating ops (`navigate`, `click`,
//! `type`, `key`, `tabs open`) prompt per action on an untrusted origin; the
//! prompt offers "Allow this site for the session", which persists the origin
//! in the session's `permission.json`. The hard floor — payment, credentials,
//! deletion, posting as the user — always confirms, even on a trusted site,
//! and never offers the site grant.
//!
//! Classification is pure logic in `permission/browser.rs`; this file owns
//! the engine flow: live origin lookup, prompting, and grant persistence.

use super::types::AgentEngine;
use crate::engine::permission;
use crate::engine::tools::{browser_tool, AskUserOption, AskUserQuestion};
use serde_json::Value as JsonValue;
use tracing::info;

pub(crate) enum BrowserGateDecision {
    Proceed,
    /// Blocked — the message is fed back to the model as the tool result.
    Deny(String),
    /// The prompt timed out; the agent turn should end.
    Timeout,
}

impl AgentEngine {
    /// Gate one mutating browser action. Only called for actions where
    /// `permission::browser_action_mutating` is true.
    pub(crate) async fn browser_safety_gate(
        &mut self,
        tool: &str,
        args: &JsonValue,
    ) -> BrowserGateDecision {
        // No bridge in this context (CLI / eval) — skip the prompt and let
        // the tool itself fail with the helpful "not connected" guidance.
        if self.tools.builtins.browser_bridge.is_none() {
            return BrowserGateDecision::Proceed;
        }

        // Target element metadata (ref-carrying actions) — feeds both the
        // hard-floor check and a readable prompt.
        let meta = args
            .get("ref")
            .and_then(JsonValue::as_str)
            .and_then(|r| self.tools.builtins.browser_ref_meta(r));
        let floor = meta.as_ref().and_then(permission::browser_hard_floor);

        // Origin: the destination URL for navigate/open; otherwise the live
        // controlled-tab origin (never a cached URL — redirects would make
        // the gate check a site the action doesn't touch).
        let url_arg = args.get("url").and_then(JsonValue::as_str).unwrap_or("");
        let origin = match browser_tool::origin_of(url_arg) {
            Some(o) => Some(o),
            None => self.tools.builtins.browser_current_origin().await,
        };

        let trusted = origin
            .as_deref()
            .map(|o| self.session_permissions.browser_origin_trusted(o))
            .unwrap_or(false);
        if trusted && floor.is_none() {
            return BrowserGateDecision::Proceed;
        }

        let summary = permission::browser_action_summary(tool, args, meta.as_ref());
        let origin_label = origin.clone().unwrap_or_else(|| "unknown site".to_string());

        // Missions and proxy consumers cannot prompt — mutating browser
        // actions on untrusted origins are simply unavailable to them.
        if !self.session_permissions.interactive {
            return BrowserGateDecision::Deny(format!(
                "Browser action blocked: '{summary}' on {origin_label} needs user \
                 confirmation, and this session cannot prompt."
            ));
        }

        let question = build_browser_question(&summary, &origin_label, floor);
        match self.ask_permission_raw(tool, question).await {
            Some(permission::PermissionAction::AllowOnce) => BrowserGateDecision::Proceed,
            Some(permission::PermissionAction::AllowSession) => {
                // The floor prompt never offers the site grant, so reaching
                // here means a plain site-trust prompt was approved.
                if let Some(o) = &origin {
                    self.session_permissions.grant_browser_origin(o);
                    if let Some(ref sdir) = self.session_dir {
                        self.session_permissions.save(sdir);
                    }
                    info!("Browser gate: origin {o} trusted for the session");
                }
                BrowserGateDecision::Proceed
            }
            Some(permission::PermissionAction::Deny) => BrowserGateDecision::Deny(format!(
                "Permission denied by user for {tool}: {summary}"
            )),
            Some(permission::PermissionAction::DenyWithMessage(user_msg)) => {
                BrowserGateDecision::Deny(format!(
                    "Permission denied by user for {tool} '{summary}'. User says: {user_msg}"
                ))
            }
            None => BrowserGateDecision::Timeout,
        }
    }
}

fn build_browser_question(
    summary: &str,
    origin: &str,
    floor: Option<&'static str>,
) -> AskUserQuestion {
    let mut options = vec![AskUserOption {
        label: "Allow once".to_string(),
        description: Some("Run this action now".to_string()),
        preview: None,
    }];
    if floor.is_none() {
        options.push(AskUserOption {
            label: "Allow this site for the session".to_string(),
            description: Some(format!(
                "Browser actions on {origin} run without further prompts (sensitive actions still confirm)"
            )),
            preview: None,
        });
    }
    options.push(AskUserOption {
        label: "Deny".to_string(),
        description: Some("Block this action".to_string()),
        preview: None,
    });
    AskUserQuestion {
        question: match floor {
            Some(category) => {
                format!("{summary} — {origin} · sensitive ({category}), always confirmed")
            }
            None => format!("{summary} — {origin}"),
        },
        header: "Browser".to_string(),
        options,
        multi_select: false,
    }
}
