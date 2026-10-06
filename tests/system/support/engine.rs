//! The engine under test: the built `ling` binary as its own process, on a
//! random port, over the world's home — the same `--web` server a person
//! runs, nothing in-process.

use super::guard;
use super::home::Home;
use super::process::{free_port, hermetic_env, start_server, Owned};
use serde_json::Value;
use std::process::Command;
use std::time::{Duration, Instant};

pub struct Engine {
    pub port: u16,
    proc: Owned,
}

/// How a run the test waited on ended.
#[derive(Debug)]
pub enum RunEnd {
    /// `run/finish` logged.
    Finished,
    /// The agent is parked on a question (a permission prompt or AskUser).
    Asked(Value),
}

/// Where the engine log stood before an action (its length in bytes); lines
/// after it are the action's.
#[derive(Clone, Copy, Debug)]
pub struct Mark(usize);

impl Engine {
    /// Start `ling --web` on `port` (the one the config names; a fresh one
    /// if it was taken meanwhile, and the config rewritten to name it).
    /// `mem_url` is the world's ling-mem (`LING_MEM_URL`). With no config
    /// (`has_config` false) the flag alone names the port, and the env alone
    /// names ling-mem — a first install.
    pub async fn start(home: &Home, port: u16, mem_url: &str, has_config: bool) -> Self {
        let env = hermetic_env(home, Some(mem_url));
        let spawn = |port: u16| {
            if has_config {
                set_config_port(home, port);
            }
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_ling"));
            cmd.args(["--web", "--port", &port.to_string(), "--root"])
                .arg(home.work());
            Owned::spawn(cmd, &env, &home.work(), home.root.join("engine.log"))
        };
        let server = start_server(Some(port), "/api/health", spawn)
            .await
            .unwrap_or_else(|e| panic!("engine: {e}"));
        guard::engine_banner(&server.proc.log_text(), &home.root, has_config);
        if !has_config {
            guard::no_config(home);
        }
        Self {
            port: server.port,
            proc: server.proc,
        }
    }

    pub fn new_port() -> u16 {
        free_port()
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn log(&self) -> String {
        self.proc.log_text()
    }

    pub fn mark(&self) -> Mark {
        Mark(self.proc.log_len())
    }

    /// The log written since `mark`.
    pub fn log_since(&self, mark: Mark) -> String {
        self.proc.log_from(mark.0)
    }

    /// Wait for `agent`'s run in `sid` (begun after `mark`) to finish, or to
    /// park on a question; `pending` reads the open questions.
    /// `sid: None` waits for the agent's run in any session.
    pub async fn wait_run<F, Fut>(
        &self,
        sid: Option<&str>,
        agent: &str,
        mark: Mark,
        pending: F,
    ) -> RunEnd
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Vec<Value>>,
    {
        let start = Instant::now();
        let in_session = |l: &str| sid.is_none_or(|s| names(l, "session_id", s));
        let finish =
            |l: &str| l.contains("run/finish") && names(l, "agent_id", agent) && in_session(l);
        let sid = sid.unwrap_or("any session");
        while start.elapsed() < Duration::from_secs(30) {
            if self.log_since(mark).lines().any(finish) {
                return RunEnd::Finished;
            }
            let asks = pending().await;
            if let Some(ask) = asks.into_iter().next() {
                return RunEnd::Asked(ask);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!(
            "{agent}'s run in {sid} did not finish in 30s; log since:\n{}",
            tail(&self.log_since(mark), 40)
        );
    }
}

/// Point the rendered config's `[server] url` at `port`, the one the engine
/// is about to bind — the config and the flag never disagree.
fn set_config_port(home: &Home, port: u16) {
    let path = home.config_path();
    let text = std::fs::read_to_string(&path).expect("read the rendered config");
    let server_url = regex::Regex::new(r#"(?m)^url = "127\.0\.0\.1:\d+"$"#).expect("regex");
    assert!(
        server_url.find_iter(&text).count() == 1,
        "the rendered config has no single [server] url line"
    );
    let text = server_url.replace(&text, format!("url = \"127.0.0.1:{port}\""));
    guard::config_text(&text);
    std::fs::write(&path, text.as_ref()).expect("write the rendered config");
}

/// Whether a tracing line carries `key=value` (quoted or not).
fn names(line: &str, key: &str, value: &str) -> bool {
    line.contains(&format!("{key}={value} "))
        || line.ends_with(&format!("{key}={value}"))
        || line.contains(&format!("{key}=\"{value}\""))
}

pub fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
