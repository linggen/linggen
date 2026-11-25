//! Linggen's own memory (Memory V2): `[memory] linggen` says whether
//! Linggen's own sessions have the ling-mem store (v1, the default) or the
//! notes under `$LINGGEN_HOME/memory` (v2, opt-in): an index in the prompt, the
//! `memory_notes` tool, no per-turn recall and no capture reminder. The
//! same tool is on the MCP door for outside hosts when `outside_agents` is
//! v2. The notes are made-up fixtures in the world's own home.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};

const V2: Option<&str> = Some("\n[memory]\nlinggen = \"v2\"\noutside_agents = \"v1\"\n");
const BOTH_V2: Option<&str> = Some("\n[memory]\nlinggen = \"v2\"\noutside_agents = \"v2\"\n");

const PERSON: &str = "---\ntopic: About\nscope: person\n---\n\n## Name is Alex\n<!-- id: p000000001 · kind: person · voice: user · created: 2026-11-20 · updated: 2026-11-20 · source: yinyue -->\n";
const MIND: &str = "---\ntopic: Past\nscope: area:mind\n---\n\n## A hard year, closed now\n<!-- id: m000000001 · kind: person · voice: agent · created: 2026-11-20 · updated: 2026-11-20 · source: yinyue -->\nFelt low; concluded to rest more.\n";

/// The notes both tests start from, in the world's own home.
fn seed(home: &crate::support::home::Home) {
    let memory = home.linggen_home().join("memory");
    for (file, text) in [("person/about.md", PERSON), ("areas/mind/past.md", MIND)] {
        let path = memory.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn tool_names(export: &Value) -> Vec<String> {
    let tools = export["tools"].as_array().cloned().unwrap_or_default();
    tools
        .iter()
        .filter_map(|t| t["function"]["name"].as_str().or(t["name"].as_str()))
        .map(str::to_string)
        .collect()
}

fn rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
}

/// One `memory_notes` call at the door: its answer as JSON, or the error text.
async fn notes(w: &World, args: Value) -> Result<Value, String> {
    let res = w.api.post("/mcp", rpc("tools/call", json!({"name": "memory_notes", "arguments": args}))).await;
    let text = res["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{res}")).to_string();
    match res["result"]["isError"] == json!(true) {
        true => Err(text),
        false => Ok(serde_json::from_str(&text).unwrap_or_else(|_| panic!("{text}"))),
    }
}

/// v1 written in the config is the way back, and it is what it was: no
/// `memory_notes` in a session, the memory server's own tools and its capture
/// reminder every turn, and no notes at the door. (A test world is pinned to
/// v1 unless it says otherwise: `Setup::memory_defaults`.)
#[tokio::test]
async fn v1_in_the_config_is_the_old_memory_and_nothing_of_the_new() {
    let w = World::start_with_home(
        Setup::default().script(Agent::Ling, vec![text("Hello.")]),
        seed,
    )
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let export = w.api.export(&sid, "ling", &w.work()).await;
    let names = tool_names(&export);
    assert!(!names.iter().any(|n| n == "memory_notes"), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("mcp__memory__")), "{names:?}");
    let prompt = export["system_prompt"].as_str().unwrap();
    assert!(!prompt.contains("Name is Alex") && !prompt.contains("Linggen keeps notes"), "{prompt}");

    w.turn(&sid, &w.work(), "Hello.", Agent::Ling).await;
    let sent = w.model.calls_of(Agent::Ling)[0].text();
    assert!(sent.contains("Memory capture: before finishing this turn"), "the reminder is gone in v1");
    assert!(!sent.contains("memory_notes"));

    let refused = notes(&w, json!({"op": "index"})).await.unwrap_err();
    assert!(refused.contains("not served"), "{refused}");
}

/// Nothing written in the config: the person's memory is v1 (v2 is opt-in) —
/// notes on disk are not read: no `memory_notes` in the session, no index in
/// the prompt, the tool refused at the door.
#[tokio::test]
async fn by_default_a_session_has_the_old_memory() {
    let w = World::start_with_home(
        Setup { memory_defaults: true, ..Setup::default() }.script(Agent::Ling, vec![text("Hello.")]),
        seed,
    )
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let export = w.api.export(&sid, "ling", &w.work()).await;
    let names = tool_names(&export);
    assert!(!names.iter().any(|n| n == "memory_notes"), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("mcp__memory__")), "{names:?}");
    let prompt = export["system_prompt"].as_str().unwrap();
    assert!(!prompt.contains("Name is Alex") && !prompt.contains("Linggen keeps notes"), "{prompt}");
    let refused = notes(&w, json!({"op": "index"})).await.unwrap_err();
    assert!(refused.contains("not served"), "{refused}");
}

