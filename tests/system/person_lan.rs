//! A day is its own person's over plain HTTP from the LAN too
//! (doc/one-ling-spec.md § Linggen's daily session): the LAN gate resolves the
//! person from the paired device's token, as the channel does, and sends
//! the request as theirs whatever `user_id` it named.

use crate::day_session::{day_world_at, say};
use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};

const TOKEN: &str = "tok-friend-phone";

/// This machine's own address on its network: a request from it reaches
/// the engine as from the LAN, not from loopback.
fn lan_address() -> std::net::IpAddr {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").expect("udp socket");
    // A UDP connect sends nothing: it only picks the route's source address.
    socket
        .connect("192.0.2.1:9")
        .expect("this machine has no network address: the LAN path cannot be tested");
    let ip = socket.local_addr().expect("local address").ip();
    assert!(!ip.is_loopback() && !ip.is_unspecified(), "no LAN address: {ip}");
    ip
}

/// A phone paired to this Mac and signed in to another person's account.
fn pair_friends_phone(w: &World) {
    let row = json!([{
        "id": "phone-friend", "name": "Friend's phone", "secret": TOKEN, "created_at": 0,
        "kind": "phone", "account": {"id": "u-friend", "name": "Friend"},
    }]);
    std::fs::write(w.home.linggen_home().join("paired-devices.json"), row.to_string()).unwrap();
}

/// The engine as the friend's phone reaches it over the LAN.
struct Lan {
    base: String,
    client: reqwest::Client,
}

impl Lan {
    fn new(w: &World) -> Self {
        Self {
            base: format!("http://{}:{}", lan_address(), w.engine.port),
            client: reqwest::Client::new(),
        }
    }

    async fn status(&self, path: &str, token: Option<&str>) -> u16 {
        let mut req = self.client.get(format!("{}{path}", self.base));
        if let Some(t) = token {
            req = req.header("x-linggen-device", t);
        }
        req.send().await.expect("LAN GET").status().as_u16()
    }

    async fn get(&self, path: &str) -> Value {
        let resp = self
            .client
            .get(format!("{}{path}", self.base))
            .header("x-linggen-device", TOKEN)
            .send()
            .await
            .expect("LAN GET");
        resp.json().await.unwrap_or(Value::Null)
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .header("x-linggen-device", TOKEN)
            .json(&body)
            .send()
            .await
            .expect("LAN POST");
        resp.json().await.unwrap_or(Value::Null)
    }
}

