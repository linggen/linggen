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

    println!();
    Ok(())
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
