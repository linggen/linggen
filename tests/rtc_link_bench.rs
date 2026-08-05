//! Virtual-time WebRTC link bench — measures data-channel throughput through
//! str0m + sctp-proto under controlled RTT and loss, entirely in-process.
//!
//! Why this exists: the first real off-LAN sync crawled at ~70 KB/s even
//! after the send buffer was kept full, which points at SCTP congestion
//! behavior rather than our queueing. str0m is Sans-IO, so both peers can be
//! driven on a fake clock with a simulated link — deterministic, root-free,
//! and a 10 MB transfer at 100 ms RTT finishes in milliseconds of real time.
//!
//! Run: cargo test --test rtc_link_bench -- --ignored --nocapture

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig};

const CHUNK: usize = 16 * 1024; // production DOWNLOAD_CHUNK
const TRANSFER: usize = 5 * 1024 * 1024;

/// One direction of the link: packets in flight, delivered when due.
struct Pipe {
    queue: VecDeque<(Instant, Vec<u8>)>,
    latency: Duration,
    loss_permille: u32,
    rng: u64,
    dropped: usize,
    carried: usize,
}

impl Pipe {
    fn new(latency: Duration, loss_permille: u32) -> Self {
        Pipe {
            queue: VecDeque::new(),
            latency,
            loss_permille,
            rng: 0x2545_F491_4F6C_DD1D,
            dropped: 0,
            carried: 0,
        }
    }

    /// Deterministic xorshift — the whole bench replays identically.
    fn roll(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % 1000) as u32
    }

    fn send(&mut self, now: Instant, payload: Vec<u8>) {
        if self.roll() < self.loss_permille {
            self.dropped += 1;
            return;
        }
        self.carried += 1;
        self.queue.push_back((now + self.latency, payload));
    }

    fn next_due(&self) -> Option<Instant> {
        self.queue.front().map(|(t, _)| *t)
    }

    fn take_due(&mut self, now: Instant) -> Option<Vec<u8>> {
        if self.queue.front().is_some_and(|(t, _)| *t <= now) {
            return self.queue.pop_front().map(|(_, p)| p);
        }
        None
    }
}

/// Everything the bench observes while draining a peer's outputs.
#[derive(Default)]
struct Observed {
    connected: bool,
    server_channel: Option<ChannelId>,
    received: usize,
    first_byte: Option<Instant>,
}

/// Poll one peer until it reports a timeout, forwarding transmits into its
/// outbound pipe. Sans-IO contract: Timeout is only returned once transmits
/// and events are exhausted, so this never spins.
fn drain(
    rtc: &mut Rtc,
    is_client: bool,
    out_pipe: &mut Pipe,
    now: Instant,
    obs: &mut Observed,
) -> Instant {
    loop {
        match rtc.poll_output().unwrap() {
            Output::Timeout(t) => return t,
            Output::Transmit(t) => out_pipe.send(now, t.contents.to_vec()),
            Output::Event(e) => match e {
                Event::IceConnectionStateChange(s) => {
                    if s == IceConnectionState::Completed {
                        obs.connected = true;
                    }
                }
                Event::ChannelOpen(id, _) if !is_client => {
                    obs.server_channel = Some(id);
                }
                Event::ChannelData(d) if is_client => {
                    obs.first_byte.get_or_insert(now);
                    obs.received += d.data.len();
                }
                _ => {}
            },
        }
    }
}