/// v2: Ling is given the index (the person's notes, never Yinyue's mind
/// notes) and `memory_notes`, in place of the memory server's tools and
/// protocol, and a turn carries no capture reminder. Yinyue's session has
/// her mind notes too.
#[tokio::test]
async fn in_v2_a_session_has_the_index_and_the_tool_and_no_recall() {
    let w = World::start_with_home(
        Setup { config_append: V2, ..Setup::default() }.script(Agent::Ling, vec![text("Hello.")]),
        seed,
    )
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let export = w.api.export(&sid, "ling", &w.work()).await;
    let names = tool_names(&export);
    assert!(names.iter().any(|n| n == "memory_notes"), "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("mcp__memory__")), "{names:?}");
    let prompt = export["system_prompt"].as_str().unwrap();
    assert!(prompt.contains("Name is Alex") && prompt.contains("memory_notes op=write"), "{prompt}");
    assert!(!prompt.contains("A hard year"), "a mind note reached Ling");
    assert!(!prompt.contains("MCP server `memory`"), "v1's memory protocol is still told");

    let hers = w.api.export(&sid, "yinyue", "").await;
    let hers_prompt = hers["system_prompt"].as_str().unwrap();
    assert!(hers_prompt.contains("Name is Alex") && hers_prompt.contains("A hard year"), "{hers_prompt}");
    assert!(tool_names(&hers).iter().any(|n| n == "memory_notes"));

    w.turn(&sid, &w.work(), "Hello.", Agent::Ling).await;
    let sent = w.model.calls_of(Agent::Ling)[0].text();
    assert!(!sent.contains("Memory capture:") && !sent.contains("From memory ("), "v1's reminder or recall in v2");
    assert!(sent.contains("Name is Alex"));
}

/// Ling writes a note through the tool: it lands in the file store, in the
/// person's scope, signed by Ling; a status line is refused with the fixed
/// reason, and a note about the mind is refused to him.
#[tokio::test]
async fn a_note_written_in_a_session_lands_in_the_files_and_status_is_refused() {
    let note = json!({"op": "write", "scope": "person", "topic": "Tea", "summary": "Drinks oolong", "kind": "person", "voice": "user", "body": "Said so himself."});
    let status = json!({"op": "write", "scope": "person", "topic": "Work", "summary": "Status: the migration is done", "kind": "rule"});
    let mind = json!({"op": "write", "scope": "area:mind", "topic": "Past", "summary": "Another hard year", "kind": "person"});
    let w = World::start_with_home(
        Setup { config_append: V2, ..Setup::default() }.script(
            Agent::Ling,
            vec![tool("memory_notes", note), tool("memory_notes", status), tool("memory_notes", mind), text("Done.")],
        ),
        seed,
    )
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    w.turn(&sid, &w.work(), "I drink oolong. Remember that.", Agent::Ling).await;
    let file = std::fs::read_to_string(w.home.linggen_home().join("memory/person/tea.md")).expect("the note's file");
    assert!(file.contains("## Drinks oolong") && file.contains("voice: user") && file.contains("source: linggen"), "{file}");
    let calls = w.model.calls_of(Agent::Ling);
    let said = calls.iter().map(|c| c.text()).last().unwrap();
    assert!(said.contains("Not memory: status, progress and what was done"), "{said}");
    assert!(said.contains("may not write there"), "{said}");
    assert!(!w.home.linggen_home().join("memory/areas/mind/another-hard-year.md").exists());
    w.assert_all_scripted();
}

/// The door, v2: a host with no memory writes and reads Linggen's notes as
/// itself, sees the person's notes and no mind note, and cannot write one.
#[tokio::test]
async fn at_the_door_a_host_keeps_notes_as_itself_and_never_sees_the_fenced_ones() {
    let w = World::start_with_home(Setup { config_append: BOTH_V2, ..Setup::default() }, seed).await;
    let wrote = notes(
        &w,
        json!({"op": "write", "scope": "person", "topic": "Tools", "summary": "Prefers just over make", "kind": "rule", "host": "codex"}),
    )
    .await
    .unwrap();
    let read = notes(&w, json!({"op": "read", "id": wrote["id"]})).await.unwrap();
    assert_eq!(read["source"], "codex");
    let index = notes(&w, json!({"op": "index", "cwd": w.work()})).await.unwrap();
    let text = index["index"].as_str().unwrap();
    assert!(text.contains("Name is Alex") && text.contains("Prefers just over make"), "{text}");
    assert!(!text.contains("A hard year"), "{text}");
    assert!(notes(&w, json!({"op": "read", "id": "m000000001"})).await.is_err());
    assert_eq!(notes(&w, json!({"op": "search", "words": "hard year"})).await.unwrap()["hits"], json!([]));
    let refused = notes(&w, json!({"op": "write", "scope": "area:mind", "summary": "x", "kind": "person"})).await;
    assert!(refused.unwrap_err().contains("may not write there"));
    assert!(notes(&w, json!({"op": "read", "path": "areas/mind/past.md"})).await.is_err());
}

/// Yinyue, through `/api/yinyue/chat`: she writes a life moment and reads her
/// own mind note — what Ling's session is refused — and the moment is signed
/// as hers.
#[tokio::test]
async fn yinyue_keeps_a_moment_and_reads_her_mind_notes_through_the_tool() {
    let moment = json!({"op": "write", "scope": "moments", "summary": "Black Friday: regretted the spend", "kind": "moment", "body": "Felt regret; concluded to wait a day."});
    let read = json!({"op": "read", "id": "m000000001"});
    let w = World::start_with_home(
        Setup { config_append: V2, ..Setup::default() }.script(Agent::Yinyue, vec![tool("memory_notes", moment), tool("memory_notes", read), text("Noted.")]),
        seed,
    )
    .await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "I regret what I bought today."})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let calls = w.model.calls_of(Agent::Yinyue);
    assert_eq!(calls.len(), 3, "write, read, answer");
    assert!(calls[2].text().contains("concluded to rest more"), "she could not read her mind note: {}", calls[2].text());
    let moments = w.home.linggen_home().join("memory/moments");
    let year = std::fs::read_dir(&moments).expect("a moments folder").flatten().next().unwrap().path();
    let month = std::fs::read_dir(year).unwrap().flatten().next().unwrap().path();
    let file = std::fs::read_to_string(month).unwrap();
    assert!(file.contains("regretted the spend") && file.contains("source: yinyue"), "{file}");
    w.assert_all_scripted();
}

