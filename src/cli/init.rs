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
                let rel = dir
                    .strip_prefix(crate::paths::linggen_home())
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
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&dest, file.data.as_ref())?;
            count += 1;
        }
    }

    println!(
        "  {}[OK]{} Installed {} default agent specs",
        GREEN, RESET, count
    );
    Ok(())
}

/// Seed built-in missions (the memory `dream`, `memory-scan` and
/// `memory-solve`) into `~/.linggen/missions/`. **Once per mission, then
/// user-owned:** the sentinel (`.builtin-missions-installed`) lists the
/// missions already seeded, one name a line, so an upgrade adds a mission
/// this binary brings for the first time, never clobbers a user's
/// schedule/enabled edits, and never resurrects a mission the user
/// deliberately deleted (deleting `dream` is a supported choice, not a bug).
/// The empty sentinel older versions wrote means `dream` was seeded.
/// Idempotent and safe to call on every daemon start.
pub fn install_default_missions() -> Result<()> {
    let count = seed_missions(&crate::paths::global_missions_dir())?;
    if count > 0 {
        println!(
            "  {}[OK]{} Seeded {} built-in mission(s)",
            GREEN, RESET, count
        );
    }
    Ok(())
}

const SEEDED: &str = ".builtin-missions-installed";

/// Seed every built-in mission `missions_dir` has not had yet; returns how
/// many files were written.
fn seed_missions(missions_dir: &std::path::Path) -> Result<usize> {
    let sentinel = missions_dir.join(SEEDED);
    let mut seeded = seeded_missions(&sentinel);
    let new: Vec<String> = builtin_mission_names()
        .into_iter()
        .filter(|name| !seeded.contains(name))
        .collect();
    if new.is_empty() {
        return Ok(0);
    }
    fs::create_dir_all(missions_dir)?;
    let mut count = 0;
    for name in &new {
        count += seed_mission(missions_dir, name)?;
    }
    seeded.extend(new);
    fs::write(&sentinel, seeded.join("\n") + "\n")?;
    Ok(count)
}

/// The missions the sentinel lists. An empty sentinel is the old
/// install-once marker: `dream` was the only built-in then.
fn seeded_missions(sentinel: &std::path::Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(sentinel) else {
        return Vec::new();
    };
    let names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return vec!["dream".to_string()];
    }
    names
}

/// The built-in missions, by folder name.
fn builtin_mission_names() -> Vec<String> {
    let mut names: Vec<String> = MissionAssets::iter()
        .filter_map(|f| f.split('/').next().map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Write one built-in mission's files, skipping any already there (a
/// user's own file of that name wins).
fn seed_mission(missions_dir: &std::path::Path, name: &str) -> Result<usize> {
    let prefix = format!("{name}/");
    let mut count = 0;
    for filename in MissionAssets::iter().filter(|f| f.starts_with(&prefix)) {
        let Some(file) = MissionAssets::get(&filename) else {
            continue;
        };
        let dest = missions_dir.join(filename.as_ref());
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, file.data.as_ref())?;
        count += 1;
    }
    Ok(count)
}

/// Create a default `linggen.runtime.toml` if no config file exists.
fn ensure_default_config() -> Result<()> {
    let (_, existing_path) = crate::config::Config::load_with_path()?;
    if let Some(path) = &existing_path {
        println!(
            "  {}[OK]{} Config already exists: {}",
            GREEN,
            RESET,
            path.display()
        );
        return Ok(());
    }

    let config = crate::config::Config::default();
    let path = config.save_runtime(None)?;
    println!(
        "  {}[OK]{} Created default config: {}",
        GREEN,
        RESET,
        path.display()
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
                    ran.push(
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                    );
                }
                Ok(None) => {} // no install script
                Err(e) => {
                    println!(
                        "  {}[WARN]{} Install script failed for {}: {}",
                        YELLOW,
                        RESET,
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
            GREEN,
            RESET,
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
            match marketplace::extract_skills_from_zip(
                &temp_zip,
                &target_dir,
                marketplace::SkillPick::All,
            )
            .context("Failed to extract skills")
            {
                Ok(installed) if !installed.is_empty() => {
                    println!(
                        "  {}[OK]{} Installed {} skills from linggen/skills",
                        GREEN,
                        RESET,
                        installed.len()
                    );
                }
                Ok(_) => {
                    println!("  {}[SKIP]{} No skills found in repository", YELLOW, RESET);
                }
                Err(e) => {
                    println!(
                        "  {}[SKIP]{} Skills extraction failed: {}",
                        YELLOW, RESET, e
                    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded(dir: &std::path::Path) -> Vec<String> {
        seeded_missions(&dir.join(SEEDED))
    }

    #[test]
    fn a_fresh_home_gets_every_built_in_mission_once() {
        let dir = tempfile::tempdir().unwrap();
        assert!(seed_missions(dir.path()).unwrap() >= 3);
        for name in ["dream", "memory-scan", "memory-solve"] {
            assert!(dir.path().join(name).join("mission.md").is_file(), "{name}");
        }
        assert_eq!(seeded(dir.path()), builtin_mission_names());
        assert_eq!(seed_missions(dir.path()).unwrap(), 0);
    }

    #[test]
    fn an_old_install_gets_the_new_missions_and_not_a_deleted_dream() {
        let dir = tempfile::tempdir().unwrap();
        // What an older engine left: the empty sentinel, and the user had
        // deleted `dream`.
        fs::write(dir.path().join(SEEDED), b"").unwrap();
        seed_missions(dir.path()).unwrap();
        assert!(
            !dir.path().join("dream").exists(),
            "a deleted dream came back"
        );
        assert!(dir.path().join("memory-scan/mission.md").is_file());
        assert!(dir.path().join("memory-solve/mission.md").is_file());
        assert!(seeded(dir.path()).contains(&"dream".to_string()));
    }

    #[test]
    fn a_users_own_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let mine = dir.path().join("memory-scan/mission.md");
        fs::create_dir_all(mine.parent().unwrap()).unwrap();
        fs::write(&mine, "mine").unwrap();
        seed_missions(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&mine).unwrap(), "mine");
    }
}
