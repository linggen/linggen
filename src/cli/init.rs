use crate::extensions::skills::marketplace;
use anyhow::{Context, Result};
use rust_embed::Embed;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde_norway::Mapping;

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

/// All files under `agents/` are embedded at compile time.
#[derive(Embed)]
#[folder = "agents/"]
struct AgentAssets;

/// Built-in mission definitions, embedded at compile time.
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

/// Seed new built-in missions and refresh the installed ones without
/// changing personal settings. The sentinel keeps deleted missions deleted.
pub fn install_default_missions() -> Result<()> {
    let count = sync_builtin_missions(&crate::paths::global_missions_dir())?;
    if count > 0 {
        println!(
            "  {}[OK]{} Installed or refreshed {} built-in mission file(s)",
            GREEN, RESET, count
        );
    }
    Ok(())
}

const SEEDED: &str = ".builtin-missions-installed";

/// The person owns scheduling and model selection. Everything else,
/// including kickoff messages, follows the binary.
const PERSONAL_MISSION_FIELDS: &[&str] = &["enabled", "schedule", "catchup_hours", "model"];

/// Seed new built-ins and refresh previously seeded definitions. Return the
/// number of files installed or refreshed (not counting backups).
fn sync_builtin_missions(missions_dir: &Path) -> Result<usize> {
    let sentinel = missions_dir.join(SEEDED);
    let mut seeded = seeded_missions(&sentinel)?;
    let mut count = 0;
    for name in builtin_mission_names() {
        let dir = missions_dir.join(&name);
        match dir.symlink_metadata() {
            Ok(metadata) if !metadata.is_dir() => continue,
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        let path = dir.join("mission.md");
        if seeded.contains(&name) {
            match refresh_mission(&path, &name) {
                Ok(changed) => count += usize::from(changed),
                Err(e) => tracing::warn!("Failed to refresh built-in mission {name}: {e:#}"),
            }
            continue;
        }
        // A pre-existing definition was not seeded by the engine. Do not
        // claim it, even if its directory has a built-in name.
        if seed_mission(&path, &name)? {
            seeded.push(name);
            fs::write(&sentinel, seeded.join("\n") + "\n")?;
            count += 1;
        }
    }
    Ok(count)
}

fn refresh_mission(path: &Path, name: &str) -> Result<bool> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let old = fs::read_to_string(path)?;
    let builtin = builtin_mission(name)?;
    let Some(updated) = merge_mission(&old, std::str::from_utf8(&builtin.data)?)? else {
        return Ok(false);
    };

    let dir = path.parent().context("Missing mission directory")?;
    let backup = path.with_extension("md.prev");
    if !backup.try_exists()? {
        let mut copy = tempfile::NamedTempFile::new_in(dir)?;
        copy.write_all(old.as_bytes())?;
        if let Err(e) = copy.persist_noclobber(&backup) {
            if e.error.kind() != ErrorKind::AlreadyExists {
                return Err(e.into());
            }
        }
    }
    let mut replacement = tempfile::NamedTempFile::new_in(dir)?;
    replacement.write_all(updated.as_bytes())?;
    replacement.as_file().set_permissions(metadata.permissions())?;
    replacement.persist(path)?;
    Ok(true)
}

fn merge_mission(installed: &str, builtin: &str) -> Result<Option<String>> {
    use crate::extensions::frontmatter::split;
    let (installed_yaml, installed_body) = split(installed);
    let (builtin_yaml, builtin_body) = split(builtin);
    let mut installed: Mapping = serde_norway::from_str(installed_yaml.unwrap_or("{}"))?;
    let mut builtin: Mapping =
        serde_norway::from_str(builtin_yaml.context("Missing built-in frontmatter")?)?;
    let mut personal = Mapping::new();
    for key in PERSONAL_MISSION_FIELDS {
        builtin.remove(*key);
        if let Some(value) = installed.remove(*key) {
            personal.insert((*key).into(), value);
        }
    }
    if installed == builtin && installed_body == builtin_body {
        return Ok(None);
    }
    builtin.extend(personal);
    let yaml = serde_norway::to_string(&builtin)?;
    Ok(Some(format!("---\n{yaml}---{builtin_body}")))
}

/// The missions the sentinel lists. An empty sentinel is the old
/// install-once marker: `dream` was the only built-in then.
fn seeded_missions(sentinel: &Path) -> Result<Vec<String>> {
    let text = match fs::read_to_string(sentinel) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return Ok(vec!["dream".to_string()]);
    }
    Ok(names)
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

/// Seed a definition only if no mission already occupies this name.
fn seed_mission(path: &Path, name: &str) -> Result<bool> {
    let builtin = builtin_mission(name)?;
    fs::create_dir_all(path.parent().context("Missing mission directory")?)?;
    let mut file = match fs::File::create_new(path) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    file.write_all(builtin.data.as_ref())?;
    Ok(true)
}