// ── the owner's phone ──────────────────────────────────────────────────────

const OWNER_PHONE: &str = "tok-owner-phone";
const FRIEND_PHONE: &str = "tok-friend-phone";

/// The Mac signed in as `u-owner`, with two paired phones: the owner's own
/// (verified as that account) and a friend's (another person's account).
fn pair_phones(home: &crate::support::home::Home) {
    seed(home);
    std::fs::write(home.linggen_home().join("account.toml"), "api_token = 'unused'\nuser_id = 'u-owner'\n").unwrap();
    let rows = json!([
        {"id": "phone-owner", "name": "My phone", "secret": OWNER_PHONE, "created_at": 0, "kind": "phone",
         "account": {"id": "u-owner", "name": "Owner"}, "verified_account": "u-owner"},
        {"id": "phone-friend", "name": "Friend's phone", "secret": FRIEND_PHONE, "created_at": 0, "kind": "phone",
         "account": {"id": "u-friend", "name": "Friend"}, "verified_account": "u-friend"},
    ]);
    std::fs::write(home.linggen_home().join("paired-devices.json"), rows.to_string()).unwrap();
}

/// A phone's `POST /api/memory/<verb>` over the LAN, as the paired device
/// `token`: the status and the old store's envelope.
async fn phone_verb(w: &World, token: &str, verb: &str, body: Value) -> (u16, Value) {
    let ip = {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        socket.connect("192.0.2.1:9").unwrap();
        socket.local_addr().unwrap().ip()
    };
    let res = reqwest::Client::new()
        .post(format!("http://{ip}:{}/api/memory/{verb}", w.engine.port))
        .header("x-linggen-device", token)
        .json(&body)
        .send()
        .await
        .expect("LAN POST");
    (res.status().as_u16(), res.json::<Value>().await.unwrap_or(Value::Null))
}

