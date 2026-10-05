//! Child processes of a test world: their env, their ports, their lifetime.

use super::guard;
use super::home::Home;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Nothing outside reaches the internet from a test world: every HTTP client
/// that honours the proxy variables is sent to a closed local port.
const DEAD_PROXY: &str = "http://127.0.0.1:9";

/// The only environment a child gets (`env -i` + this list).
pub fn hermetic_env(home: &Home) -> Vec<(String, String)> {
    let root = &home.root;
    let path = |p: &str| root.join(p).display().to_string();
    let mut vars = vec![
        ("HOME", home.home().display().to_string()),
        ("LINGGEN_HOME", home.linggen_home().display().to_string()),
        ("LINGGEN_CONFIG", home.config_path().display().to_string()),
        ("TMPDIR", path("tmp")),
        ("XDG_CONFIG_HOME", path("xdg/config")),
        ("XDG_DATA_HOME", path("xdg/data")),
        ("XDG_CACHE_HOME", path("xdg/cache")),
        ("XDG_STATE_HOME", path("xdg/state")),
        ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin".to_string()),
        ("LANG", "en_US.UTF-8".to_string()),
        ("TZ", "UTC".to_string()),
        ("SHELL", "/bin/sh".to_string()),
        ("LINGGEN_NO_TELEMETRY", "1".to_string()),
        ("LING_MEM_NO_TELEMETRY", "1".to_string()),
        ("LINGGEN_SITE_URL", DEAD_PROXY.to_string()),
        ("HF_HUB_OFFLINE", "1".to_string()),
        ("NO_COLOR", "1".to_string()),
    ];
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        vars.push((key, DEAD_PROXY.to_string()));
    }
    for key in ["NO_PROXY", "no_proxy"] {
        vars.push((key, "127.0.0.1,localhost,::1".to_string()));
    }
    let vars: Vec<(String, String)> = vars.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    guard::env(&vars, root);
    vars
}

/// A free loopback port — never one of the real Linggen ports.
pub fn free_port() -> u16 {
    loop {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a probe port");
        let port = listener.local_addr().expect("probe addr").port();
        if !guard::REAL_PORTS.contains(&port) {
            return port;
        }
    }
}

/// Runs the real child and stops it when told to (TERM) or when the test
/// process is gone — a test killed without unwinding (a nextest timeout)
/// must not leave an engine or a ling-mem running.
const WATCHDOG: &str = r#"
owner=$1; shift
"$@" & child=$!
# (`sleep & wait`: a trapped TERM interrupts the wait, not after a full sleep.)
trap 'kill $child 2>/dev/null; wait $child; exit 0' TERM INT
while kill -0 $child 2>/dev/null; do
  kill -0 $owner 2>/dev/null || { kill $child 2>/dev/null; break; }
  sleep 1 & wait $!
done
wait $child
"#;

/// A child that is stopped when the test is done with it.
pub struct Owned {
    pub child: Child,
    pub log: PathBuf,
}

impl Owned {
    pub fn spawn(real: Command, env: &[(String, String)], cwd: &Path, log: PathBuf) -> Self {
        let out = std::fs::File::create(&log).expect("create a child log");
        let err = out.try_clone().expect("clone the child log");
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", WATCHDOG, "watchdog", &std::process::id().to_string()])
            .arg(real.get_program())
            .args(real.get_args());
        cmd.env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        let child = cmd.spawn().unwrap_or_else(|e| panic!("spawn {cmd:?}: {e}"));
        Self { child, log }
    }

    pub fn log_text(&self) -> String {
        strip_ansi(&std::fs::read_to_string(&self.log).unwrap_or_default())
    }

    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// The last lines of the log, for a failure message.
    pub fn log_tail(&self, lines: usize) -> String {
        let text = self.log_text();
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // TERM to the watchdog, which stops the real child and waits for it.
        unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
        let _ = self.child.wait();
    }
}

/// A server child up and answering on its port.
pub struct Server {
    pub proc: Owned,
    pub port: u16,
}

/// Start a server on `port` (else a free one) and wait for `health` to
/// answer. A port taken in the moment between choosing and binding it gets
/// two more tries on fresh ports.
pub async fn start_server(
    port: Option<u16>,
    health: &str,
    spawn: impl Fn(u16) -> Owned,
) -> Result<Server, String> {
    let mut port = port.unwrap_or_else(free_port);
    let mut last = String::new();
    for _ in 0..3 {
        guard::port(port, "a test server");
        let mut proc = spawn(port);
        let url = format!("http://127.0.0.1:{port}{health}");
        match wait_healthy(&mut proc, &url, Duration::from_secs(60)).await {
            Ok(()) => return Ok(Server { proc, port }),
            Err(e) if e.contains("in use") => last = e,
            Err(e) => return Err(e),
        }
        port = free_port();
    }
    Err(last)
}

/// Poll `url` until it answers 2xx, the child dies, or `timeout` passes.
pub async fn wait_healthy(child: &mut Owned, url: &str, timeout: Duration) -> Result<(), String> {
    let client = reqwest::Client::new();
    let start = Instant::now();
    while start.elapsed() < timeout {
        if child.exited() {
            return Err(format!(
                "exited before {url} answered:\n{}",
                child.log_tail(30)
            ));
        }
        let ok = client
            .get(url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if ok {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!(
        "{url} did not answer in {timeout:?}:\n{}",
        child.log_tail(30)
    ))
}

/// Logs with their colour codes removed.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
}