fn builtin_mission(name: &str) -> Result<rust_embed::EmbeddedFile> {
    MissionAssets::get(&format!("{name}/mission.md")).context("Missing built-in definition")
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
        seeded_missions(&dir.join(SEEDED)).unwrap()
    }

    #[test]
    fn a_fresh_home_gets_every_built_in_mission_once() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sync_builtin_missions(dir.path()).unwrap() >= 3);
        for name in ["dream", "memory-scan", "memory-solve"] {
            assert!(dir.path().join(name).join("mission.md").is_file(), "{name}");
        }
        assert_eq!(seeded(dir.path()), builtin_mission_names());
        assert_eq!(sync_builtin_missions(dir.path()).unwrap(), 0);
    }

    #[test]
    fn an_old_install_gets_the_new_missions_and_not_a_deleted_dream() {
        let dir = tempfile::tempdir().unwrap();
        // What an older engine left: the empty sentinel, and the user had
        // deleted `dream`.
        fs::write(dir.path().join(SEEDED), b"").unwrap();
        sync_builtin_missions(dir.path()).unwrap();
        assert!(
            !dir.path().join("dream").exists(),
            "a deleted dream came back"
        );
        assert!(dir.path().join("memory-scan/mission.md").is_file());
        assert!(dir.path().join("memory-solve/mission.md").is_file());
        assert!(seeded(dir.path()).contains(&"dream".to_string()));
    }

    #[test]
    fn refresh_replaces_engine_fields_but_keeps_personal_settings() {
        let dir = tempfile::tempdir_in("target").unwrap();
        sync_builtin_missions(dir.path()).unwrap();
        let path = dir.path().join("dream/mission.md");
        let old = "---\nname: dream\nschedule: '17 5 * * *'\nenabled: false\nmodel: my-model\ncatchup_hours: 0\nallowed-tools: [OldTool]\npermission: {paths: [{path: '~', mode: admin}]}\nkickoff: [Old kickoff]\n---\n\nOld instructions.\n";
        fs::write(&path, old).unwrap();
        sync_builtin_missions(dir.path()).unwrap();
        let refreshed = fs::read_to_string(&path).unwrap();
        let builtin = MissionAssets::get("dream/mission.md").unwrap();
        let parse = |text: &str| {
            let (yaml, _) = crate::extensions::frontmatter::split(text);
            serde_norway::from_str::<serde_norway::Mapping>(yaml.unwrap()).unwrap()
        };
        let mut expected = parse(std::str::from_utf8(&builtin.data).unwrap());
        for (key, value) in [
            ("schedule", "'17 5 * * *'"),
            ("enabled", "false"),
            ("model", "my-model"),
            ("catchup_hours", "0"),
        ] {
            expected.insert(key.into(), serde_norway::from_str(value).unwrap());
        }
        assert_eq!(parse(&refreshed), expected);
        assert_eq!(fs::read_to_string(path.with_extension("md.prev")).unwrap(), old);

        // A later refresh keeps the first backup, and unset overrides stay unset.
        fs::write(&path, "---\nname: dream\n---\n\nAnother older body.\n").unwrap();
        sync_builtin_missions(dir.path()).unwrap();
        let refreshed = parse(&fs::read_to_string(&path).unwrap());
        for key in ["schedule", "enabled", "model", "catchup_hours"] {
            assert!(!refreshed.contains_key(key), "restored an unset {key}");
        }
        assert_eq!(fs::read_to_string(path.with_extension("md.prev")).unwrap(), old);
    }

    #[test]
    fn refresh_ignores_edits_to_personal_settings_alone() {
        let dir = tempfile::tempdir_in("target").unwrap();
        sync_builtin_missions(dir.path()).unwrap();
        let path = dir.path().join("dream/mission.md");
        let customized = fs::read_to_string(&path)
            .unwrap()
            .replace("enabled: true", "enabled: false # paused");
        fs::write(&path, &customized).unwrap();
        assert_eq!(sync_builtin_missions(dir.path()).unwrap(), 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), customized);
        assert!(!path.with_extension("md.prev").exists());
    }

    #[test]
    fn a_users_own_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let mine = dir.path().join("memory-scan/mission.md");
        fs::create_dir_all(mine.parent().unwrap()).unwrap();
        fs::write(&mine, "mine").unwrap();
        sync_builtin_missions(dir.path()).unwrap();
        assert_eq!(sync_builtin_missions(dir.path()).unwrap(), 0);
        assert_eq!(fs::read_to_string(&mine).unwrap(), "mine");
        assert!(!mine.with_extension("md.prev").exists());
    }
}
