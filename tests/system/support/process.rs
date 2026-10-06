//! Child processes of a test world: their env, their ports, their lifetime.

use super::guard;
use super::home::Home;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Nothing outside reaches the internet from a test world: every HTTP client
/// that honours the proxy variables is sent to a closed local port.
const DEAD_PROXY: &str = "http://127.0.0.1:9";

/// The only environment a child gets (`env -i` + this list). `mem_url` is
/// the world's ling-mem, passed as `LING_MEM_URL` — the engine's default
/// memory address when no config names one (a world with no config).
pub fn hermetic_env(home: &Home, mem_url: Option<&str>) -> Vec<(String, String)> {
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
        (
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", path("bin")),
        ),
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
    if let Some(url) = mem_url {
        vars.push(("LING_MEM_URL", url.to_string()));
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

/// Runs the real child and stops it when the test process is gone — a test
/// killed without unwinding (a nextest timeout) must not leave an engine or a
/// ling-mem running. The watchdog leads its own process group (see
/// [`Owned::spawn`]), so the child and anything it starts share it: a stop
/// is TERM to the group, then KILL to whatever is left after [`STOP_GRACE`].
const WATCHDOG: &str = r#"
owner=$1; shift
"$@" & child=$!
# A stop TERMs the whole group: the child got it too; wait for it to go.
# (`sleep & wait`: a trapped TERM interrupts the wait, not after a full sleep.)
trap 'wait $child; exit 0' TERM INT
while kill -0 $child 2>/dev/null; do
  if ! kill -0 $owner 2>/dev/null; then
    trap '' TERM INT
    kill -TERM 0
    (sleep 5; kill -KILL 0) &
    wait $child
    kill -KILL 0
  fi
  sleep 1 & wait $!
done
wait $child
"#;

/// How long a stopped child gets to exit on TERM before the group is killed.
const STOP_GRACE: Duration = Duration::from_secs(5);

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
        // Its own group, so a stop reaches the child and its children.
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
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
        self.log_from(0)
    }

    /// The log's length in bytes — a mark [`Owned::log_from`] reads after.
    pub fn log_len(&self) -> usize {
        self.log_bytes().len()
    }

    /// The log written after byte `from`. Read as bytes and decoded lossily:
    /// a character the child is halfway through writing shows as U+FFFD,
    /// never a panic or a reset mark.
    pub fn log_from(&self, from: usize) -> String {
        let bytes = self.log_bytes();
        let tail = bytes.get(from..).unwrap_or_default();
        strip_ansi(&String::from_utf8_lossy(tail))
    }

    fn log_bytes(&self) -> Vec<u8> {
        std::fs::read(&self.log).unwrap_or_default()
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
        self.stop();
    }
}

impl Owned {
    /// TERM the child's whole group; KILL whatever is left after
    /// [`STOP_GRACE`]. No grandchild outlives it.
    fn stop(&mut self) {
        let group = -(self.child.id() as libc::pid_t);
        signal(group, libc::SIGTERM);
        let start = Instant::now();
        while group_alive(group) && start.elapsed() < STOP_GRACE {
            // Reap the watchdog as soon as it exits, so its zombie does not
            // keep the group looking alive.
            let _ = self.child.try_wait();
            std::thread::sleep(Duration::from_millis(20));
        }
        if group_alive(group) {
            signal(group, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

fn signal(target: libc::pid_t, sig: libc::c_int) {
    unsafe { libc::kill(target, sig) };
}

/// Whether any process of the group `-pgid` still exists.
fn group_alive(group: libc::pid_t) -> bool {
    unsafe { libc::kill(group, 0) == 0 }
}

/// A server child up and answering on its port.
pub struct Server {
    pub proc: Owned,
    pub port: u16,
}

/// Start a server on `port` (else a free one) and wait for `health` to
/// answer — from our child, not whoever else holds the port. A port taken
/// in the moment between choosing and binding it gets two more tries on
/// fresh ports.
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
            Ok(()) if owns_port(&mut proc, port) => return Ok(Server { proc, port }),
            Ok(()) => last = format!("port {port} answered, but not from our child"),
            Err(e) if e.contains("in use") => last = e,
            Err(e) => return Err(e),
        }
        port = free_port();
    }
    Err(last)
}

/// Whether `proc` is still running and alone on `port`: every listener there
/// belongs to its process group (the child under the watchdog) — a stranger
/// on the same port could be the one that answered.
fn owns_port(proc: &mut Owned, port: u16) -> bool {
    if proc.exited() {
        return false;
    }
    let group = proc.child.id() as libc::pid_t;
    let pids = listeners(port);
    !pids.is_empty()
        && pids
            .iter()
            .all(|&pid| unsafe { libc::getpgid(pid) } == group)
}

/// The pids listening on loopback TCP `port`.
fn listeners(port: u16) -> Vec<libc::pid_t> {
    let out = Command::new("lsof")
        .args(["-nP", "-t", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
        .output()
        .unwrap_or_else(|e| panic!("lsof (to see who listens on {port}): {e}"));
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
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
