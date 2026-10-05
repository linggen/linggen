//! The fake model: llmposter in the test process, scripted per agent.
//!
//! A test says what each member answers, call by call — Ling's first call
//! uses Look, his second says a line — and gets exactly that. A member is
//! told apart by its system prompt (its soul's opening line), so one fake
//! server serves every member and model at the table. A call no script
//! covers gets [`UNSCRIPTED`], and shows up in [`FakeModel::unscripted`].

use llmposter::{CapturedRequest, Fixture, MockServer, RequestOutcome, ServerBuilder, ToolCall};
use serde_json::Value;

/// What a call nobody scripted gets back.
pub const UNSCRIPTED: &str = "(unscripted reply)";

/// The scenario name of the catch-all, so its calls can be counted.
const CATCH_ALL: &str = "unscripted";

/// One member, as the fake model recognises it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Ling,
    Yinyue,
}

impl Agent {
    pub fn id(self) -> &'static str {
        match self {
            Agent::Ling => "ling",
            Agent::Yinyue => "yinyue",
        }
    }

    /// The opening of the agent's soul (agents/<id>.md), in every system prompt
    /// of its own.
    fn marker(self) -> &'static str {
        match self {
            Agent::Ling => "You are Ling — Linggen itself",
            Agent::Yinyue => "You are Yinyue — the user's friend",
        }
    }
}

/// One scripted answer.
#[derive(Clone, Debug)]
pub enum Reply {
    Text(String),
    Tool(String, Value),
}

pub fn text(s: &str) -> Reply {
    Reply::Text(s.to_string())
}

pub fn tool(name: &str, args: Value) -> Reply {
    Reply::Tool(name.to_string(), args)
}

/// Extra fixtures a test needs beside the member scripts (a summarizer…).
pub struct Extra {
    pub system_contains: &'static str,
    pub reply: &'static str,
}

pub struct FakeModel {
    server: MockServer,
}

/// A captured call, its body parsed.
pub struct Call {
    pub path: String,
    pub body: Value,
    pub scenario: Option<String>,
    pub matched: bool,
}

impl FakeModel {
    pub async fn start(scripts: &[(Agent, Vec<Reply>)], extras: &[Extra]) -> Self {
        let mut builder = ServerBuilder::new();
        for (agent, replies) in scripts {
            builder = builder.fixtures(member_fixtures(*agent, replies));
        }
        for extra in extras {
            builder = builder.fixture(
                Fixture::new()
                    .match_system_prompt(extra.system_contains)
                    .respond_with_content(extra.reply)
                    .with_priority(10),
            );
        }
        let catch_all = Fixture::new()
            .respond_with_content(UNSCRIPTED)
            .with_scenario(CATCH_ALL, None, None)
            .as_catch_all();
        let server = builder
            .fixture(catch_all)
            .build()
            .await
            .expect("start llmposter");
        Self { server }
    }

    /// The base URL the fixture config's models point at.
    pub fn url(&self) -> String {
        self.server.url()
    }

    pub fn calls(&self) -> Vec<Call> {
        self.server.get_requests().iter().map(parse_call).collect()
    }

    /// The calls made by `agent` (by its scenario), in order.
    pub fn calls_of(&self, agent: Agent) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| c.scenario.as_deref() == Some(agent.id()))
            .collect()
    }

    /// Calls no script covered: unmatched, or answered by the catch-all.
    pub fn unscripted(&self) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| !c.matched || c.scenario.as_deref() == Some(CATCH_ALL))
            .collect()
    }
}

/// A member's replies as a chain of scenario states: call 0 needs the
/// initial state and moves to "1", call 1 needs "1", and so on.
fn member_fixtures(agent: Agent, replies: &[Reply]) -> Vec<Fixture> {
    replies
        .iter()
        .enumerate()
        .map(|(i, reply)| {
            let need = if i == 0 { String::new() } else { i.to_string() };
            let next = (i + 1).to_string();
            respond(Fixture::new().match_system_prompt(agent.marker()), reply)
                .with_scenario(agent.id(), Some(&need), Some(&next))
                .with_priority(5)
        })
        .collect()
}

fn respond(fixture: Fixture, reply: &Reply) -> Fixture {
    match reply {
        Reply::Text(s) => fixture.respond_with_content(s),
        Reply::Tool(name, args) => fixture.respond_with_tool_calls(vec![ToolCall {
            name: name.clone(),
            arguments: args.clone(),
        }]),
    }
}

fn parse_call(r: &CapturedRequest) -> Call {
    Call {
        path: r.path.clone(),
        body: serde_json::from_str(&r.body).unwrap_or(Value::Null),
        scenario: r.matched_scenario.clone(),
        matched: r.outcome == RequestOutcome::Matched,
    }
}

impl Call {
    /// The model id the engine asked for.
    pub fn model(&self) -> &str {
        self.body["model"].as_str().unwrap_or("")
    }

    /// Tool names offered on this call (Chat Completions or Responses shape).
    pub fn tool_names(&self) -> Vec<String> {
        let tools = self.body["tools"].as_array().cloned().unwrap_or_default();
        tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str().or(t["name"].as_str()))
            .map(str::to_string)
            .collect()
    }

    /// The conversation as (role, text) pairs — Chat Completions `messages`
    /// or Responses `input` — with the system prompt left out.
    pub fn thread(&self) -> Vec<(String, String)> {
        if let Some(msgs) = self.body["messages"].as_array() {
            let skip = usize::from(msgs.first().is_some_and(|m| m["role"] == "system"));
            return msgs.iter().skip(skip).map(chat_message).collect();
        }
        let items = self.body["input"].as_array().cloned().unwrap_or_default();
        items.iter().map(responses_item).collect()
    }

    /// Everything the call carried, as one string (for `contains` checks).
    pub fn text(&self) -> String {
        self.body.to_string()
    }
}

fn chat_message(m: &Value) -> (String, String) {
    let role = m["role"].as_str().unwrap_or("?").to_string();
    let mut text = content_text(&m["content"]);
    if let Some(calls) = m["tool_calls"].as_array() {
        for c in calls {
            let f = &c["function"];
            text.push_str(&format!(
                "[call {} {}]",
                f["name"].as_str().unwrap_or("?"),
                f["arguments"].as_str().unwrap_or("")
            ));
        }
    }
    (role, text)
}

fn responses_item(item: &Value) -> (String, String) {
    let kind = item["type"].as_str().unwrap_or("message");
    match kind {
        "function_call" => (
            "assistant".into(),
            format!(
                "[call {} {}]",
                item["name"].as_str().unwrap_or("?"),
                item["arguments"].as_str().unwrap_or("")
            ),
        ),
        "function_call_output" => ("tool".into(), item["output"].as_str().unwrap_or("").into()),
        _ => (
            item["role"].as_str().unwrap_or("?").to_string(),
            content_text(&item["content"]),
        ),
    }
}

fn content_text(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    let parts = content.as_array().cloned().unwrap_or_default();
    parts
        .iter()
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
