//! The phone's catalog on subscription (retained read) and on live topic updates.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use str0m::change::SdpAnswer;
use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};
use tokio::net::UdpSocket;

use crate::support::{Setup, World};

const LATEST: &str = "/api/topic/latest?topic=mac&op=tools";

struct Peer {
    rtc: Rtc,
    udp: UdpSocket,
    control: ChannelId,
}

impl Peer {
    async fn connect(base: &str, token: Option<&str>) -> Self {
        let http = reqwest::Client::new();
        let device = |req: reqwest::RequestBuilder| match token {
            Some(token) => req.header("x-linggen-device", token),
            None => req,
        };
        let grant: Value = device(http.get(format!("{base}/api/rtc/token")))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut rtc = Rtc::new(Instant::now());
        rtc.add_local_candidate(Candidate::host(udp.local_addr().unwrap(), "udp").unwrap())
            .unwrap();
        let mut sdp = rtc.sdp_api();
        let control = sdp.add_channel("control".into());
        let (offer, pending) = sdp.apply().unwrap();
        let answer = device(http.post(format!("{base}/api/rtc/whip")))
            .bearer_auth(grant["token"].as_str().unwrap())
            .body(offer.to_sdp_string())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        rtc.sdp_api()
            .accept_answer(pending, SdpAnswer::from_sdp_string(&answer).unwrap())
            .unwrap();
        let mut peer = Self { rtc, udp, control };
        tokio::time::timeout(
            Duration::from_secs(10),
            peer.until(|e| {
                matches!(e, Event::ChannelOpen(_, label) if label == "control").then_some(())
            }),
        )
        .await
        .expect("control channel opened");
        peer
    }

    async fn until<T>(&mut self, mut read: impl FnMut(Event) -> Option<T>) -> T {
        let mut buf = [0; 65536];
        loop {
            match self.rtc.poll_output().unwrap() {
                Output::Transmit(t) => {
                    self.udp.send_to(&t.contents, t.destination).await.unwrap();
                }
                Output::Timeout(at) => tokio::select! {
                    packet = self.udp.recv_from(&mut buf) => {
                        let (n, source) = packet.unwrap();
                        let input = Receive::new(Protocol::Udp, source, self.udp.local_addr().unwrap(), &buf[..n]).unwrap();
                        self.rtc.handle_input(Input::Receive(Instant::now(), input)).unwrap();
                    }
                    _ = tokio::time::sleep(at.saturating_duration_since(Instant::now())) => {
                        self.rtc.handle_input(Input::Timeout(Instant::now())).unwrap();
                    }
                },
                Output::Event(e) => {
                    if let Some(value) = read(e) {
                        return value;
                    }
                }
            }
        }
    }

    async fn message(&mut self, matching: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(
            Duration::from_secs(10),
            self.until(|e| {
                let Event::ChannelData(data) = e else {
                    return None;
                };
                let value = serde_json::from_slice(&data.data).ok()?;
                matching(&value).then_some(value)
            }),
        )
        .await
        .expect("matching control message")
    }

    async fn replay(&mut self) -> Value {
        let msg =
            json!({"type":"http_request", "request_id":"latest", "method":"GET", "url":LATEST});
        assert!(self
            .rtc
            .channel(self.control)
            .unwrap()
            .write(false, msg.to_string().as_bytes())
            .unwrap());
        let reply = self.message(|msg| msg["request_id"] == "latest").await;
        assert_eq!(reply["data"]["status"], 200, "{reply}");
        serde_json::from_str::<Value>(reply["data"]["body"].as_str().unwrap()).unwrap()["payload"]
            .clone()
    }

    async fn live(&mut self, matching: impl Fn(&Value) -> bool) -> Value {
        self.message(|msg| {
            msg["kind"] == "device_topic"
                && msg["data"]["topic"] == "mac"
                && msg["data"]["op"] == "tools"
                && matching(&msg["data"]["payload"])
        })
        .await["data"]["payload"]
            .clone()
    }
}

