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

/// What the owner chose for people at this Mac (`people.json`, as the engine
/// keeps it): the Mac's fake model shared with each of `people` — a person
/// nobody shared a model with takes no turn on this Mac.
fn share_the_model_with(w: &World, people: &[&str]) {
    let rows: serde_json::Map<String, Value> = people
        .iter()
        .map(|id| (id.to_string(), json!({"models": ["fake-ling"]})))
        .collect();
    let file = json!({"version": 1, "owner": {"mine_only": []}, "people": rows});
    std::fs::write(w.home.linggen_home().join("people.json"), file.to_string()).unwrap();
}

/// A phone paired to this Mac and signed in to another person's account,
/// which the relay vouched for.
fn pair_friends_phone(w: &World) {
    let row = json!([{
        "id": "phone-friend", "name": "Friend's phone", "secret": TOKEN, "created_at": 0,
        "kind": "phone", "account": {"id": "u-friend", "name": "Friend"},
        "verified_account": "u-friend",
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
    share_the_model_with(&w, &["u-friend"]);
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
    share_the_model_with(&w, &["device:phone-signed-out"]);
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
    pair_friends_phone_with(&w, json!({"contract": 1}));
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
    let (status, _) = lan.send(reqwest::Method::GET, "/api/topic/latest?topic=phone&op=status", None).await;
    assert_eq!(status, 404, "another person's phone gets no retained reading");
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

/// The friend's phone row with `extra` fields (its contract, its level); its
/// account is vouched for by the relay unless `extra` says otherwise.
fn pair_friends_phone_with(w: &World, extra: Value) {
    let mut row = json!({
        "id": "phone-friend", "name": "Friend's phone", "secret": TOKEN, "created_at": 0,
        "kind": "phone", "account": {"id": "u-friend", "name": "Friend"},
        "verified_account": "u-friend",
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
    share_the_model_with(&w, &["u-friend"]);
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
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Write what the owner chose for each person (`people.json`) and wait out
/// the engine's re-read of the file.
async fn set_people(w: &World, people: Value) {
    let owner = people.get("owner").cloned().unwrap_or(json!({"mine_only": []}));
    let file = json!({"version": 1, "owner": owner, "people": people["people"].clone()});
    std::fs::write(w.home.linggen_home().join("people.json"), file.to_string()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
}

async fn wait_for_the_phone(lan: &Lan) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while lan.status("/api/pair/me", Some(TOKEN)).await != 200 {
        assert!(std::time::Instant::now() < deadline, "the phone is not known");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// The same app rule applies when creating a session, without changing what
/// a person with every app or the Mac's own UI can create.
#[tokio::test]
async fn creating_a_session_checks_the_named_app() {
    use reqwest::Method;
    let w = World::start(Setup {
        lan: true,
        ..Setup::default()
    })
    .await;
    pair_friends_phone_with(&w, json!({"contract": 1}));
    let lan = Lan::new(&w);
    wait_for_the_phone(&lan).await;
    let body = json!({"title": "A game", "skill": "dice"});
    let create = || lan.send(Method::POST, "/api/sessions", Some(body.clone()));
    assert_eq!(create().await.0, 200, "a person with every app");

    set_people(&w, json!({"people": {"u-friend": {"apps": []}}})).await;
    assert_eq!(
        create().await,
        (403, json!({"error": "not_your_app", "app": "dice"}))
    );
    assert_eq!(w.api.post_status("/api/sessions", body).await, 200);
}

#[tokio::test]
async fn pair_me_lists_the_persons_models() {
    let w = World::start(Setup {
        lan: true,
        ..Setup::default()
    })
    .await;
    pair_friends_phone_with(&w, json!({"settings": {"models": ["gpt-6.2-sol"]}}));
    set_people(
        &w,
        json!({"people": {"u-friend": {"models": ["qwen3.8-flash", "qwen4-max"]}}}),
    )
    .await;
    let lan = Lan::new(&w);
    wait_for_the_phone(&lan).await;
    let body = lan.get("/api/pair/me").await;
    // Only the ticked Linggen Cloud model (the starter) is theirs to use; the unticked one is not offered.
    assert_eq!(
        body["models"],
        json!([{"id": "qwen3.8-flash", "kind": "cloud"}])
    );
    assert_eq!(body["settings"]["models"], json!(["gpt-6.2-sol"]));

    // With no account, the same phone has its own person settings.
    pair_friends_phone_with(
        &w,
        json!({"account": null, "verified_account": null, "settings": {"models": ["gpt-6.2-sol"]}}),
    );
    set_people(
        &w,
        json!({"people": {"device:phone-friend": {"models": ["gpt-6-luna"]}}}),
    )
    .await;
    assert_eq!(
        lan.get("/api/pair/me").await["models"],
        json!([{"id": "gpt-6-luna", "kind": "oauth"}])
    );
}

/// A paired phone of another person is held to what the owner chose for
/// them (doc/permission-spec.md § People): with no model of this Mac shared,
/// its turn does not start — a fact, nothing kept — and shared, it runs; a
/// model it was not given is not one its turn runs on.
#[tokio::test]
async fn a_persons_turn_starts_only_on_a_model_they_were_given() {
    use reqwest::Method;
    let w = World::start(Setup { lan: true, ..Setup::default() }.script(Agent::Ling, vec![text("Hello, friend.")])).await;
    pair_friends_phone_with(&w, json!({"contract": 1}));
    let lan = Lan::new(&w);
    wait_for_the_phone(&lan).await;
    let mut wrong = Vec::new();

    let chat = json!({"project_root": w.work(), "message": "FIRST-TRY"});
    let (status, said) = lan.send(Method::POST, "/api/chat", Some(chat.clone())).await;
    if (status, said["status"].as_str()) != (200, Some("no_model")) {
        wrong.push(format!("no model shared: {status} {said}"));
    }
    if w.model.calls().iter().any(|c| c.text().contains("FIRST-TRY")) {
        wrong.push("a turn ran for a person with no model".into());
    }

    // Their session's summary is a call on this Mac's model too: refused alike.
    let (_, own) = lan.send(Method::POST, "/api/sessions", Some(json!({"title": "Friend's"}))).await;
    let sid = own["id"].as_str().unwrap_or_default().to_string();
    let compact = json!({"project_root": w.work(), "session_id": sid});
    let (_, said) = lan.send(Method::POST, "/api/chat/compact", Some(compact)).await;
    if said["status"] != "no_model" {
        wrong.push(format!("a summary for a person with no model: {said}"));
    }

    set_people(&w, json!({"people": {"u-friend": {"models": ["not-a-model-here"]}}})).await;
    let (_, said) = lan.send(Method::POST, "/api/chat", Some(chat.clone())).await;
    let ran_elsewhere = w.model.calls().iter().any(|c| c.text().contains("FIRST-TRY"));
    if ran_elsewhere {
        wrong.push(format!("a turn ran on a model that was not the person's: {said}"));
    }

    set_people(&w, json!({"people": {"u-friend": {"models": ["fake-ling"]}}})).await;
    let mark = w.mark();
    let (status, said) = lan.send(Method::POST, "/api/chat", Some(json!({"project_root": w.work(), "message": "SECOND-TRY"}))).await;
    if status != 200 || said["status"] == json!("no_model") {
        wrong.push(format!("a shared model: {status} {said}"));
    } else if let Some(sid) = said["session_id"].as_str() {
        w.finish_run(sid, Agent::Ling, mark).await;
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A person with a daily limit takes no turn once their day's tokens are
/// used: a turn on a model that reports its usage is counted, from what the
/// turn spent, against the person alone.
#[tokio::test]
async fn a_person_over_their_daily_limit_takes_no_turn() {
    use reqwest::Method;
    let (url, _) = crate::spend::host().await;
    let models = format!(
        "\n[[models]]\nid = \"stub-metered\"\nprovider = \"anthropic\"\nurl = \"{url}\"\nmodel = \"claude-sonnet-5-5\"\napi_key = \"test-not-a-key\"\ncontext_window = 200000\nsupports_tools = true\n"
    );
    let w = World::start(Setup {
        lan: true,
        config_append: Some(Box::leak(models.into_boxed_str())),
        default_model: Some("stub-metered"),
        ..Setup::default()
    })
    .await;
    set_people(&w, json!({"people": {"u-friend": {"models": ["stub-metered"], "daily_tokens": 5000}}})).await;
    pair_friends_phone_with(&w, json!({"contract": 1}));
    let lan = Lan::new(&w);
    wait_for_the_phone(&lan).await;
    let mark = w.mark();
    let (status, said) = lan.send(Method::POST, "/api/chat", Some(json!({"project_root": w.work(), "message": "UNDER"}))).await;
    assert_eq!((status, said["status"].as_str()), (200, Some("started")), "{said}");
    w.finish_run(said["session_id"].as_str().unwrap(), Agent::Ling, mark).await;
    // The count is made as the turn ends: look for it a moment.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut used = w.api.get("/api/people").await;
    while used["people"][0]["used_today"].as_i64().unwrap_or(0) == 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        used = w.api.get("/api/people").await;
    }
    // What the stand-in host reports for one call: 1,200 + 3,000 + 5,000 in, 400 out.
    assert_eq!(used["people"][0]["used_today"], 9_600, "{used}");
    let (_, said) = lan.send(Method::POST, "/api/chat", Some(json!({"project_root": w.work(), "message": "OVER"}))).await;
    assert_eq!(said["status"], "over_limit", "{said}");
    set_people(&w, json!({"people": {"u-friend": {"models": ["stub-metered"], "daily_tokens": 20000}}})).await;
    let mark = w.mark();
    let (_, said) = lan.send(Method::POST, "/api/chat", Some(json!({"project_root": w.work(), "message": "AGAIN"}))).await;
    assert_eq!(said["status"], "started", "a higher limit lets the turn start: {said}");
    w.finish_run(said["session_id"].as_str().unwrap(), Agent::Ling, mark).await;
}

const TASK_MODEL: &str = r#"
[[models]]
id = "task-metered"
provider = "anthropic"
url = "{{MODEL_URL}}"
model = "task-metered-wire"
api_key = "test-not-a-key"
context_window = 32000
supports_tools = true

[[models]]
id = "task-alternate"
provider = "anthropic"
url = "{{MODEL_URL}}"
model = "task-alternate-wire"
api_key = "test-not-a-key"
context_window = 32000
supports_tools = true
"#;

async fn task_end_world(cold: bool, after: Vec<crate::support::model::Reply>) -> (World, Lan, String) {
    let mut script = vec![
        tool("Bash", json!({"cmd": "sleep 5; echo TASK-RESULT", "run_in_background": true})),
        text("Background task started."),
    ];
    script.extend(after);
    let setup = Setup {
        lan: true,
        config_append: Some(TASK_MODEL),
        env: if cold { vec![("LINGGEN_TEST_COLD_TASK_END", "1".into())] } else { vec![] },
        ..Setup::default()
    };
    let w = World::start(setup.script(Agent::Ling, script)).await;
    crate::app_index::install_jukebox(&w, "jukebox").await;
    pair_friends_phone_with(&w, json!({"contract": 1}));
    let lan = Lan::new(&w);
    wait_for_the_phone(&lan).await;
    set_task_person(&w, json!({"level": "edit", "apps": ["jukebox"], "models": ["task-metered"]})).await;
    let session = lan.post("/api/sessions", json!({"title": "Background work", "project_root": w.work()})).await;
    let sid = session["id"].as_str().unwrap().to_string();
    w.api.patch("/api/sessions/permission", json!({"session_id": sid, "path": "*", "mode": "edit"})).await;
    let mark = w.mark();
    assert_eq!(task_chat(&w, &lan, &sid, "Start the background task.").await["status"], "started");
    w.finish_run(&sid, Agent::Ling, mark).await;
    assert!(task_rows(&w, &sid).contains("Started in the background as task"), "{}", task_rows(&w, &sid));
    (w, lan, sid)
}

async fn set_task_person(w: &World, patch: Value) {
    let (status, body) = w.api.put_json("/api/people/u-friend", patch).await;
    assert_eq!(status, 200, "{body}");
}

async fn task_chat(w: &World, lan: &Lan, sid: &str, message: &str) -> Value {
    lan.post("/api/chat", json!({"session_id": sid, "project_root": w.work(), "message": message})).await
}

fn task_rows(w: &World, sid: &str) -> String {
    serde_json::to_string(&w.raw_rows(sid)).unwrap()
}

async fn changed_task_action(change: Value, action: crate::support::model::Reply, denial: &str) {
    let mut wrong = Vec::new();
    for cold in [false, true] {
        let (w, lan, sid) = task_end_world(cold, vec![action.clone(), text("Task end checked."), action.clone(), text("Typed check done.")]).await;
        let mark = w.mark();
        set_task_person(&w, change.clone()).await;
        w.finish_run(&sid, Agent::Ling, mark).await;
        let end = w.raw_rows(&sid);
        let end_text = serde_json::to_string(&end).unwrap();
        assert!(end_text.contains("Last output:\\nTASK-RESULT"), "{end_text}");
        if !end_text.contains(denial) {
            wrong.push(format!("cold={cold}: task-end action was not refused with {denial}"));
        }
        let mark = w.mark();
        assert_eq!(task_chat(&w, &lan, &sid, "Check the same action.").await["status"], "started");
        w.finish_run(&sid, Agent::Ling, mark).await;
        let rows = w.raw_rows(&sid);
        let typed = serde_json::to_string(&rows[end.len()..]).unwrap();
        assert!(typed.contains(denial), "typed action must be refused: {typed}");
        w.assert_all_scripted();
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[tokio::test]
async fn a_task_end_uses_the_persons_lowered_level() {
    changed_task_action(
        json!({"level": "read"}),
        tool("Write", json!({"path": "TASK-WRITE.txt", "content": "fixture"})),
        "Beyond this turn's level",
    ).await;
}

#[tokio::test]
async fn a_task_end_cannot_reach_an_app_taken_away() {
    changed_task_action(
        json!({"apps": []}),
        tool("AppTool", json!({"app": "jukebox", "tool": "List", "args": {"what": "jazz"}})),
        "is not one of the apps you may use",
    ).await;
}

async fn refused_task_end(change: Value, status: &str) {
    let mut wrong = Vec::new();
    for cold in [false, true] {
        let (w, lan, sid) = task_end_world(cold, vec![text("Unexpected task-end reply.")]).await;
        let calls = w.model.calls_of(Agent::Ling).len();
        set_task_person(&w, change.clone()).await;
        assert_eq!(task_chat(&w, &lan, &sid, "Cannot start a turn.").await["status"], status);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let rows = loop {
            let rows = task_rows(&w, &sid);
            if rows.contains(status) || w.model.calls_of(Agent::Ling).len() > calls {
                break rows;
            }
            assert!(std::time::Instant::now() < deadline, "no task result: {rows}");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert!(rows.contains("Last output:\\nTASK-RESULT"), "{rows}");
        if !rows.contains(status) || w.model.calls_of(Agent::Ling).len() != calls {
            wrong.push(format!("cold={cold}: expected {status} without an extra model call"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[tokio::test]
async fn a_task_result_is_kept_after_all_the_persons_models_are_taken_away() {
    refused_task_end(json!({"models": []}), "no_model").await;
}

#[tokio::test]
async fn a_task_result_is_kept_when_the_person_is_over_the_daily_limit() {
    refused_task_end(json!({"daily_tokens": 1}), "over_limit").await;
}

#[tokio::test]
async fn a_task_end_switches_models_and_counts_use_to_its_person() {
    let mut wrong = Vec::new();
    for cold in [false, true] {
        let (w, lan, sid) = task_end_world(cold, vec![text("Task end answered."), text("Typed answer.")]).await;
        let used = || async { w.api.get("/api/people").await["people"][0]["used_today"].as_u64().unwrap() };
        let before = used().await;
        assert!(before > 0, "the initiating typed turn must report usage");
        let mark = w.mark();
        set_task_person(&w, json!({"models": ["task-alternate"]})).await;
        w.finish_run(&sid, Agent::Ling, mark).await;
        let end_model = w.model.calls_of(Agent::Ling).last().unwrap().model().to_string();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while used().await == before && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let after_end = used().await;
        if after_end <= before {
            wrong.push(format!("cold={cold}: no usage charged for task end"));
        }
        let mark = w.mark();
        assert_eq!(task_chat(&w, &lan, &sid, "Check the current model.").await["status"], "started");
        w.finish_run(&sid, Agent::Ling, mark).await;
        let calls = w.model.calls_of(Agent::Ling);
        let typed_model = calls.last().unwrap().model();
        if end_model != typed_model {
            wrong.push(format!("cold={cold}: task model {end_model} differs from typed {typed_model}"));
        }
        assert_eq!(typed_model, "task-alternate-wire");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while used().await == after_end && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(used().await > after_end, "typed turn must count for the same person");
        w.assert_all_scripted();
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// An app is for a person only when they may use it — by its page files, by
/// the list of apps they are shown — and an app the owner keeps is the
/// owner's alone, whatever a person's own list says.
#[tokio::test]
async fn a_person_reaches_and_is_shown_only_the_apps_they_may_use() {
    use reqwest::Method;
    let w = World::start(Setup { lan: true, ..Setup::default() }).await;
    pair_friends_phone_with(&w, json!({"contract": 1}));
    let lan = Lan::new(&w);
    wait_for_the_phone(&lan).await;
    let names = |list: &Value| -> Vec<String> {
        list.as_array().map(|a| a.iter().filter_map(|s| s["name"].as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    let mut wrong = Vec::new();

    // Nobody set anything: every app, in the page and in the list.
    let (status, page) = lan.send(Method::GET, "/apps/dice/index.html", None).await;
    if status != 200 {
        wrong.push(format!("an app's page, by default: {status} {page}"));
    }
    let (_, list) = lan.send(Method::GET, "/api/skills", None).await;
    if !names(&list).contains(&"dice".to_string()) {
        wrong.push(format!("the default list lacks dice: {:?}", names(&list)));
    }

    // Their own list: dice is not on it.
    set_people(&w, json!({"people": {"u-friend": {"apps": ["abacus"]}}})).await;
    let (status, refused) = lan.send(Method::GET, "/apps/dice/index.html", None).await;
    if (status, refused["error"].as_str()) != (403, Some("not_your_app")) {
        wrong.push(format!("a page of an app off their list: {status} {refused}"));
    }
    let (_, list) = lan.send(Method::GET, "/api/skills", None).await;
    if names(&list) != ["abacus"] {
        wrong.push(format!("the list they are shown: {:?}", names(&list)));
    }
    let (status, tool) = lan.send(Method::POST, "/api/skills/dice/tools/Roll", Some(json!({}))).await;
    if (status, tool["error"].as_str()) != (403, Some("not_your_app")) {
        wrong.push(format!("a tool of an app off their list: {status} {tool}"));
    }
    let owners = w.api.get("/api/skills").await;
    if !names(&owners).contains(&"dice".to_string()) {
        wrong.push("the owner's list lost an app".into());
    }

    // The owner keeps dice: no list of theirs gives it.
    set_people(&w, json!({"owner": {"mine_only": ["dice"]}, "people": {"u-friend": {"apps": ["dice", "abacus"]}}})).await;
    let (status, refused) = lan.send(Method::GET, "/apps/dice/index.html", None).await;
    if (status, refused["error"].as_str()) != (403, Some("not_your_app")) {
        wrong.push(format!("an app the owner keeps: {status} {refused}"));
    }
    let (status, _) = lan.send(Method::GET, "/apps/abacus/index.html", None).await;
    if status == 403 {
        wrong.push("an app on their list was refused".into());
    }
    let (status, _) = w.api.call("GET", "/apps/dice/index.html").await;
    if status != 200 {
        wrong.push(format!("the owner's own page: {status}"));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The household is set only at this Mac: the owner's own page changes a
/// person's settings, a paired phone — whoever it is — cannot, and a bad
/// setting changes nothing.
#[tokio::test]
async fn the_household_is_set_only_at_this_mac() {
    use reqwest::Method;
    let w = World::start(Setup { lan: true, ..Setup::default() }).await;
    pair_friends_phone_with(&w, json!({"contract": 1}));
    let lan = Lan::new(&w);
    wait_for_the_phone(&lan).await;
    let body = json!({"level": "edit", "models": ["fake-ling"], "daily_tokens": 5});
    let (status, refused) = lan.send(Method::PUT, "/api/people/u-friend", Some(body.clone())).await;
    assert_eq!(status, 403, "{refused}");
    assert_eq!(lan.send(Method::GET, "/api/people", None).await.0, 403);
    let (status, set) = w.api.put_json("/api/people/u-friend", body).await;
    assert_eq!(status, 200, "{set}");
    let listed = w.api.get("/api/people").await;
    assert_eq!(listed["people"][0]["id"], "u-friend", "{listed}");
    assert_eq!(listed["people"][0]["level"], "edit");
    assert_eq!(listed["people"][0]["settings"]["daily_tokens"], 5);
    let (status, bad) = w.api.put_json("/api/people/u-friend", json!({"level": "admin"})).await;
    assert_eq!(status, 400, "{bad}");
    assert_eq!(w.api.get("/api/people").await["people"][0]["level"], "edit", "a bad setting left a trace");
}

/// A phone is the account the relay vouched for and nothing more: the same
/// row, with what the owner chose for that account, stands at it only once
/// vouched; until then it is its device, at read.
#[tokio::test]
async fn a_phone_is_the_account_the_relay_vouched_for_and_nothing_more() {
    let w = World::start(Setup { lan: true, ..Setup::default() }).await;
    set_people(&w, json!({"people": {"u-friend": {"level": "edit"}}})).await;
    let lan = Lan::new(&w);
    let beyond = json!({"error": "beyond_level", "level": "read", "needs": "edit"});
    async fn at_backup(lan: &Lan) -> (u16, Value) {
        lan.send(reqwest::Method::POST, "/api/media/backup", Some(json!({}))).await
    }
    let mut wrong = Vec::new();

    pair_friends_phone_with(&w, json!({"contract": 1, "verified_account": null}));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let unvouched = loop {
        let got = at_backup(&lan).await;
        if got.0 == 403 || std::time::Instant::now() > deadline {
            break got;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    if unvouched != (403, beyond.clone()) {
        wrong.push(format!("a claim the relay did not vouch for stood at the account's level: {unvouched:?}"));
    }

    pair_friends_phone_with(&w, json!({"contract": 1}));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let vouched = loop {
        let got = at_backup(&lan).await;
        if got.1 != beyond || std::time::Instant::now() > deadline {
            break got;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    if vouched.1.get("error") == Some(&json!("beyond_level")) {
        wrong.push(format!("a vouched account did not stand at its level: {vouched:?}"));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Another person's turn reaches an app's action through `AppTool` only
/// when the owner gave them the app: one off their list is refused with the
/// reason. One on it is still refused under their turn's ceiling — an app
/// lives in Linggen's own folder, which is the owner's alone to any turn a
/// ceiling holds, whatever the app's own grant (`permission::owners_alone`);
/// the rule that was there first wins, and nothing runs.
#[tokio::test]
async fn a_persons_turn_reaches_an_app_through_apptool_only_as_given_and_under_their_ceiling() {
    use reqwest::Method;
    let call = |tool_name: &str| tool("AppTool", json!({"app": "jukebox", "tool": tool_name, "args": {"what": "jazz"}}));
    let setup = Setup { lan: true, ..Setup::default() }.script(
        Agent::Ling,
        vec![call("List"), text("Closed."), call("List"), call("Play"), text("Done.")],
    );
    let w = World::start(setup).await;
    crate::app_index::install_jukebox(&w, "jukebox").await;
    pair_friends_phone_with(&w, json!({"contract": 1}));
    let lan = Lan::new(&w);
    wait_for_the_phone(&lan).await;
    let turn = |apps: Value| {
        let (w, lan) = (&w, &lan);
        async move {
            let people = json!({"people": {"u-friend": {"apps": apps, "models": ["fake-ling"]}}});
            set_people(w, people).await;
            let own = lan.send(Method::POST, "/api/sessions", Some(json!({"title": "Friend's"}))).await;
            let sid = own.1["id"].as_str().unwrap_or_default().to_string();
            let mark = w.mark();
            let chat = json!({"session_id": sid, "project_root": w.work(), "message": "Play some jazz."});
            let sent = lan.send(Method::POST, "/api/chat", Some(chat)).await;
            assert_eq!(sent.0, 200, "{sent:?}");
            if let RunEnd::Asked(ask) = w.wait_run(&sid, Agent::Ling, mark).await {
                panic!("a person's turn asked: {ask}");
            }
            serde_json::to_string(&w.raw_rows(&sid)).unwrap()
        }
    };
    let closed = turn(json!(["abacus"])).await;
    assert!(closed.contains("is not one of the apps you may use"), "{closed}");
    let given = turn(json!(["jukebox"])).await;
    assert!(!given.contains("is not one of the apps you may use"), "the app was given: {given}");
    assert_eq!(given.matches("Beyond this turn's level").count(), 2, "both calls stop at the ceiling: {given}");
    for ran in ["listing", "playing"] {
        assert!(!closed.contains(ran) && !given.contains(ran), "an action ran: {ran}");
    }
    w.assert_all_scripted();
}
