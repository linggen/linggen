use crate::config::{Config, ProviderKind, SignIn};
use crate::provider::{claude_auth, codex_auth};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn ok(label: &str, detail: &str) {
    println!("  {GREEN}[OK]{RESET}   {label}: {detail}");
}

fn fail(label: &str, detail: &str) {
    println!("  {RED}[FAIL]{RESET} {label}: {detail}");
}

fn info(label: &str, detail: &str) {
    println!("  {CYAN}[INFO]{RESET} {label}: {detail}");
}

pub async fn run(config: &Config, config_path: Option<&Path>) -> Result<()> {
    println!("ling status\n");

    // 1. Version + update check
    let current = env!("CARGO_PKG_VERSION");
    let exe = crate::util::resolved_current_exe()?;
    let latest = if super::prev_binary::in_app_bundle(exe.parent().unwrap_or(&exe)) {
        None
    } else {
        fetch_latest_version().await
    };
    match &latest {
        Some(v) if v != current => {
            println!("  Version:     v{current}  {DIM}(latest: v{v} — run `ling update`){RESET}");
        }
        Some(_) => {
            println!("  Version:     v{current}  {DIM}(up to date){RESET}");
        }
        None => {
            println!("  Version:     v{current}");
        }
    }

    // 2. Config
    match config_path {
        Some(p) => println!("  Config:      {}", p.display()),
        None => println!("  Config:      (default)"),
    }
    for line in Config::health_lines(&Config::candidate_paths()) {
        fail("Config", &line);
    }

    // 3. Workspace
    match crate::paths::resolve_workspace_root(None) {
        Ok(ws) => println!("  Workspace:   {}", ws.display()),
        Err(_) => println!("  Workspace:   none"),
    }

    // 4. Agent server
    let port = config.server.port();
    let listening = is_port_listening(port).await;
    let recorded = std::fs::read_to_string(crate::paths::linggen_home().join("ling.pid"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(recorded_pid);
    let listener = if listening { listener_pid(port) } else { None };
    let (icon, detail) = server_verdict(port, listening, recorded, listener);
    println!("  Agent:       {} {}", icon, detail);

    let (exe, commit) = if listening {
        fetch_running_binary(port).await
    } else {
        None
    }
    .unwrap_or_else(|| (exe.display().to_string(), env!("LINGGEN_BUILD_COMMIT").into()));
    println!("Binary: {exe} · {commit}");

    // 5. Logs
    check_log_dir(config);

    // 6. Models
    println!();
    check_models(config).await;

    // 7. Skills
    println!();
    check_skills_dirs();

    // 8. Agents
    println!();
    check_agents_dir();

    // 9. ling-mem (memory backend)
    println!();
    check_ling_mem().await;

    // 10. Hosts still on an old Linggen plugin
    check_old_plugins();

    println!();
    Ok(())
}

/// A host that called a retired Linggen door or tool lately: its plugin is
/// old, and its hooks say nothing — so this does, with the update for it.
fn check_old_plugins() {
    for (host, seen) in crate::perception::old_plugin::recent() {
        let name = crate::perception::old_plugin::host_name(&host);
        fail(
            "Old plugin",
            &format!(
                "{name} called the retired {} on {} — {}",
                seen.called,
                seen.day,
                crate::perception::old_plugin::update_commands(&host)
            ),
        );
    }
}

/// Memory health. Memory is served by the engine itself (`memory_host`): the
/// store's lock names the process holding it and the port it serves on, and
/// that port's `/mem/api/health` says whether it answers. Then the on-disk
/// store and its backup — each independent.
async fn check_ling_mem() {
    println!("  Memory:");

    let holder = ling_mem::hosted::StoreLock::holder(crate::paths::linggen_home());
    match holder.and_then(|h| h.port.map(|port| (h.pid, port))) {
        Some((pid, port)) => {
            let url = format!("http://127.0.0.1:{port}{}", crate::memory_host::PREFIX);
            match fetch_ling_mem_health(&url).await {
                Some(v) => ok(
                    "Serving   ",
                    &format!("in the engine (pid {pid}) at {url} (v{v})"),
                ),
                None => info("Serving   ", &format!("{url} not answering")),
            }
        }
        None => info("Serving   ", "not open (the running engine serves memory)"),
    }
    let home = crate::paths::linggen_home();
    let version = crate::memory_host::VERSION;
    match ling_mem::hosted::backup_failure(home, version) {
        Some(why) => fail(
            "Backup    ",
            &format!(
                "the store backup before {version} first opened it failed: {}",
                why.trim()
            ),
        ),
        None if ling_mem::hosted::backup_taken(home, version) => ok(
            "Backup    ",
            &format!("taken once for {version} (memory/backups/)"),
        ),
        None => info("Backup    ", &format!("not yet taken for {version}")),
    }

    // Store. Memory rows live under ~/.linggen/memory/memory.lancedb/
    // with two tables: `semantic` (core + long-term) and `episodic`
    // (staging). The dir exists from first daemon launch.
    let store = crate::paths::linggen_home().join("memory/memory.lancedb");
    if store.exists() {
        ok("Store     ", &format!("{}", store.display()));
    } else {
        info(
            "Store     ",
            &format!("{} (will be created on first write)", store.display()),
        );
    }

}

async fn fetch_ling_mem_health(url: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        ok: bool,
        data: Option<HealthData>,
    }
    #[derive(serde::Deserialize)]
    struct HealthData {
        #[allow(dead_code)]
        status: String,
        version: String,
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(format!("{url}/api/health")).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let env: Envelope = resp.json().await.ok()?;
    if !env.ok {
        return None;
    }
    env.data.map(|d| d.version)
}

/// The Agent line. `recorded` is what `ling.pid` says and whether that
/// process is alive; `listener` is who actually holds the port. A pid is
/// only printed when it is alive — a listening port with a dead pidfile
/// shows the real listener (or "unknown") and names the stale file, never
/// a pid from a previous life.
fn server_verdict(
    port: u16,
    listening: bool,
    recorded: Option<(u32, bool)>,
    listener: Option<u32>,
) -> (&'static str, String) {
    // The recorded engine is the server only when no other holder of the port is known.
    let recorded = match (recorded, listener) {
        (Some((pid, true)), Some(holder)) if holder != pid => Some((pid, false)),
        _ => recorded,
    };
    match (listening, recorded) {
        (true, Some((pid, true))) => ("\u{2705}", format!("port {} running (PID {})", port, pid)),
        (true, stale) => {
            let who = listener
                .map(|p| format!("PID {p}"))
                .unwrap_or_else(|| "PID unknown".to_string());
            let note = match stale {
                Some((pid, false)) => format!(" \u{b7} ling.pid is stale ({pid})"),
                _ => String::new(),
            };
            (
                "\u{2705}",
                format!("port {} running ({}){}", port, who, note),
            )
        }
        (false, Some((pid, true))) => (
            "\u{274c}",
            format!(
                "port {} process alive (PID {}) but port not listening",
                port, pid
            ),
        ),
        (false, Some((pid, false))) => (
            "\u{274c}",
            format!("port {} not running (stale PID {})", port, pid),
        ),
        (false, None) => ("\u{274c}", format!("port {} not running", port)),
    }
}

/// Who holds the port right now, via lsof (macOS and most Linux). None when
/// lsof is absent or finds nothing — the caller prints "PID unknown".
#[cfg(unix)]
fn listener_pid(port: u16) -> Option<u32> {
    let out = std::process::Command::new("lsof")
        .args(["-ti", &format!("TCP:{port}"), "-sTCP:LISTEN"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().parse::<u32>().ok())
}

#[cfg(not(unix))]
fn listener_pid(_port: u16) -> Option<u32> {
    None
}

/// The pid `ling.pid` names, and whether it is our engine running.
fn recorded_pid(pid: u32) -> (u32, bool) {
    (pid, is_process_running(pid) && super::engine_restart::is_our_engine(pid))
}

fn is_process_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn fetch_running_binary(port: u16) -> Option<(String, String)> {
    #[derive(serde::Deserialize)]
    struct Binary {
        exe: String,
        commit: String,
    }
    let binary: Binary = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?
        .get(format!("http://127.0.0.1:{port}/api/health"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    Some((binary.exe, binary.commit))
}

async fn is_port_listening(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

async fn fetch_latest_version() -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        version: String,
    }

    let client = reqwest::Client::builder()
        .user_agent("linggen")
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;

    let resp = client
        .get(super::self_update::ReleaseSource::from_env().manifest_url())
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let manifest: Manifest = resp.json().await.ok()?;
    Some(manifest.version)
}

async fn check_models(config: &Config) {
    // What the load set right instead of refusing the config (a bad price).
    for problem in &config.problems {
        fail("Config", problem);
    }
    // Chat models only: a live-voice row has no chat endpoint to probe.
    if config.chat_models().next().is_none() {
        println!("  Models (0):");
        info("  none configured", "");
        return;
    }

    println!("  Models ({}):", config.chat_models().count());

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            fail("  HTTP client", "failed to build");
            return;
        }
    };

    // Keys come from where the engine gets them (TOML > credentials.json >
    // env) — the old probe only saw the TOML field, so a key kept in
    // credentials.json looked like a 401.
    let creds = crate::credentials::Credentials::load_for(
        &crate::credentials::credentials_file(),
        &config.models,
    );

    for m in config.chat_models() {
        let label = format!("Model [{}]", m.id);
        let base = m.url.trim_end_matches('/');
        let sign_in = m.sign_in();
        let kind = m.provider_kind();

        // OAuth-backed models have no key to send and nothing to ping
        // anonymously — the honest check is the sign-in state.
        if sign_in == SignIn::ChatGpt {
            let tokens = codex_auth::CodexAuthTokens::load(&codex_auth::codex_auth_file());
            if tokens.is_valid() {
                ok(
                    &label,
                    &format!("{} via ChatGPT OAuth (signed in)", m.model),
                );
            } else {
                fail(
                    &label,
                    &format!(
                        "{} via ChatGPT OAuth (not signed in \u{2014} run `ling auth login`)",
                        m.model
                    ),
                );
            }
            continue;
        }
        if kind == ProviderKind::Anthropic && sign_in == SignIn::Claude {
            match claude_auth::load() {
                Ok(t) if t.can_do_inference() => {
                    ok(&label, &format!("{} via Claude OAuth (signed in)", m.model));
                }
                _ => fail(
                    &label,
                    &format!("{} via Claude OAuth (not signed in)", m.model),
                ),
            }
            continue;
        }
        if sign_in == SignIn::LinggenAccount {
            info(
                &label,
                &format!("{} via Linggen account (checked at run time)", m.model),
            );
            continue;
        }

        let key = crate::credentials::resolve_api_key_shared(m, &config.models, &creds);
        // Mirror the engine's own routing (provider/models.rs): ollama and
        // anthropic speak their own wire; every other provider — openai,
        // deepseek, gemini, groq, … — is OpenAI-compatible, so `/models` is
        // the probe for all of them.
        let mut req = match kind {
            ProviderKind::Ollama => client.get(format!("{base}/api/tags")),
            ProviderKind::Anthropic => {
                let url = if base.ends_with("/v1") {
                    format!("{base}/models")
                } else {
                    format!("{base}/v1/models")
                };
                let mut r = client.get(url).header("anthropic-version", "2023-06-01");
                if let Some(k) = &key {
                    r = r.header("x-api-key", k);
                }
                r
            }
            _ => client.get(format!("{base}/models")),
        };
        if kind != ProviderKind::Anthropic {
            if let Some(k) = &key {
                req = req.header("Authorization", format!("Bearer {k}"));
            }
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                ok(&label, &format!("{} @ {} (reachable)", m.model, m.url));
            }
            Ok(resp) if matches!(resp.status().as_u16(), 401 | 403) => {
                let why = if key.is_some() {
                    "key rejected"
                } else {
                    "no API key in credentials.json or TOML"
                };
                fail(
                    &label,
                    &format!(
                        "{} @ {} (HTTP {} \u{2014} {})",
                        m.model,
                        m.url,
                        resp.status(),
                        why
                    ),
                );
            }
            Ok(resp) => {
                fail(
                    &label,
                    &format!("{} @ {} (HTTP {})", m.model, m.url, resp.status()),
                );
            }
            Err(e) => {
                fail(&label, &format!("{} @ {} ({})", m.model, m.url, e));
            }
        }
    }
}

fn check_skills_dirs() {
    println!("  Skills:");
    let dirs: Vec<(PathBuf, &str)> = vec![
        (crate::paths::global_skills_dir(), "global"),
        (PathBuf::from(".linggen/skills"), "project"),
    ];

    for (dir, scope) in dirs {
        if dir.exists() {
            let count = std::fs::read_dir(&dir)
                .map(|entries| entries.filter_map(|e| e.ok()).count())
                .unwrap_or(0);
            ok(
                &format!("Skills ({})", scope),
                &format!("{} entries in {}", count, dir.display()),
            );
        } else {
            info(
                &format!("Skills ({})", scope),
                &format!("{} (not found)", dir.display()),
            );
        }
    }
}

fn check_agents_dir() {
    println!("  Agents:");
    let count_md = |dir: &Path| -> usize {
        if !dir.exists() {
            return 0;
        }
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
                    .count()
            })
            .unwrap_or(0)
    };

    let global_dir = crate::paths::global_agents_dir();
    if global_dir.exists() {
        let count = count_md(&global_dir);
        ok(
            "Agents (global)",
            &format!("{} agent files in {}", count, global_dir.display()),
        );
    } else {
        info(
            "Agents (global)",
            &format!("{} (not found)", global_dir.display()),
        );
    }

    let project_dir = PathBuf::from("agents");
    if project_dir.exists() {
        let count = count_md(&project_dir);
        ok("Agents (project)", &format!("{} agent files", count));
    } else {
        info("Agents (project)", "agents/ directory not found");
    }
}

