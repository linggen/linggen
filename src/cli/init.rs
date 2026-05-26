use crate::extensions::skills::marketplace;
use anyhow::{Context, Result};
use rust_embed::Embed;
use std::fs;
use std::path::PathBuf;

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

/// All files under `agents/` are embedded at compile time.
#[derive(Embed)]
#[folder = "agents/"]
struct AgentAssets;

/// Built-in missions (e.g. the memory `dream` consolidation pass),
/// embedded at compile time. Unlike agents, these are seeded **once**
/// (sentinel-gated) and then owned by the user — see
/// [`install_default_missions`].
#[derive(Embed)]
#[folder = "missions/"]
struct MissionAssets;

pub async fn run(_global: bool, _root: Option<PathBuf>) -> Result<()> {
    println!("ling init — setting up Linggen environment\n");

    // 1. Create ~/.linggen/ directory tree
    ensure_directories();

    // 2. Install default agent specs
    install_default_agents()?;
    install_default_missions()?;

    // 3. Create default config if missing
    ensure_default_config()?;

    // 4. Download default skills (best-effort)
    install_default_skills().await;

    // 5. Run install scripts for skills that declare one
    run_skill_install_scripts();

    // 6. Summary
    println!();
    println!("{}Done!{} Linggen is ready.", GREEN, RESET);
    println!("  Run `ling` to start the server and open the web UI.");
    println!("  Run `ling doctor` to verify your setup.");

    Ok(())
}

/// Create all standard directories under ~/.linggen/ if they don't exist.
fn ensure_directories() {
    let dirs = [
        crate::paths::linggen_home().to_path_buf(),
        crate::paths::config_dir(),
        crate::paths::logs_dir(),
        crate::paths::global_agents_dir(),
        crate::paths::global_skills_dir(),
        crate::paths::global_missions_dir(),
    ];

    for dir in &dirs {
        match fs::create_dir_all(dir) {
            Ok(_) => {
                let rel = dir.strip_prefix(crate::paths::linggen_home())
                    .map(|p| format!("~/.linggen/{}", p.display()))
                    .unwrap_or_else(|_| dir.display().to_string());
                println!("  {}[OK]{} {}", GREEN, RESET, rel);
            }
            Err(e) => {
                println!("  [ERR] {} — {}", dir.display(), e);
            }
        }
    }
}

/// Install (or update) built-in agent specs to `~/.linggen/agents/`.
/// Always overwrites to keep agents in sync with the binary version.
pub fn install_default_agents() -> Result<()> {
    let agents_dir = crate::paths::global_agents_dir();
    fs::create_dir_all(&agents_dir)?;

    let mut count = 0;
    for filename in AgentAssets::iter() {
        if let Some(file) = AgentAssets::get(&filename) {
            let dest = agents_dir.join(filename.as_ref());
            fs::write(&dest, file.data.as_ref())?;
            count += 1;
        }
    }

    println!("  {}[OK]{} Installed {} default agent specs", GREEN, RESET, count);
    Ok(())
}

/// Seed built-in missions (the memory `dream` consolidation pass) into
/// `~/.linggen/missions/`. **Install-once, then user-owned:** a sentinel
/// (`.builtin-missions-installed`) records that seeding has happened, so
/// upgrades never clobber a user's schedule/enabled edits and — crucially
/// — never resurrect a mission the user deliberately deleted (deleting
/// `dream` is a supported choice that disables auto-consolidation, not a
/// bug). Idempotent and safe to call on every daemon start.
pub fn install_default_missions() -> Result<()> {
    let missions_dir = crate::paths::global_missions_dir();
    let sentinel = missions_dir.join(".builtin-missions-installed");
    if sentinel.exists() {
        return Ok(());
    }
    fs::create_dir_all(&missions_dir)?;

    let mut count = 0;
    for filename in MissionAssets::iter() {
        let Some(file) = MissionAssets::get(&filename) else {
            continue;
        };
        let dest = missions_dir.join(filename.as_ref());
        // Only seed if absent — never overwrite an existing (possibly
        // user-edited) mission.
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, file.data.as_ref())?;
        count += 1;
    }

    // Mark seeding done regardless of count, so a user-deleted built-in
    // mission is not re-created on the next start.
    fs::write(&sentinel, b"")?;
    if count > 0 {
        println!("  {}[OK]{} Seeded {} built-in mission(s)", GREEN, RESET, count);
    }
    Ok(())
}

/// Create a default `linggen.runtime.toml` if no config file exists.
fn ensure_default_config() -> Result<()> {
    let (_, existing_path) = crate::config::Config::load_with_path()?;
    if let Some(path) = &existing_path {
        println!("  {}[OK]{} Config already exists: {}", GREEN, RESET, path.display());
        return Ok(());
    }

    let config = crate::config::Config::default();
    let path = config.save_runtime(None)?;
    println!(
        "  {}[OK]{} Created default config: {}",
        GREEN, RESET, path.display()
    );
    println!(
        "        {}Tip:{} Edit this file to add your model providers and API keys.",
        YELLOW, RESET
    );

    Ok(())
}

/// Run install scripts for any installed skills that declare an `install` field.
fn run_skill_install_scripts() {
    let skills_dir = crate::paths::global_skills_dir();
    let entries = match std::fs::read_dir(&skills_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut ran = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            match crate::extensions::skills::run_install_script(&path) {
                Ok(Some(_)) => {
                    ran.push(path.file_name().unwrap_or_default().to_string_lossy().to_string());
                }
                Ok(None) => {} // no install script
                Err(e) => {
                    println!(
                        "  {}[WARN]{} Install script failed for {}: {}",
                        YELLOW, RESET,
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        e
                    );
                }
            }
        }
    }
    if !ran.is_empty() {
        println!(
            "  {}[OK]{} Ran install scripts for: {}",
            GREEN, RESET,
            ran.join(", ")
        );
    }
}

/// Download skills from the linggen/skills GitHub repo (best-effort).
async fn install_default_skills() {
    let target_dir = crate::paths::global_skills_dir();

    let (owner, repo) = ("linggen", "skills");
    let zip_url = marketplace::build_github_zip_url(owner, repo, "main");

    let client = match marketplace::http_client() {
        Ok(c) => c,
        Err(_) => {
            println!(
                "  {}[SKIP]{} Skills download (could not create HTTP client)",
                YELLOW, RESET
            );
            return;
        }
    };

    match marketplace::download_to_temp(&client, &zip_url).await {
        Ok(temp_zip) => {
            match marketplace::extract_all_skills_from_zip(&temp_zip, &target_dir)
                .context("Failed to extract skills")
            {
                Ok(installed) if !installed.is_empty() => {
                    println!(
                        "  {}[OK]{} Installed {} skills from linggen/skills",
                        GREEN, RESET, installed.len()
                    );
                }
                Ok(_) => {
                    println!("  {}[SKIP]{} No skills found in repository", YELLOW, RESET);
                }
                Err(e) => {
                    println!("  {}[SKIP]{} Skills extraction failed: {}", YELLOW, RESET, e);
                }
            }
            let _ = fs::remove_file(&temp_zip);
        }
        Err(_) => {
            println!(
                "  {}[SKIP]{} Skills download failed (offline or repo unavailable)",
                YELLOW, RESET
            );
        }
    }
}
