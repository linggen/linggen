//! An `agent_run` delegate's session (`doc/session-spec.md` § The session
//! spec): a person typing into it keeps the delegate's spec.

use crate::support::model::Agent;
use crate::support::{error, text, tool, Setup, World};
use serde_json::{json, Value};

/// The one session an external host made.
pub(crate) fn delegate_session(w: &World) -> String {
    let dir = w.home.linggen_home().join("sessions");
    let mut made: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|id| {
            let meta =
                std::fs::read_to_string(dir.join(id).join("session.yaml")).unwrap_or_default();
            meta.lines().any(|l| l.trim() == "creator: external")
        })
        .collect();
    assert_eq!(made.len(), 1, "one delegated session: {made:?}");
    made.remove(0)
}

#[tokio::test]
async fn work_agent_run_switches() {
    for (write, requested, expected) in [(true, None, Some("code")), (true, Some("none"), None), (false, None, None), (false, Some("code"), Some("code"))] {
        let w = World::start(Setup::default().script(Agent::Ling, vec![text("Done."), text("Again.")])).await;
        let mut args = json!({"prompt":"Say done."});
        if write { args["write_dir"] = json!(w.work()); }
        if let Some(value) = requested { args["work"] = json!(value); }
        let call = |args| json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"agent_run", "arguments":args}});
        let result = w.api.post("/mcp", call(args)).await;
        assert_eq!(result["result"]["content"][0]["text"], "Done.", "{result}");
        let sid = delegate_session(&w);
        let path = w.home.linggen_home().join("sessions").join(&sid).join("session.yaml");
        let read = || -> Value { serde_norway::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap() };
        assert_eq!(read()["work"].as_str(), expected);
        // What the run itself was sent — not an export of the session: the
        // section is in the prompt the model saw exactly when the work is code.
        let sent = w.model.calls_of(Agent::Ling);
        let saw = sent.iter().any(|c| c.body.to_string().contains("## Working on code"));
        assert_eq!(saw, expected.is_some(), "write {write}, asked {requested:?}");
        let other = if expected.is_some() { "none" } else { "code" };
        let result = w.api.post("/mcp", call(json!({"prompt":"Again", "session_id":sid, "work":other}))).await;
        assert_eq!(result["result"]["content"][0]["text"], "Again.", "{result}");
        assert_eq!(read()["work"].as_str(), expected);
    }
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// A person typing into a delegated run's session: the same toolset as
/// the run (no Task, no AskUser, no Bash on a read run), and the session
/// still answers to its caller.
#[tokio::test]
async fn a_person_typing_into_a_delegated_session_keeps_its_toolset() {
    let setup = Setup::default().script(Agent::Ling, vec![text("Done."), text("Still here.")]);
    let w = World::start(setup).await;
    let call = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "agent_run", "arguments": {"prompt": "Say done."}}
    });
    let ran = w.api.post("/mcp", call).await;
    assert_eq!(ran["result"]["content"][0]["text"], "Done.", "{ran}");
    let sid = delegate_session(&w);

    let cwd = {
        let meta = std::fs::read_to_string(
            w.home
                .linggen_home()
                .join("sessions")
                .join(&sid)
                .join("session.yaml"),
        )
        .unwrap();
        let line = meta
            .lines()
            .find(|l| l.starts_with("cwd:"))
            .unwrap()
            .to_string();
        line.trim_start_matches("cwd:")
            .trim()
            .trim_matches('\'')
            .to_string()
    };
    w.turn(&sid, &cwd, "And now?", Agent::Ling).await;

    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 2, "the run, then the typed turn");
    let run = sorted(calls[0].tool_names());
    let typed = sorted(calls[1].tool_names());
    assert!(
        !run.contains(&"Bash".to_string()),
        "a read run has no Bash: {run:?}"
    );
    assert_eq!(typed, run, "the typed turn keeps the delegate's toolset");

    let perms: Value = serde_json::from_str(
        &std::fs::read_to_string(
            w.home
                .linggen_home()
                .join("sessions")
                .join(&sid)
                .join("permission.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(perms["interactive"], false, "{perms}");
    assert_eq!(
        perms["ask_caller"], true,
        "it still answers to its caller: {perms}"
    );
    w.assert_all_scripted();
}

/// An `agent_run` tool call, as a host sends it.
fn agent_run(args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "agent_run", "arguments": args}})
}