fn check_log_dir(config: &Config) {
    let log_dir: Option<PathBuf> = config
        .logging
        .directory
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| Some(crate::paths::logs_dir()));

    match log_dir {
        Some(dir) if dir.exists() => {
            let test_path = dir.join(".doctor-check");
            match std::fs::write(&test_path, "") {
                Ok(_) => {
                    let _ = std::fs::remove_file(&test_path);
                    println!("  Logs:        {}", dir.display());
                }
                Err(_) => println!(
                    "  Logs:        {} {RED}(not writable){RESET}",
                    dir.display()
                ),
            }
        }
        Some(dir) => println!("  Logs:        {} {CYAN}(not found){RESET}", dir.display()),
        None => println!("  Logs:        {CYAN}(unknown){RESET}"),
    }
}

#[cfg(test)]
mod tests {
    use super::server_verdict;

    #[test]
    fn live_pidfile_is_printed() {
        let (icon, s) = server_verdict(9527, true, Some((4724, true)), Some(4724));
        assert_eq!(icon, "\u{2705}");
        assert_eq!(s, "port 9527 running (PID 4724)");
    }

    #[test]
    fn dead_pidfile_never_shown_as_the_server() {
        // The 2026-09-01 case: ling.pid from July, real listener elsewhere.
        let (_, s) = server_verdict(9527, true, Some((45747, false)), Some(71972));
        assert_eq!(
            s,
            "port 9527 running (PID 71972) \u{b7} ling.pid is stale (45747)"
        );
    }

