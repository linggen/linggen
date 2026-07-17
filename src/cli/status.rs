use crate::config::Config;
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
    let latest = fetch_latest_version().await;
    match &latest {
        Some(v) if v != current => {
            println!(
                "  Version:     v{current}  {DIM}(latest: v{v} — run `ling update`){RESET}"
            );
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

    // 3. Workspace
    match crate::paths::resolve_workspace_root(None) {
        Ok(ws) => println!("  Workspace:   {}", ws.display()),
        Err(_) => println!("  Workspace:   none"),
    }

    // 4. Agent server
    let port = config.server.port;
    let listening = is_port_listening(port).await;
    let pid = std::fs::read_to_string(crate::paths::linggen_home().join("ling.pid"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    print_server_status(port, listening, pid);

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

    println!();
    Ok(())
}

/// Memory backend (`ling-mem`) health. Reports the binary location +
/// version, the daemon's `/api/health` response on :9888, the on-disk
/// store, and the canonical skill bundle. Each independent — a missing
/// daemon doesn't mask a present binary, etc.
async fn check_ling_mem() {
    println!("  Memory (ling-mem):");

    // Binary on PATH.
    let bin = which_path("ling-mem");
    match &bin {
        Some(path) => {
            let ver = ling_mem_version(path).unwrap_or_else(|| "unknown".to_string());
            ok("Binary    ", &format!("{} (v{})", path.display(), ver));
        }
        None => fail("Binary    ", "ling-mem not on PATH (auto-installs on first memory op; or see linggen.dev/memory)"),
    }

    // Daemon on the default port. Check the standard port; users running
    // a non-default port will see a Skip and can rely on `ling-mem status`.
    let port: u16 = crate::config::DEFAULT_LING_MEM_PORT;
    if is_port_listening(port).await {
        match fetch_ling_mem_health(port).await {
            Some(v) => ok("Daemon    ", &format!(":{port} healthy (v{v})")),
            None    => fail("Daemon    ", &format!(":{port} listening but /api/health did not respond")),
        }
    } else {
        info("Daemon    ", &format!(":{port} not running (start with `ling-mem start`)"));
    }

    // Store. Memory rows live under ~/.linggen/memory/memory.lancedb/
    // with two tables: `semantic` (core + long-term) and `episodic`
    // (staging). The dir exists from first daemon launch.
    let store = crate::paths::linggen_home().join("memory/memory.lancedb");
    if store.exists() {
        ok("Store     ", &format!("{}", store.display()));
    } else {
        info("Store     ", &format!("{} (will be created on first write)", store.display()));
    }

    // Canonical shared-memory skill bundle. Per-host SKILL.md stubs
    // point back here for references/scripts/hooks; absence means the
    // skill hasn't been installed (or was installed pre-canonical-layout).
    let skill = crate::paths::linggen_home().join("skills/shared-memory");
    if skill.join("SKILL.md").is_file() {
        ok("Skill     ", &format!("{}", skill.display()));
    } else {
        info("Skill     ", &format!("{} (run install-shared-memory.sh to install)", skill.display()));
    }
}

fn which_path(bin: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("which").arg(bin).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(PathBuf::from(s)) }
}

fn ling_mem_version(bin: &Path) -> Option<String> {
    // `ling-mem --version` prints `ling-mem <ver>` — pick the 2nd token.
    let out = std::process::Command::new(bin).arg("--version").output().ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout);
    s.split_whitespace().nth(1).map(|v| v.to_string())
}

async fn fetch_ling_mem_health(port: u16) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Envelope { ok: bool, data: Option<HealthData> }
    #[derive(serde::Deserialize)]
    struct HealthData { #[allow(dead_code)] status: String, version: String }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build().ok()?;
    let resp = client.get(format!("http://127.0.0.1:{port}/api/health")).send().await.ok()?;
    if !resp.status().is_success() { return None; }
    let env: Envelope = resp.json().await.ok()?;
    if !env.ok { return None; }
    env.data.map(|d| d.version)
}

fn print_server_status(port: u16, listening: bool, pid: Option<u32>) {
    let (icon, detail) = match (listening, pid) {
        (true, Some(pid)) => ("\u{2705}", format!("port {} running (PID {})", port, pid)),
        (true, None) => ("\u{2705}", format!("port {} running", port)),
        (false, Some(pid)) => {
            if is_process_running(pid) {
                (
                    "\u{274c}",
                    format!("port {} process alive (PID {}) but port not listening", port, pid),
                )
            } else {
                ("\u{274c}", format!("port {} not running (stale PID)", port))
            }
        }
        (false, None) => ("\u{274c}", format!("port {} not running", port)),
    };
    println!("  Agent:       {} {}", icon, detail);
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
        .get("https://github.com/linggen/linggen/releases/latest/download/manifest.json")
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
    if config.models.is_empty() {
        println!("  Models (0):");
        info("  none configured", "");
        return;
    }

    println!("  Models ({}):", config.models.len());

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

    for m in &config.models {
        let label = format!("Model [{}]", m.id);
        let check_url = match m.provider.as_str() {
            "ollama" => format!("{}/api/tags", m.url.trim_end_matches('/')),
            "openai" => format!("{}/models", m.url.trim_end_matches('/')),
            _ => {
                info(&label, &format!("unknown provider '{}'", m.provider));
                continue;
            }
        };

        let mut req = client.get(&check_url);
        if let Some(key) = &m.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                ok(&label, &format!("{} @ {} (reachable)", m.model, m.url));
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
                    .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
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
                Err(_) => println!("  Logs:        {} {RED}(not writable){RESET}", dir.display()),
            }
        }
        Some(dir) => println!("  Logs:        {} {CYAN}(not found){RESET}", dir.display()),
        None => println!("  Logs:        {CYAN}(unknown){RESET}"),
    }
}