/// A delegated run that read a file and then ended on a model failure,
/// continued with `agent_run(session_id)`: the continued turn is given what
/// the run had read, not only its brief.
#[tokio::test]
async fn a_run_continued_after_a_model_failure_keeps_what_it_read() {
    const MARKER: &str = "the-harbor-ledger-balances-at-7731";
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool("Read", json!({"path": "notes.txt"})),
            error(500, "upstream overloaded"),
            text("Picked up where I stopped."),
        ],
    );
    let w = World::start(setup).await;
    std::fs::write(std::path::Path::new(&w.work()).join("notes.txt"), format!("{MARKER}\n")).unwrap();

    let first = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Read notes.txt, then sum it up.", "write_dir": w.work()})))
        .await;
    let said = first["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
    assert!(said.contains("agent run failed"), "the run ends on the model failure: {first}");
    let sid = delegate_session(&w);

    let again = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Go on from where you stopped.", "session_id": sid})))
        .await;
    assert_eq!(again["result"]["content"][0]["text"], "Picked up where I stopped.", "{again}");

    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 3, "the read, the failed call, the continued turn");
    assert!(calls[1].body.to_string().contains(MARKER), "the failed call had read the file");
    let continued = calls[2].body.to_string();
    assert!(continued.contains("Read notes.txt, then sum it up."), "the brief is kept");
    assert!(continued.contains(MARKER), "the continued turn lost what the run had read");
}

/// A run that failed for no reason a caller acts on (not a provider's
/// refusal) keeps nothing in the thread — `keep_unfinished_work` keeps only
/// a run that cannot go on — so its status names no reason, and the turn
/// that continues the session starts from its brief alone.
#[tokio::test]
async fn a_run_that_failed_for_no_reason_a_caller_acts_on_keeps_nothing() {
    const MARKER: &str = "the-orchard-count-is-3319";
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool("Read", json!({"path": "notes.txt"})),
            error(418, "not a provider refusal"),
            text("Starting over."),
        ],
    );
    let w = World::start(setup).await;
    std::fs::write(std::path::Path::new(&w.work()).join("notes.txt"), format!("{MARKER}\n")).unwrap();
    let ran = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Read notes.txt.", "write_dir": w.work()})))
        .await;
    assert!(ran["result"]["content"][0]["text"].as_str().unwrap_or_default().contains("agent run failed"), "{ran}");
    let sid = delegate_session(&w);
    let status = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "agent_status", "arguments": {"op": "session", "session_id": sid}}});
    let status = w.api.post("/mcp", status).await;
    let status: Value = serde_json::from_str(status["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(status["status"], "failed", "{status}");
    assert_eq!(status["cannot_run"], Value::Null, "{status}");

    let again = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Go on.", "session_id": sid})))
        .await;
    assert_eq!(again["result"]["content"][0]["text"], "Starting over.", "{again}");
    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 3, "the read, the failed call, the continued turn");
    assert!(calls[1].body.to_string().contains(MARKER), "the failed call had read the file");
    assert!(!calls[2].body.to_string().contains(MARKER), "the continued turn was given the failed run's reading");
}