fn drive(rtt: Duration, loss_permille: u32) -> (f64, usize, usize, f64) {
    let start = Instant::now();
    let mut now = start;

    let client_addr: SocketAddr = "1.1.1.1:1000".parse().unwrap();
    let server_addr: SocketAddr = "2.2.2.2:2000".parse().unwrap();

    // Client plays the phone (offerer). Server mirrors production's
    // relay-peer config: full ICE, host candidate only.
    let mut client = RtcConfig::new().build(now);
    let mut server = RtcConfig::new().build(now);
    client
        .add_local_candidate(Candidate::host(client_addr, "udp").unwrap())
        .unwrap();
    server
        .add_local_candidate(Candidate::host(server_addr, "udp").unwrap())
        .unwrap();

    let mut change = client.sdp_api();
    let _cid = change.add_channel("media".into());
    let (offer, pending) = change.apply().unwrap();
    let answer = server.sdp_api().accept_offer(offer).unwrap();
    client.sdp_api().accept_answer(pending, answer).unwrap();

    let mut a_to_b = Pipe::new(rtt / 2, loss_permille); // client → server
    let mut b_to_a = Pipe::new(rtt / 2, loss_permille); // server → client

    let mut obs = Observed::default();
    let mut to_send: usize = TRANSFER;
    let mut stalls: usize = 0;
    let mut stalled = Duration::ZERO;

    let deadline = now + Duration::from_secs(600); // virtual watchdog

    while obs.received < TRANSFER && now < deadline {
        // Outputs first — a write below may need the buffer they free.
        let t_client = drain(&mut client, true, &mut a_to_b, now, &mut obs);
        let t_server = drain(&mut server, false, &mut b_to_a, now, &mut obs);

        // The production write pattern: each wake, fill until write refuses.
        let mut wrote = false;
        if let Some(id) = obs.server_channel {
            if let Some(mut ch) = server.channel(id) {
                while to_send > 0 {
                    let n = CHUNK.min(to_send);
                    match ch.write(true, &vec![0u8; n]) {
                        Ok(true) => {
                            to_send -= n;
                            wrote = true;
                        }
                        _ => break,
                    }
                }
            }
        }
        if wrote {
            // The writes queued packets — ship them at this same instant.
            let _ = drain(&mut server, false, &mut b_to_a, now, &mut obs);
        }

        // Advance virtual time to the next thing that can happen.
        let mut next = t_client.min(t_server);
        for t in [a_to_b.next_due(), b_to_a.next_due()].into_iter().flatten() {
            next = next.min(t);
        }
        let prev = now;
        now = next.max(now + Duration::from_micros(10));
        // A jump well past the RTT during the data phase is a retransmission
        // timer firing — the stall signature this bench exists to count.
        if obs.first_byte.is_some() && now - prev > rtt * 3 {
            stalls += 1;
            stalled += now - prev;
        }

        // Deliver due packets one at a time, draining the receiver between
        // deliveries — DTLS keeps a small inbound queue.
        loop {
            let mut delivered = false;
            if let Some(p) = a_to_b.take_due(now) {
                let recv = Receive {
                    proto: Protocol::Udp,
                    source: client_addr,
                    destination: server_addr,
                    contents: p.as_slice().try_into().unwrap(),
                };
                server.handle_input(Input::Receive(now, recv)).unwrap();
                let _ = drain(&mut server, false, &mut b_to_a, now, &mut obs);
                delivered = true;
            }
            if let Some(p) = b_to_a.take_due(now) {
                let recv = Receive {
                    proto: Protocol::Udp,
                    source: server_addr,
                    destination: client_addr,
                    contents: p.as_slice().try_into().unwrap(),
                };
                client.handle_input(Input::Receive(now, recv)).unwrap();
                let _ = drain(&mut client, true, &mut a_to_b, now, &mut obs);
                delivered = true;
            }
            if !delivered {
                break;
            }
        }
        client.handle_input(Input::Timeout(now)).unwrap();
        server.handle_input(Input::Timeout(now)).unwrap();
    }

    assert!(obs.connected, "ICE never completed (rtt={rtt:?})");
    assert!(
        obs.received >= TRANSFER,
        "transfer incomplete: {}/{TRANSFER} (rtt={rtt:?} loss={loss_permille}‰)",
        obs.received
    );

    let elapsed = now - obs.first_byte.unwrap_or(start);
    let kbs = (obs.received as f64 / 1024.0) / elapsed.as_secs_f64();
    (
        kbs,
        a_to_b.dropped + b_to_a.dropped,
        stalls,
        stalled.as_secs_f64() / elapsed.as_secs_f64(),
    )
}

#[test]
#[ignore = "bench — run explicitly with --ignored --nocapture"]
fn throughput_matrix() {
    println!(
        "\n{:>8} {:>7} {:>12} {:>9} {:>9} {:>10}",
        "RTT", "loss‰", "KB/s", "dropped", "stalls", "stalled%"
    );
    for (rtt_ms, loss) in [(2u64, 0u32), (100, 0), (100, 10), (100, 30), (200, 10)] {
        let (kbs, dropped, stalls, stalled) = drive(Duration::from_millis(rtt_ms), loss);
        println!(
            "{rtt_ms:>6}ms {loss:>7} {kbs:>12.0} {dropped:>9} {stalls:>9} {:>9.0}%",
            stalled * 100.0
        );
    }
}