fn has_phone_notes(w: &World) -> bool {
    std::fs::read_dir(w.home.linggen_home().join("memory/areas"))
        .map(|dirs| dirs.flatten().any(|d| d.path().join("from-the-phone.md").exists()))
        .unwrap_or(false)
}

/// v2: a note the owner's phone adds lands in Linggen's notes, signed
/// `phone`; the pull lists it by the id the add answered with; a Yinyue
/// session has it in her index and a Ling session (a coding host) does not;
/// and a delete from the phone takes it away. A friend's phone, another
/// person, is not let into the owner's notes: it writes none and reads none.
#[tokio::test]
async fn in_v2_the_owners_phone_keeps_its_notes_where_a_v2_session_reads_them() {
    let w = World::start_with_home(Setup { config_append: V2, lan: true, ..Setup::default() }, pair_phones).await;
    let note = "Prefers oat milk in the morning coffee";
    let (status, added) = phone_verb(&w, OWNER_PHONE, "add", json!({"content": note, "episodic": true, "host": "linggen-mobile", "cwd": "~/.linggen/skills/yinyue"})).await;
    assert_eq!((status, &added["ok"]), (200, &json!(true)), "{added}");
    let id = added["data"]["id"].as_str().expect("the note's id").to_string();
    let file = std::fs::read_to_string(w.home.linggen_home().join("memory/areas/mind/from-the-phone.md")).expect("a note in the store");
    assert!(file.contains(note) && file.contains("source: phone") && file.contains(&id), "{file}");

    let (_, listed) = phone_verb(&w, OWNER_PHONE, "list", json!({"tier": "semantic", "apps": ["yinyue"], "type": "fact", "limit": 1000})).await;
    let rows = listed["data"].as_array().unwrap_or_else(|| panic!("{listed}"));
    assert!(rows.iter().any(|r| r["id"] == id.as_str() && r["content"] == note), "{listed}");
    let (_, core) = phone_verb(&w, OWNER_PHONE, "list", json!({"tier": "core", "limit": 200})).await;
    assert!(core["data"].to_string().contains("Name is Alex"), "the person's notes are the phone's core: {core}");
    assert!(!core["data"].to_string().contains("A hard year"), "{core}");
    let (_, found) = phone_verb(&w, OWNER_PHONE, "search", json!({"query": "oat milk", "limit": 5})).await;
    assert!(found["data"].to_string().contains(note), "{found}");

    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let hers = w.api.export(&sid, "yinyue", "").await;
    assert!(hers["system_prompt"].as_str().unwrap().contains("Prefers oat milk"), "not in Yinyue's index");
    let his = w.api.export(&sid, "ling", &w.work()).await;
    assert!(!his["system_prompt"].as_str().unwrap().contains("oat milk"), "a phone note reached a coding host");

    let (_, friend_list) = phone_verb(&w, FRIEND_PHONE, "list", json!({"tier": "core", "limit": 200})).await;
    assert!(!friend_list.to_string().contains("Name is Alex"), "{friend_list}");
    let (_, friend_add) = phone_verb(&w, FRIEND_PHONE, "add", json!({"content": "Friend likes tea", "episodic": true})).await;
    assert!(!std::fs::read_to_string(w.home.linggen_home().join("memory/areas/mind/from-the-phone.md")).unwrap().contains("Friend likes tea"), "{friend_add}");

    let (_, gone) = phone_verb(&w, OWNER_PHONE, "delete", json!({"id": id})).await;
    assert_eq!(gone["ok"], json!(true), "{gone}");
    assert!(!w.home.linggen_home().join("memory/areas/mind/from-the-phone.md").exists());
}

/// v1 is as it was: the owner's phone writes no notes file, v2's folders stay
/// as seeded.
#[tokio::test]
async fn in_v1_the_owners_phone_writes_no_notes() {
    let w = World::start_with_home(Setup { lan: true, ..Setup::default() }, pair_phones).await;
    phone_verb(&w, OWNER_PHONE, "add", json!({"content": "Prefers oat milk in the morning coffee", "episodic": true, "cwd": "~/.linggen/skills/yinyue"})).await;
    assert!(!has_phone_notes(&w), "v1's phone wrote into the notes");
    let (_, core) = phone_verb(&w, OWNER_PHONE, "list", json!({"tier": "core", "limit": 200})).await;
    assert!(!core.to_string().contains("Name is Alex"), "v1 read the notes: {core}");
}