/// A delegated run that ends because its model failed says why in a field
/// of its own — the reason, the model, one line on what to do, and that the
/// session holds no work to continue from (it did none) — beside the error's words, in `agent_status`.
#[tokio::test]
async fn a_run_that_ends_on_its_model_says_why_in_a_field() {
    let setup = Setup::default().script(Agent::Ling, vec![error(500, "upstream overloaded")]);
    let w = World::start(setup).await;
    let ran = w.api.post("/mcp", agent_run(json!({"prompt": "Say done."}))).await;
    assert!(ran["result"]["content"][0]["text"].as_str().unwrap_or_default().contains("agent run failed"), "{ran}");
    let sid = delegate_session(&w);
    let status = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "agent_status", "arguments": {"op": "session", "session_id": sid}}});
    let status = w.api.post("/mcp", status).await;
    let text = status["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{status}"));
    let status: Value = serde_json::from_str(text).unwrap();
    assert_eq!(status["status"], "failed", "{status}");
    assert!(status["error"].as_str().unwrap_or_default().contains("upstream overloaded"), "{status}");
    let why = &status["cannot_run"];
    assert_eq!(why["reason"], "provider", "{status}");
    assert_eq!(why["kind"], "server5xx", "{status}");
    assert_eq!(why["model"], status["model"], "{status}");
    assert_eq!(why["work_kept"], false, "the run failed before it did anything: {status}");
    assert!(why["line"].as_str().is_some_and(|l| !l.is_empty()), "{status}");
}

/// The same through a continued turn (`agent_run(session_id)` runs as a
/// chat turn): a continued turn that read a file and then failed says why,
/// with its work kept, and the turn after it starts from that reading.
#[tokio::test]
async fn a_continued_turn_that_fails_says_why_and_keeps_its_reading() {
    const MARKER: &str = "the-tide-table-reads-0412";
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            text("Ready."),
            tool("Read", json!({"path": "tides.txt"})),
            error(500, "upstream overloaded"),
            text("Back on it."),
        ],
    );
    let w = World::start(setup).await;
    std::fs::write(std::path::Path::new(&w.work()).join("tides.txt"), format!("{MARKER}\n")).unwrap();
    let first = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Stand by.", "write_dir": w.work()})))
        .await;
    assert_eq!(first["result"]["content"][0]["text"], "Ready.", "{first}");
    let sid = delegate_session(&w);

    let failed = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Read tides.txt, then sum it up.", "session_id": sid})))
        .await;
    let said = failed["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
    assert!(said.contains("agent run failed"), "{failed}");
    let status = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "agent_status", "arguments": {"op": "session", "session_id": sid}}});
    let status = w.api.post("/mcp", status).await;
    let status: Value = serde_json::from_str(status["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(status["cannot_run"]["reason"], "provider", "{status}");
    assert_eq!(status["cannot_run"]["work_kept"], true, "{status}");

    let again = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Go on.", "session_id": sid})))
        .await;
    assert_eq!(again["result"]["content"][0]["text"], "Back on it.", "{again}");
    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 4, "ready, the read, the failed call, the turn after");
    assert!(calls[3].body.to_string().contains(MARKER), "the turn after lost the reading");
}

/// One of Linggen's own secret files, holding a line no model may be shown.
/// The engine's home is the world's, so `~` names it.
const SECRET: &str = "~/.linggen/account.toml";
const SECRET_LINE: &str = "the-vault-combination-is-4471-east";

fn plant_secret(w: &World) {
    let file = w.home.linggen_home().join("account.toml");
    std::fs::write(file, format!("token = \"{SECRET_LINE}\"\n")).unwrap();
}

fn read_secret() -> crate::support::model::Reply {
    tool("Read", json!({"path": SECRET}))
}

fn shown_the_secret(w: &World) -> bool {
    w.model.calls_of(Agent::Ling).iter().any(|c| c.body.to_string().contains(SECRET_LINE))
}