/// Another person's paired phone, over plain HTTP from the LAN, is given
/// their own day and refused the builder's by its id: the day's lookup,
/// a page's history of it, and a chat naming it (refused) — whatever
/// `user_id` the request names; nor is it given the builder's notes.
#[tokio::test]
async fn another_persons_phone_over_the_lan_gets_its_own_day_and_never_the_builders() {
    let setup = Setup {
        lan: true,
        ..Setup::default()
    }
    .script(Agent::Ling, vec![text("BUILDER-REPLY."), text("For the friend.")]);
    let (w, _) = day_world_at(12, setup).await;
    pair_friends_phone(&w);
    let lan = Lan::new(&w);
    assert_eq!(lan.status("/api/sessions/day", None).await, 401, "not the LAN path");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lan.status("/api/pair/me", Some(TOKEN)).await != 200 {
        assert!(std::time::Instant::now() < deadline, "the phone is not known");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let mine = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let mark = w.mark();
    say(&w, &mine, Some("dice"), "BUILDER-WORDS").await;
    w.finish_run(&mine, Agent::Ling, mark).await;

    let mut wrong = Vec::new();
    let theirs = lan.get("/api/sessions/day").await["id"].as_str().unwrap_or_default().to_string();
    if theirs == mine || !theirs.contains("-p-") {
        wrong.push(format!("the day's lookup gave {theirs:?}, the builder's is {mine}"));
    }
    let named = lan.get("/api/sessions/day?user_id=__local__").await;
    if named["id"] == json!(mine) {
        wrong.push(format!("naming the builder, the lookup gave the builder's day: {named}"));
    }
    let aliased = lan.get("/api/sessions/d%61y").await;
    if aliased["id"] == json!(mine) {
        wrong.push(format!("a path written another way gave the builder's day: {aliased}"));
    }
    let page = format!("/api/skill-sessions/state?skill=dice&session_id={mine}");
    let shown = lan.get(&page).await["messages"].to_string();
    if shown.contains("BUILDER-WORDS") {
        wrong.push("a page's history of the builder's day was shown".into());
    }
    let body = json!({"session_id": mine, "project_root": w.work(), "message": "FRIEND-WORDS", "skill_name": "dice"});
    let (status, sent) = lan.send(reqwest::Method::POST, "/api/chat", Some(body)).await;
    if status != 403 {
        wrong.push(format!("a chat naming the builder's day was not refused: {status} {sent}"));
    }
    let mark = w.mark();
    let body = json!({"project_root": w.work(), "message": "FRIEND-OWN", "skill_name": "dice"});
    let sent = lan.post("/api/chat", body).await;
    let landed = sent["session_id"].as_str().unwrap_or_default().to_string();
    if landed != theirs {
        wrong.push(format!("the friend's own chat did not land in their day: {sent}"));
    } else {
        w.finish_run(&landed, Agent::Ling, mark).await;
    }
    let notes = w.home.linggen_home().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let book = json!({"next": 2, "notes": [{"id": "n1", "text": "BUILDER-NOTE", "written_at": 0, "device": "mac"}]});
    std::fs::write(notes.join("yinyue.json"), book.to_string()).unwrap();
    if lan.get("/api/notes?agent=yinyue&user_id=__local__").await.to_string().contains("BUILDER-NOTE") {
        wrong.push("the builder's notes were read".into());
    }
    let rows = serde_json::to_string(&w.raw_rows(&mine)).unwrap();
    if rows.contains("FRIEND-WORDS") {
        wrong.push("the friend's words are in the builder's day".into());
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[tokio::test]
async fn a_signed_out_phone_can_only_read_and_write_its_own_day() {
    use reqwest::Method;

    let setup = Setup { lan: true, ..Setup::default() }
        .script(Agent::Ling, vec![text("Owner reply."), text("Phone reply.")]);
    let (w, _) = day_world_at(12, setup).await;
    let lan = pair_signed_out_phone(&w).await;
    let owner = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let mark = w.mark();
    say(&w, &owner, Some("dice"), "OWNER-DAY-WORDS").await;
    w.finish_run(&owner, Agent::Ling, mark).await;
    let owner_rows = w.raw_rows(&owner);

    let (status, day) = lan.send(Method::GET, "/api/sessions/day?user_id=__local__", None).await;
    assert_eq!(status, 200, "{day}");
    assert_eq!(day["on"], true);
    let phone = day["id"].as_str().unwrap();
    let device_hex: String = b"device:phone-signed-out".iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(phone, format!("{owner}-p-{device_hex}"));
    let (status, list) = lan.send(Method::GET, "/api/skill-sessions?skill=dice", None).await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["sessions"][0]["id"], phone);
    assert!(!list.to_string().contains(&format!("\"{owner}\"")), "{list}");

    let page = format!("/api/skill-sessions/state?skill=dice&session_id={owner}");
    let (status, body) = lan.send(Method::GET, &page, None).await;
    assert_eq!(status, 403, "{body}");
    let chat = json!({"session_id": owner, "project_root": w.work(), "message": "PHONE-REFUSED", "skill_name": "dice"});
    let (status, body) = lan.send(Method::POST, "/api/chat", Some(chat)).await;
    assert_eq!(status, 403, "{body}");

    let mark = w.mark();
    let chat = json!({"project_root": w.work(), "message": "PHONE-OWN", "skill_name": "dice"});
    let (status, body) = lan.send(Method::POST, "/api/chat", Some(chat)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["session_id"], phone, "{body}");
    w.finish_run(phone, Agent::Ling, mark).await;
    assert!(w.raw_rows(phone).iter().any(|row| row["content"] == "PHONE-OWN"));
    assert_eq!(w.raw_rows(&owner), owner_rows, "the owner's day was changed");
}

#[tokio::test]
async fn a_signed_out_phone_is_not_given_the_owners_notes_facts_or_companion_thread() {
    use ling_mem::memory::{Memory, MemoryStore, MemoryType, Origin, Tier};
    use reqwest::Method;

    let (w, _) = day_world_at(12, Setup { lan: true, ..Setup::default() }).await;
    let home = w.home.linggen_home();
    let notes = home.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let book = json!({"next": 2, "notes": [{"id": "n1", "text": "OWNER-NOTE", "written_at": 0, "device": "mac"}]});
    std::fs::write(notes.join("yinyue.json"), book.to_string()).unwrap();
    // Seed a core fact directly so the fixture needs no embedding model.
    let store = MemoryStore::open_semantic(&home).await.unwrap();
    let mut fact = Memory::new("OWNER-FACT", MemoryType::Fact, Origin::User);
    fact.tier = Tier::Core;
    store.insert(&[fact]).await.unwrap();

    let companion = "sess-yinyue-2020-01-01";
    let thread = home.join("sessions").join(companion);
    std::fs::create_dir_all(&thread).unwrap();
    std::fs::write(thread.join("session.yaml"), format!("id: {companion}\ntitle: Companion\ncreated_at: 1\ncreator: agent\n")).unwrap();
    let line = json!({"agent_id": "yinyue", "from_id": "yinyue", "to_id": "user", "content": "OWNER-COMPANION", "timestamp": 1, "is_observation": false});
    std::fs::write(thread.join("messages.jsonl"), format!("{line}\n")).unwrap();

    let notes_path = "/api/notes?agent=yinyue";
    let memory_path = "/api/memory/session_start";
    let thread_path = format!("/api/skill-sessions/state?skill=dice&session_id={companion}");
    assert!(w.api.get(notes_path).await.to_string().contains("OWNER-NOTE"));
    assert!(w.api.post(memory_path, json!({})).await.to_string().contains("OWNER-FACT"));
    assert!(w.api.get(&thread_path).await.to_string().contains("OWNER-COMPANION"));

    let lan = pair_signed_out_phone(&w).await;
    let (status, notes) = lan.send(Method::GET, notes_path, None).await;
    assert_eq!(status, 200, "{notes}");
    assert_eq!(notes["notes"], json!([]));
    let (status, memory) = lan.send(Method::POST, memory_path, Some(json!({}))).await;
    assert_eq!(status, 200, "{memory}");
    assert_eq!(memory["data"]["core"], json!([]), "{memory}");
    let (status, body) = lan.send(Method::GET, &thread_path, None).await;
    assert_eq!(status, 403, "{body}");
    let (status, body) = lan.send(Method::POST, "/api/yinyue/chat", Some(json!({"text": "Phone words"}))).await;
    assert_eq!(status, 403, "{body}");
}

async fn pair_signed_out_phone(w: &World) -> Lan {
    let row = json!([{
        "id": "phone-signed-out", "name": "Signed-out phone", "secret": TOKEN,
        "created_at": 0, "kind": "phone", "contract": 1, "permission": "read",
    }]);
    std::fs::write(w.home.linggen_home().join("paired-devices.json"), row.to_string()).unwrap();
    let lan = Lan::new(w);
    assert_eq!(lan.status("/api/pair/me", None).await, 401, "not the LAN path");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lan.status("/api/pair/me", Some(TOKEN)).await != 200 {
        assert!(std::time::Instant::now() < deadline, "the phone is not known");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    lan
}

/// With the setting off, plain HTTP from the LAN is as before: a body the
/// gate cannot read is passed through untouched (the route refuses it as
/// it always did, never an empty one in its place), and a phone of the
/// owner's that is signed out still reads the notes.
#[tokio::test]
async fn with_the_setting_off_the_lan_path_is_as_before() {
    let w = World::start(Setup {
        lan: true,
        daily_session: Some(false),
        ..Setup::default()
    })
    .await;
    let row = json!([{
        "id": "phone-own", "name": "Own phone", "secret": TOKEN, "created_at": 0, "kind": "phone",
    }]);
    std::fs::write(w.home.linggen_home().join("paired-devices.json"), row.to_string()).unwrap();
    let lan = Lan::new(&w);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lan.status("/api/pair/me", Some(TOKEN)).await != 200 {
        assert!(std::time::Instant::now() < deadline, "the phone is not known");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let notes = w.home.linggen_home().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let book = json!({"next": 2, "notes": [{"id": "n1", "text": "OWN-NOTE", "written_at": 0, "device": "mac"}]});
    std::fs::write(notes.join("yinyue.json"), book.to_string()).unwrap();

    let mut wrong = Vec::new();
    let send = |base: String, token: Option<&'static str>| {
        let mut req = lan
            .client
            .post(format!("{base}/api/chat"))
            .header("content-type", "application/json")
            .body("{not json");
        if let Some(t) = token {
            req = req.header("x-linggen-device", t);
        }
        async move { req.send().await.expect("POST").status().as_u16() }
    };
    let here = send(w.engine.base_url(), None).await;
    let over_lan = send(lan.base.clone(), Some(TOKEN)).await;
    if here != over_lan {
        wrong.push(format!("a body that is not JSON was replaced: {over_lan} over the LAN, {here} here"));
    }
    if !lan.get("/api/notes?agent=yinyue").await.to_string().contains("OWN-NOTE") {
        wrong.push("the signed-out phone lost the notes".into());
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

impl Lan {
    /// One request with the friend's token: its status and body.
    async fn send(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header("x-linggen-device", TOKEN);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.expect("LAN request");
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }
}

/// Every kind of route that takes a session, asked for the builder's day by
/// another person's phone over the LAN: each is refused, and nothing of
/// the day is read, answered, cleared or deleted.
#[tokio::test]
async fn another_persons_phone_over_the_lan_is_refused_the_builders_day_by_every_route() {
    use reqwest::Method;
    let ask = json!({"questions": [{"question": "BUILDER-QUESTION which box?", "header": "Box",
        "options": [{"label": "Red"}, {"label": "Blue"}]}]});
    let setup = Setup {
        lan: true,
        ..Setup::default()
    }
    .script(Agent::Ling, vec![tool("AskUser", ask), text("Red.")]);
    let (w, _) = day_world_at(12, setup).await;
    pair_friends_phone(&w);
    let lan = Lan::new(&w);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lan.status("/api/pair/me", Some(TOKEN)).await != 200 {
        assert!(std::time::Instant::now() < deadline, "the phone is not known");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let mine = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let mark = w.mark();
    say(&w, &mine, Some("dice"), "BUILDER-WORDS pick a box").await;
    let question = loop {
        let asks = w.api.get("/api/pending-ask-user?skill=dice").await;
        if let Some(ask) = asks.as_array().and_then(|a| a.first()).cloned() {
            break ask;
        }
        assert!(std::time::Instant::now() < deadline + std::time::Duration::from_secs(20), "no question");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    let run_id = w
        .engine
        .log_since(mark)
        .split("run_id=")
        .nth(1)
        .map(|r| r.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').next().unwrap_or("").to_string())
        .unwrap_or_default();
    let q = question["question_id"].clone();
    let refused = |s: u16| s == 403;
    let mut wrong = Vec::new();
    let probe = |wrong: &mut Vec<String>, what: &str, (status, body): (u16, Value)| {
        if !refused(status) {
            wrong.push(format!("{what}: {status} {body}"));
        }
    };
    let enc = |s: &str| urlencoding::encode(s).into_owned();
    let read = lan.send(Method::GET, "/api/pending-ask-user?skill=dice", None).await;
    if read.1.to_string().contains("BUILDER-QUESTION") {
        wrong.push(format!("read the question: {}", read.1));
    }
    let answer = json!({"question_id": q, "answers": [{"question_index": 0, "selected": ["Red"], "custom_text": null}]});
    lan.send(Method::POST, "/api/ask-user-response", Some(answer)).await;
    probe(&mut wrong, "clear", lan.send(Method::POST, "/api/chat/clear", Some(json!({"session_id": mine}))).await);
    probe(&mut wrong, "approve a plan", lan.send(Method::POST, "/api/plan/approve", Some(json!({"session_id": mine}))).await);
    let grant = json!({"session_id": mine, "path": w.work(), "mode": "admin"});
    probe(&mut wrong, "change permission", lan.send(Method::PATCH, "/api/sessions/permission", Some(grant)).await);
    lan.send(Method::POST, "/api/agent-cancel", Some(json!({"run_id": run_id}))).await;
    let pic = format!("/api/sessions/images?session_id={}&name=x.png", enc(&mine));
    probe(&mut wrong, "pictures", lan.send(Method::GET, &pic, None).await);
    let prompt = format!("/api/chat/system-prompt?session_id={}&agent_id=ling&project_root={}", enc(&mine), enc(&w.work()));
    probe(&mut wrong, "prompt export", lan.send(Method::GET, &prompt, None).await);
    probe(&mut wrong, "her chat", lan.send(Method::POST, "/api/yinyue/chat", Some(json!({"text": "hi"}))).await);
    probe(&mut wrong, "delete", lan.send(Method::DELETE, "/api/sessions/all", Some(json!({"session_id": mine}))).await);

    let still = w.api.get("/api/pending-ask-user?skill=dice").await;
    if !still.to_string().contains("BUILDER-QUESTION") {
        wrong.push(format!("the question was answered or its turn stopped (run {run_id})"));
    }
    let thread = w.home.linggen_home().join("sessions").join(&mine).join("messages.jsonl");
    if !std::fs::read_to_string(thread).unwrap_or_default().contains("BUILDER-WORDS") {
        wrong.push("the day was cleared or deleted".into());
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// What a phone publishes is its person's: another person's phone, over the
/// LAN, keeps no reading on this Mac and wakes nothing of the builder's — a
/// topic it retains is not kept, its app moment is not taken up — while the
/// builder's own publish is kept as before.
#[tokio::test]
async fn another_persons_phone_over_the_lan_changes_nothing_the_builder_reads() {
    let setup = Setup {
        lan: true,
        ..Setup::default()
    };
    let (w, _) = day_world_at(12, setup).await;
    pair_friends_phone(&w);
    let lan = Lan::new(&w);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lan.status("/api/pair/me", Some(TOKEN)).await != 200 {
        assert!(std::time::Instant::now() < deadline, "the phone is not known");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let mut wrong = Vec::new();
    let publish = json!({"topic": "phone", "op": "status", "payload": {"who": "FRIEND"}, "retain": true});
    let (status, _) = lan.send(reqwest::Method::POST, "/api/topic/publish", Some(publish)).await;
    let latest = format!("{}/api/topic/latest?topic=phone&op=status", w.engine.base_url());
    let kept = reqwest::get(&latest).await.expect("latest").text().await.unwrap_or_default();
    if kept.contains("FRIEND") {
        wrong.push(format!("the friend's reading was kept ({status}): {kept}"));
    }
    let moment = json!({"app": "dice", "text": "FRIEND-MOMENT", "asked": true});
    let (status, _) = lan.send(reqwest::Method::POST, "/api/yinyue/event", Some(moment)).await;
    if status != 409 {
        wrong.push(format!("the friend's moment was taken up: {status}"));
    }
    let own = json!({"topic": "phone", "op": "status", "payload": {"who": "BUILDER"}, "retain": true});
    w.api.post("/api/topic/publish", own).await;
    let kept = w.api.get("/api/topic/latest?topic=phone&op=status").await;
    if !kept.to_string().contains("BUILDER") {
        wrong.push(format!("the builder's own reading was not kept: {kept}"));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A body past the framework's limit is refused over the LAN as it is
/// here, whether it says its length or comes in chunks; one under it goes
/// through as before.
#[tokio::test]
async fn a_body_past_the_limit_is_answered_over_the_lan_as_here() {
    let w = World::start(Setup {
        lan: true,
        ..Setup::default()
    })
    .await;
    pair_friends_phone(&w);
    let lan = Lan::new(&w);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lan.status("/api/pair/me", Some(TOKEN)).await != 200 {
        assert!(std::time::Instant::now() < deadline, "the phone is not known");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let big = json!({"text": "x".repeat(3 * 1024 * 1024)}).to_string();
    // No kept connections: one a refused body closed is never reused.
    let client = reqwest::Client::builder().pool_max_idle_per_host(0).build().unwrap();
    let post = |base: String, token: Option<&'static str>, body: reqwest::Body| {
        let mut req = client
            .post(format!("{base}/api/presence"))
            .header("content-type", "application/json")
            .body(body);
        if let Some(t) = token {
            req = req.header("x-linggen-device", t);
        }
        async move { req.send().await.map(|r| r.status().as_u16()).unwrap_or(0) }
    };
    // Refused is a 413 — or no answer at all (0): the engine answers before
    // the body is all sent and closes, and a caller still writing may get
    // the reset instead of the answer. Which of the two is a race in the
    // transport, not a rule; what must never happen is the body let through.
    let refused = |status: u16| status == 413 || status == 0;
    let mut wrong = Vec::new();
    let here = post(w.engine.base_url(), None, big.clone().into()).await;
    let over_lan = post(lan.base.clone(), Some(TOKEN), big.clone().into()).await;
    if !refused(here) || !refused(over_lan) {
        wrong.push(format!("a long body: {over_lan} over the LAN, {here} here"));
    }
    let chunks = || {
        let parts: Vec<_> = big.as_bytes().chunks(64 * 1024).map(|c| Ok::<_, std::io::Error>(c.to_vec())).collect();
        reqwest::Body::wrap_stream(futures_util::stream::iter(parts))
    };
    let here = post(w.engine.base_url(), None, chunks()).await;
    let over_lan = post(lan.base.clone(), Some(TOKEN), chunks()).await;
    if !refused(here) || !refused(over_lan) {
        wrong.push(format!("a long body in chunks: {over_lan} over the LAN, {here} here"));
    }
    let small = post(lan.base.clone(), Some(TOKEN), json!({"focused": true}).to_string().into()).await;
    if small != 200 {
        wrong.push(format!("a short body over the LAN: {small}"));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The friend's phone row with `extra` fields (its contract, its level).
fn pair_friends_phone_with(w: &World, extra: Value) {
    let mut row = json!({
        "id": "phone-friend", "name": "Friend's phone", "secret": TOKEN, "created_at": 0,
        "kind": "phone", "account": {"id": "u-friend", "name": "Friend"},
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        row[k] = v;
    }
    std::fs::write(w.home.linggen_home().join("paired-devices.json"), json!([row]).to_string()).unwrap();
}

/// A phone's standing over plain HTTP from the LAN (doc/permission-spec.md
/// § Remote trust): one that never said its contract stands at chat; at
/// this release's contract, read — a read route passes, an edit route and
/// the raw shell are refused with its level and what they need; raised to
/// edit at this Mac, the edit route passes and the shell is still refused.
/// The level is set only at this Mac, never from the phone. `/api/health`
/// says the contract numbers.
#[tokio::test]
async fn a_phone_over_the_lan_stands_at_its_level() {
    let w = World::start(Setup {
        lan: true,
        ..Setup::default()
    })
    .await;
    let lan = Lan::new(&w);
    let beyond = |level: &str, needs: &str| json!({"error": "beyond_level", "level": level, "needs": needs});
    // Waits out the store's re-read after a rewrite of the row: asks until
    // the answer is (`is`) or is no longer (`!is`) `body`.
    let until = |path: &'static str, body: Value, is: bool| {
        let lan = &lan;
        async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let got = lan.send(reqwest::Method::GET, path, None).await;
                if (got.1 == body) == is || std::time::Instant::now() > deadline {
                    return got;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    };
    let mut wrong = Vec::new();
    let health = lan.get("/api/health").await;
    if health["contract"] != json!(1) || health["oldest_peer"] != json!(1) {
        wrong.push(format!("health does not say the contract: {health}"));
    }

    pair_friends_phone(&w);
    let old = until("/api/topic/latest?topic=t", beyond("chat", "read"), true).await;
    if old != (403, beyond("chat", "read")) {
        wrong.push(format!("a phone with no contract was not held at chat: {old:?}"));
    }

    pair_friends_phone_with(&w, json!({"contract": 1}));
    let read = until("/api/topic/latest?topic=t", beyond("chat", "read"), false).await;
    if read.0 == 403 {
        wrong.push(format!("a read route was refused at read: {read:?}"));
    }
    let edit = lan.send(reqwest::Method::POST, "/api/media/backup", Some(json!({}))).await;
    if edit != (403, beyond("read", "edit")) {
        wrong.push(format!("an edit route at read: {edit:?}"));
    }
    let shell = lan.send(reqwest::Method::POST, "/api/bash", Some(json!({"command": "true"}))).await;
    if shell != (403, beyond("read", "admin")) {
        wrong.push(format!("the raw shell at read: {shell:?}"));
    }
    let raise = json!({"permission": "edit"});
    let own_try = lan
        .send(reqwest::Method::PUT, "/api/pair/devices/phone-friend/permission", Some(raise.clone()))
        .await;
    if own_try.0 != 403 {
        wrong.push(format!("the phone raised its own level: {own_try:?}"));
    }

    let at_mac = reqwest::Client::new()
        .put(format!("{}/api/pair/devices/phone-friend/permission", w.engine.base_url()))
        .json(&raise)
        .send()
        .await
        .expect("PUT at this Mac")
        .status()
        .as_u16();
    if at_mac != 200 {
        wrong.push(format!("the level was not set at this Mac: {at_mac}"));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let raised = loop {
        let got = lan.send(reqwest::Method::POST, "/api/media/backup", Some(json!({}))).await;
        if got.1 != beyond("read", "edit") || std::time::Instant::now() > deadline {
            break got;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    if raised.1.get("error") == Some(&json!("beyond_level")) {
        wrong.push(format!("an edit route at edit: {raised:?}"));
    }
    let shell = lan.send(reqwest::Method::POST, "/api/bash", Some(json!({"command": "true"}))).await;
    if shell != (403, beyond("edit", "admin")) {
        wrong.push(format!("the raw shell at edit: {shell:?}"));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A session is its person's (doc/permission-spec.md § A paired device's
/// standing): another person's phone, even at edit, may not name the
/// builder's ordinary session — chat into it, clear it, read its pictures
/// or state — while a session it made itself is its own.
#[tokio::test]
async fn another_persons_phone_over_the_lan_may_not_name_the_builders_session() {
    use reqwest::Method;
    let w = World::start(Setup {
        lan: true,
        ..Setup::default()
    })
    .await;
    pair_friends_phone_with(&w, json!({"contract": 1, "permission": "edit"}));
    let lan = Lan::new(&w);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lan.status("/api/pair/me", Some(TOKEN)).await != 200 {
        assert!(std::time::Instant::now() < deadline, "the phone is not known");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let made = w.api.post("/api/sessions", json!({"title": "Builder's"})).await;
    let mine = made["id"].as_str().expect("the builder's session").to_string();
    let enc = |s: &str| urlencoding::encode(s).into_owned();
    let not_theirs = json!({"error": "that session is another person's"});
    let mut wrong = Vec::new();
    let mut probe = |what: &str, got: (u16, Value)| {
        if got != (403, not_theirs.clone()) {
            wrong.push(format!("{what}: {got:?}"));
        }
    };
    let chat = json!({"session_id": mine, "project_root": w.work(), "message": "FRIEND-WORDS"});
    probe("chat", lan.send(Method::POST, "/api/chat", Some(chat)).await);
    probe("clear", lan.send(Method::POST, "/api/chat/clear", Some(json!({"session_id": mine}))).await);
    let pic = format!("/api/sessions/images?session_id={}&name=x.png", enc(&mine));
    probe("pictures", lan.send(Method::GET, &pic, None).await);
    let state = format!("/api/skill-sessions/state?skill=dice&session_id={}", enc(&mine));
    probe("state", lan.send(Method::GET, &state, None).await);
    let steer = json!({"session_id": mine, "agent_id": "ling", "id": "q-1"});
    probe("remove a queued message", lan.send(Method::POST, "/api/queue/remove", Some(steer)).await);

    let own = lan.send(Method::POST, "/api/sessions", Some(json!({"title": "Friend's"}))).await;
    let theirs = own.1["id"].as_str().unwrap_or_default().to_string();
    let cleared = lan.send(Method::POST, "/api/chat/clear", Some(json!({"session_id": theirs}))).await;
    if theirs.is_empty() || cleared.0 == 403 {
        wrong.push(format!("the friend's own session: {own:?} then {cleared:?}"));
    }
    let rows = serde_json::to_string(&w.raw_rows(&mine)).unwrap();
    if rows.contains("FRIEND-WORDS") {
        wrong.push("the friend's words are in the builder's session".into());
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A turn a phone sends runs at no more than its level (doc/permission-spec.md
/// § A paired device's standing): at read its model may not write, and is
/// told so at once, never asking; her own thread and a session another
/// runtime hosts take no turn below admin.
#[tokio::test]
async fn a_phones_turn_runs_at_its_level() {
    use reqwest::Method;
    let w = World::start(Setup { lan: true, ..Setup::default() }.script(
        Agent::Ling,
        vec![tool("Write", json!({"path": "FRIEND.txt", "content": "x"})), text("Could not.")],
    ))
    .await;
    pair_friends_phone_with(&w, json!({"contract": 1}));
    let lan = Lan::new(&w);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lan.status("/api/pair/me", Some(TOKEN)).await != 200 {
        assert!(std::time::Instant::now() < deadline, "the phone is not known");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let mut wrong = Vec::new();
    let own = lan.send(Method::POST, "/api/sessions", Some(json!({"title": "Friend's"}))).await;
    let sid = own.1["id"].as_str().unwrap_or_default().to_string();
    let mark = w.mark();
    let chat = json!({"session_id": sid, "project_root": w.work(), "message": "Write FRIEND.txt"});
    let sent = lan.send(Method::POST, "/api/chat", Some(chat)).await;
    if sent.0 != 200 {
        wrong.push(format!("the friend's chat at read: {sent:?}"));
    } else if let RunEnd::Asked(ask) = w.wait_run(&sid, Agent::Ling, mark).await {
        wrong.push(format!("a turn at read asked: {ask}"));
    }
    let rows = serde_json::to_string(&w.raw_rows(&sid)).unwrap();
    if !rows.contains("Beyond this turn's level") {
        wrong.push(format!("the write was not refused as beyond the level: {rows}"));
    }
    if w.home.work().join("FRIEND.txt").exists() || w.home.linggen_home().join("workspace/FRIEND.txt").exists() {
        wrong.push("the turn at read wrote a file".into());
    }
    let her = lan.send(Method::POST, "/api/yinyue/chat", Some(json!({"text": "hi"}))).await;
    if her.0 != 403 {
        wrong.push(format!("her own thread at read: {her:?}"));
    }
    let claude = lan.send(Method::POST, "/api/sessions", Some(json!({"title": "c", "runtime": "claude_agent_sdk", "project_root": w.work()}))).await;
    match claude.1["id"].as_str() {
        Some(cid) => {
            let into = json!({"session_id": cid, "project_root": w.work(), "message": "hi"});
            let got = lan.send(Method::POST, "/api/chat", Some(into)).await;
            let beyond = json!({"error": "beyond_level", "level": "read", "needs": "admin"});
            if got != (403, beyond) {
                wrong.push(format!("a Claude session's turn at read: {got:?}"));
            }
        }
        None => wrong.push(format!("no Claude session was made to try: {claude:?}")),
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