    /// A live pid is the listener only when it is our engine and the port's
    /// holder is not known to be another process.
    #[test]
    fn a_live_pid_that_is_not_ours_or_not_the_listener_is_not_the_server() {
        let mut other = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let recorded = super::recorded_pid(other.id());
        let _ = other.kill();
        let _ = other.wait();
        assert_eq!(recorded, (other.id(), false), "another program under the recorded pid");

        let (_, s) = server_verdict(9527, true, Some((4724, true)), Some(71972));
        assert_eq!(s, "port 9527 running (PID 71972) \u{b7} ling.pid is stale (4724)");
    }

    #[test]
    fn listening_without_any_pid_is_still_running() {
        let (icon, s) = server_verdict(9527, true, None, None);
        assert_eq!(icon, "\u{2705}");
        assert_eq!(s, "port 9527 running (PID unknown)");
    }

    #[test]
    fn not_listening_verdicts() {
        assert_eq!(
            server_verdict(9527, false, Some((7, true)), None).1,
            "port 9527 process alive (PID 7) but port not listening"
        );
        assert_eq!(
            server_verdict(9527, false, Some((7, false)), None).1,
            "port 9527 not running (stale PID 7)"
        );
        assert_eq!(
            server_verdict(9527, false, None, None).1,
            "port 9527 not running"
        );
    }
}