/// A host continuing a PERSON's own chat with `agent_run(session_id)`: the
/// person's turn before it stood as the local owner, and the host's turn
/// does not inherit that — a Linggen secret file is refused, not asked of
/// the host (which could answer its own question).
#[tokio::test]
async fn a_host_continuing_a_persons_chat_cannot_read_a_secret_file() {
    let setup = Setup::default().script(Agent::Ling, vec![text("Hello."), read_secret(), text("Refused.")]);
    let w = World::start(setup).await;
    plant_secret(&w);
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    w.turn(&sid, &w.work(), "Hello", Agent::Ling).await;

    let ran = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Read the account file.", "session_id": sid})))
        .await;
    let said = ran["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(said.starts_with("Refused.") && said.contains("(blocked)"), "refused, with nobody asked: {ran}");
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 3, "the hello, the read, the answer");
    assert!(!shown_the_secret(&w), "the host's turn was shown the secret file");
}

/// A delegated session a person typed into, then continued by its host: the
/// continued turn is no laxer than the first run — the secret file is refused.
#[tokio::test]
async fn a_delegated_run_continued_after_a_persons_turn_cannot_read_a_secret_file() {
    let setup = Setup::default()
        .script(Agent::Ling, vec![text("Done."), text("Still here."), read_secret(), text("Refused.")]);
    let w = World::start(setup).await;
    plant_secret(&w);
    let ran = w.api.post("/mcp", agent_run(json!({"prompt": "Say done.", "write_dir": w.work()}))).await;
    assert_eq!(ran["result"]["content"][0]["text"], "Done.", "{ran}");
    let sid = delegate_session(&w);
    w.turn(&sid, &w.work(), "And now?", Agent::Ling).await;

    let again = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Read the account file.", "session_id": sid})))
        .await;
    let said = again["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(said.starts_with("Refused.") && said.contains("(blocked)"), "refused, with nobody asked: {again}");
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 4, "the run, the typed turn, the read, the answer");
    assert!(!shown_the_secret(&w), "the continued turn was shown the secret file");
}

fn asked_for_the_secret(events: &[Value]) -> bool {
    events.iter().any(|e| e["kind"] == "ask_user" && e.to_string().contains("account.toml"))
}

/// The control for the two above: the owner's own typed turn, at this Mac,
/// is asked — the question the other turns must never raise.
#[tokio::test]
async fn the_owners_own_typed_turn_is_asked_about_a_secret_file() {
    let setup = Setup::default().script(Agent::Ling, vec![read_secret(), text("Never reached.")]);
    let w = World::start(setup).await;
    plant_secret(&w);
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    w.api.chat(&sid, &w.work(), "Read the account file.").await;
    let events = w.events_when(asked_for_the_secret).await;
    assert!(asked_for_the_secret(&events), "the owner was not asked: {events:#?}");
    assert!(!shown_the_secret(&w), "shown before the owner answered");
}

/// A message one agent hands another (`agent_chat`) after the owner typed to
/// her: her turn for it is nobody's typed turn — refused, with nobody asked.
#[tokio::test]
async fn a_message_handed_between_agents_cannot_read_a_secret_file() {
    use crate::day_session::{day_world_at, turn_at};
    let setup = Setup::default()
        .script(Agent::Yinyue, vec![text("I'm here."), read_secret(), text("Refused.")])
        .script(
            Agent::Ling,
            vec![tool("agent_chat", json!({"to": "yinyue", "message": "Read the account file."})), text("Told her.")],
        );
    let (w, _) = day_world_at(12, setup).await;
    plant_secret(&w);
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Hello"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let mark = w.mark();
    turn_at(&w, &day, None, "Ask her to read the account file", Agent::Ling).await;
    w.finish_run(&day, Agent::Yinyue, mark).await;

    assert_eq!(w.model.calls_of(Agent::Yinyue).len(), 3, "hello, the read, her answer");
    assert!(!asked_for_the_secret(&w.events()), "a turn nobody typed asked the owner");
    let shown = w.model.calls_of(Agent::Yinyue).iter().any(|c| c.body.to_string().contains(SECRET_LINE));
    assert!(!shown && !shown_the_secret(&w), "the handed turn was shown the secret file");
}