fn lan_base(w: &World) -> String {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    socket.connect("192.0.2.1:9").unwrap(); // Pick the LAN address without sending.
    format!(
        "http://{}:{}",
        socket.local_addr().unwrap().ip(),
        w.engine.port
    )
}

async fn world() -> World {
    World::start_with_home(
        Setup {
            lan: true,
            daily_session: Some(true),
            ..Setup::default()
        },
        |home| {
            let root = home.linggen_home();
            let devices = [
                ("given", Some("u-given")),
                ("not-given", Some("u-not-given")),
                ("owner", Some("u-owner")),
                ("signed-out", None),
            ];
            let rows: Vec<_> = devices.into_iter().map(|(id, account)| json!({
            "id": id, "name": id, "secret": format!("token-{id}"), "created_at": 0,
            "kind": "phone", "contract": 1, "account": account.map(|id| json!({"id": id})),
            "verified_account": account,
        })).collect();
            std::fs::write(root.join("paired-devices.json"), json!(rows).to_string()).unwrap();
            let people = json!({
                "version": 1,
                "owner": {"mine_only": ["private"]},
                "people": {"u-given": {"apps": ["music"]}, "u-not-given": {"apps": []}},
            });
            std::fs::write(root.join("people.json"), people.to_string()).unwrap();
            std::fs::write(
                root.join("account.toml"),
                "api_token = ''\nuser_id = 'u-owner'\n",
            )
            .unwrap();
        },
    )
    .await
}

async fn publish(w: &World, marker: &str) -> Value {
    let tool = |app: &str| json!({"app": app, "name":"List", "wire_name":format!("{app}_list"), "description":"Lists.", "params":{}, "tier":"read"});
    let payload =
        json!({"tools":[tool("music"), tool("private")], "host": marker, "published_at":"fixture"});
    let (status, result) = w
        .api
        .post_with_status(
            "/api/topic/publish",
            json!({
                "topic":"mac", "op":"tools", "retain":true, "payload":payload,
            }),
        )
        .await;
    assert_eq!(status, 200, "{result}");
    payload
}

async fn catalogs(live: bool) {
    let w = world().await;
    // Ensure the startup publisher has retained its first value before we publish ours.
    tokio::time::timeout(Duration::from_secs(10), async {
        while w.api.call("GET", LATEST).await.0 != 200 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("startup catalog");
    let lan = lan_base(&w);
    let local = w.engine.base_url();
    let mut peers = Vec::new();
    for token in [
        Some("token-owner"),
        Some("token-given"),
        Some("token-not-given"),
        Some("token-signed-out"),
        None,
    ] {
        let base = if token.is_some() { &lan } else { &local };
        peers.push(Peer::connect(base, token).await);
    }
    let marker = "catalog-test";
    let full = publish(&w, marker).await;
    let mut music = full.clone();
    music["tools"] = json!([full["tools"][0]]);
    let mut empty = full.clone();
    empty["tools"] = json!([]);
    let mut observed = Vec::new();
    for peer in &mut peers {
        observed.push(if live {
            peer.live(|payload| payload["host"] == marker).await
        } else {
            peer.replay().await
        });
    }
    assert_eq!(observed, [full.clone(), music.clone(), empty, music, full]);

    if live {
        let given = &mut peers[1];
        let tools = "  - name: List\n    description: Lists.\n    cmd: echo ok\n    tier: read\n    remote: true\n";
        crate::app_index::install_app(&w, "music", "Music tools.", tools).await;
        given
            .live(|payload| payload["host"] != marker && payload["tools"][0]["app"] == "music")
            .await;
        assert_eq!(
            w.api
                .put_json("/api/people/u-given", json!({"apps":[]}))
                .await
                .0,
            200
        );
        let updated = given.live(|payload| payload["tools"] == json!([])).await;
        assert_eq!(given.replay().await, updated);
    }
}

#[tokio::test]
async fn mac_tools_replay_is_for_each_peers_person() {
    catalogs(false).await;
}

#[tokio::test]
async fn mac_tools_live_publish_is_for_each_peers_person() {
    catalogs(true).await;
}
