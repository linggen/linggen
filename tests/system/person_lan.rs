//! A day is its own person's over plain HTTP from the LAN too
//! (doc/one-ling-spec.md § One session a day): the LAN gate resolves the
//! person from the paired device's token, as the channel does, and sends
//! the request as theirs whatever `user_id` it named.

use crate::day_session::{day_world_at, say};
use crate::support::model::Agent;
use crate::support::{text, Setup, World};
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
/// a page's history of it, and a chat naming it — whatever `user_id` the
/// request names; nor is it given the builder's notes.
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
    let mark = w.mark();
    let body = json!({"session_id": mine, "project_root": w.work(), "message": "FRIEND-WORDS", "skill_name": "dice"});
    let sent = lan.post("/api/chat", body).await;
    let landed = sent["session_id"].as_str().unwrap_or_default().to_string();
    if landed == mine {
        wrong.push(format!("a chat naming the builder's day landed in it: {sent}"));
    } else if !landed.is_empty() {
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
