use anyhow::Result;
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

use super::download::{default_http_client, download_to_temp};

#[derive(Serialize)]
struct SkillInstallRequest {
    url: String,
    skill: String,
    #[serde(rename = "ref")]
    git_ref: String,
    content: Option<String>,
    installer: String,
    installer_version: String,
    timestamp: String,
}

pub async fn handle_skills_add(
    repo_url: String,
    skill_name: String,
    git_ref: String,
    force: bool,
    registry_url: String,
    api_key: Option<String>,
    no_record: bool,
) -> Result<()> {
    handle_skills_add_impl(
        repo_url,
        skill_name,
        git_ref,
        force,
        registry_url,
        api_key,
        no_record,
        true,
    )
    .await
}

async fn handle_skills_add_impl(
    repo_url: String,
    skill_name: String,
    git_ref: String,
    force: bool,
    registry_url: String,
    api_key: Option<String>,
    no_record: bool,
    allow_fallback: bool,
) -> Result<()> {
    let mut current_repo_url = repo_url;
    let mut current_skill_name = skill_name;
    let mut current_git_ref = git_ref;
    let mut can_fallback = allow_fallback;

    loop {
        let normalized_url = normalize_github_url(&current_repo_url)?;
        let (owner, repo) = parse_github_url(&normalized_url)?;

        println!(
            "{}",
            format!(
                "🔧 Installing skill: {} from {} (ref: {})",
                current_skill_name, normalized_url, current_git_ref
            )
            .cyan()
        );

        // 1. Determine target directory
        let local_claude = Path::new(".claude");
        let target_dir = if local_claude.exists() && local_claude.is_dir() {
            local_claude.join("skills").join(&current_skill_name)
        } else {
            dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
                .join(".claude")
                .join("skills")
                .join(&current_skill_name)
        };

        println!(
            "{}",
            format!("📂 Target directory: {}", target_dir.display()).dimmed()
        );

        // 2. Ensure target directory is safe to write
        if target_dir.exists() {
            if force {
                fs::remove_dir_all(&target_dir)?;
            } else {
                anyhow::bail!(
                    "Skill '{}' is already installed at {}. Re-run with --force to overwrite.",
                    current_skill_name,
                    target_dir.display()
                );
            }
        }

        // 3. Download zipball
        let zip_url = build_github_zip_url(&owner, &repo, &current_git_ref);
        println!("{}", "⬇️  Downloading from GitHub...".dimmed());

        let client = default_http_client()?;
        let temp_zip = download_to_temp(&client, &zip_url, None).await?;

        // 4. Extract selectively
        println!("{}", "📦 Extracting skill...".dimmed());
        let skill_content = match extract_skill_from_zip(&temp_zip, &current_skill_name, &target_dir) {
            Ok(content) => content,
            Err(err) => {
                let _ = fs::remove_file(&temp_zip);

                if can_fallback
                    && is_skill_not_found_error(&err)
                    && is_linggen_skills_repo(&normalized_url, &owner, &repo)
                {
                    if let Some(fallback) = search_skills_sh(&current_skill_name).await? {
                        if fallback.top_source == "linggen/skills" {
                            return Err(err);
                        }

                        let fallback_repo = format!("https://github.com/{}", fallback.top_source);
                        println!(
                            "{}",
                            format!(
                                "🔎 Not found in linggen/skills. Using skills.sh result '{}' from {}.",
                                fallback.id, fallback.top_source
                            )
                            .yellow()
                        );

                        current_repo_url = fallback_repo;
                        current_skill_name = fallback.id;
                        current_git_ref = "main".to_string();
                        can_fallback = false;
                        continue;
                    }
                }

                return Err(err);
            }
        };

        println!(
            "{}",
            format!("✅ Skill installed to {}", target_dir.display()).green()
        );

        // 4. Record install in registry
        if !no_record {
            if let Some(key) = api_key {
                if let Ok(last_install) = get_last_install_time(&current_skill_name) {
                    let elapsed = chrono::Utc::now().signed_duration_since(last_install);
                    if elapsed < chrono::Duration::minutes(5) {
                        let wait_mins = 5 - elapsed.num_minutes();
                        println!(
                            "{}",
                            format!(
                                "ℹ️  Skill installed locally. Registry update skipped (cooldown: wait {} more minutes).",
                                wait_mins
                            )
                            .yellow()
                        );
                        return Ok(());
                    }
                }

                println!("{}", "📝 Recording install in registry...".dimmed());
                match record_install(
                    &client,
                    &registry_url,
                    &key,
                    &normalized_url,
                    &current_skill_name,
                    &current_git_ref,
                    skill_content,
                )
                .await
                {
                    Ok(counted) => {
                        if counted {
                            println!("{}", "✨ Install recorded and counted!".green());
                            let _ = save_install_time(&current_skill_name);
                        } else {
                            println!(
                                "{}",
                                "ℹ️  Install recorded (already counted recently).".yellow()
                            );
                        }
                    }
                    Err(e) => {
                        println!(
                            "{}",
                            format!("⚠️  Failed to record install: {}", e).yellow()
                        );
                    }
                }
            } else {
                println!(
                    "{}",
                    "⚠️  No API_KEY provided; skipping registry recording. Set API_KEY.".yellow()
                );
            }
        }

        // Cleanup temp zip
        let _ = fs::remove_file(temp_zip);

        return Ok(());
    }
}

pub async fn handle_skills_init(
    ai: Option<String>,
    repo_url: String,
    git_ref: String,
    local: bool,
    force: bool,
    skills: Vec<String>,
) -> Result<()> {
    let ai_choice = resolve_ai_choice(ai)?;
    let normalized_url = normalize_github_url(&repo_url)?;
    let (owner, repo) = parse_github_url(&normalized_url)?;

    let base_skills_dir = resolve_skills_dir(ai_choice, local)?;
    fs::create_dir_all(&base_skills_dir)?;

    println!(
        "{}",
        format!(
            "🔧 Initializing Linggen skills for {} from {} (ref: {})",
            ai_choice.label(),
            normalized_url,
            git_ref
        )
        .cyan()
    );
    println!(
        "{}",
        format!("📂 Skills directory: {}", base_skills_dir.display()).dimmed()
    );

    let zip_url = build_github_zip_url(&owner, &repo, &git_ref);
    println!("{}", "⬇️  Downloading skills repo from GitHub...".dimmed());

    let client = default_http_client()?;
    let temp_zip = download_to_temp(&client, &zip_url, None).await?;

    println!("{}", "📦 Detecting skills...".dimmed());
    let detected = detect_skills_in_zip(&temp_zip)?;
    if detected.is_empty() {
        anyhow::bail!(
            "No skills found in repository {} at ref {} (expected SKILL.md).",
            normalized_url,
            git_ref
        );
    }

    let requested: BTreeSet<String> = skills.into_iter().collect();
    let to_install: BTreeMap<String, String> = if requested.is_empty() {
        detected
    } else {
        let mut filtered = BTreeMap::new();
        for skill in &requested {
            if let Some(prefix) = detected.get(skill) {
                filtered.insert(skill.clone(), prefix.clone());
            }
        }
        let missing: Vec<String> = requested
            .into_iter()
            .filter(|s| !filtered.contains_key(s))
            .collect();
        if !missing.is_empty() {
            anyhow::bail!(
                "Requested skill(s) not found in repo: {}",
                missing.join(", ")
            );
        }
        filtered
    };

    println!(
        "{}",
        format!(
            "🧩 Installing {} skill(s): {}",
            to_install.len(),
            to_install.keys().cloned().collect::<Vec<_>>().join(", ")
        )
        .dimmed()
    );

    let mut installed = 0usize;
    for (skill_name, zip_prefix) in to_install {
        let target_dir = base_skills_dir.join(&skill_name);
        if target_dir.exists() {
            if force || skill_name == "linggen" {
                fs::remove_dir_all(&target_dir)?;
            } else {
                println!(
                    "{}",
                    format!(
                        "ℹ️  Skill '{}' already exists at {}; skipping (use --force to overwrite).",
                        skill_name,
                        target_dir.display()
                    )
                    .yellow()
                );
                continue;
            }
        }

        extract_dir_from_zip(&temp_zip, &zip_prefix, &target_dir)?;
        println!(
            "{}",
            format!("✅ Installed skill '{}' to {}", skill_name, target_dir.display()).green()
        );
        installed += 1;
    }

    let _ = fs::remove_file(temp_zip);

    if installed == 0 {
        println!(
            "{}",
            "ℹ️  No skills were installed (everything already present).".yellow()
        );
    } else {
        println!(
            "{}",
            format!(
                "✨ Done. {} skill(s) installed into {}",
                installed,
                base_skills_dir.display()
            )
            .green()
        );
    }

    Ok(())
}

#[derive(Copy, Clone)]
enum AiChoice {
    Claude,
    Codex,
}

impl AiChoice {
    fn label(self) -> &'static str {
        match self {
            AiChoice::Claude => "Claude",
            AiChoice::Codex => "Codex",
        }
    }
}

fn resolve_ai_choice(ai: Option<String>) -> Result<AiChoice> {
    match ai.as_deref().map(|s| s.trim().to_lowercase()) {
        Some(ref s) if s == "claude" => Ok(AiChoice::Claude),
        Some(ref s) if s == "codex" => Ok(AiChoice::Codex),
        Some(other) => anyhow::bail!("Unsupported AI provider '{}'. Use 'claude' or 'codex'.", other),
        None => prompt_ai_choice(),
    }
}

fn prompt_ai_choice() -> Result<AiChoice> {
    use std::io::{self, IsTerminal};

    if !io::stdin().is_terminal() {
        println!("ℹ️  No --ai provided and stdin is not interactive; defaulting to Claude.");
        return Ok(AiChoice::Claude);
    }

    println!("Select AI:");
    println!("1) Claude (default)");
    println!("2) Codex");
    print!("Enter choice [1-2]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let choice = input.trim().to_lowercase();

    if choice.is_empty() || choice == "1" || choice == "claude" {
        Ok(AiChoice::Claude)
    } else if choice == "2" || choice == "codex" {
        Ok(AiChoice::Codex)
    } else {
        anyhow::bail!("Invalid choice '{}'. Use 1 (Claude) or 2 (Codex).", input.trim())
    }
}

fn resolve_skills_dir(ai: AiChoice, local: bool) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;
    let path = match ai {
        AiChoice::Claude => {
            if local {
                PathBuf::from(".claude").join("skills")
            } else {
                home.join(".claude").join("skills")
            }
        }
        AiChoice::Codex => {
            if local {
                PathBuf::from(".codex").join("skills")
            } else if let Ok(codex_home) = std::env::var("CODEX_HOME") {
                PathBuf::from(codex_home).join("skills")
            } else {
                home.join(".codex").join("skills")
            }
        }
    };
    Ok(path)
}

fn get_last_install_time(skill_name: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    let state_dir = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("No local data dir"))?
        .join("Linggen")
        .join("skills_state");
    let state_file = state_dir.join(format!("{}.last_install", skill_name));

    let content = fs::read_to_string(state_file)?;
    let dt = chrono::DateTime::parse_from_rfc3339(&content.trim())?.with_timezone(&chrono::Utc);
    Ok(dt)
}

fn save_install_time(skill_name: &str) -> Result<()> {
    let state_dir = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("No local data dir"))?
        .join("Linggen")
        .join("skills_state");
    fs::create_dir_all(&state_dir)?;

    let state_file = state_dir.join(format!("{}.last_install", skill_name));
    fs::write(state_file, chrono::Utc::now().to_rfc3339())?;
    Ok(())
}

fn normalize_github_url(url: &str) -> Result<String> {
    let url = url.trim().trim_end_matches(".git").trim_end_matches('/');

    if url.starts_with("https://github.com/") {
        Ok(url.to_string())
    } else if !url.contains("://") && url.contains('/') {
        // Assume owner/repo shorthand
        Ok(format!("https://github.com/{}", url))
    } else if url.starts_with("git@github.com:") {
        let repo = url.trim_start_matches("git@github.com:");
        Ok(format!("https://github.com/{}", repo))
    } else {
        // For now, only support GitHub. We can expand this later.
        if url.contains("github.com") {
            Ok(url.to_string())
        } else {
            anyhow::bail!("Only GitHub repositories are supported currently: {}", url)
        }
    }
}

fn parse_github_url(url: &str) -> Result<(String, String)> {
    let stripped = url.trim_start_matches("https://github.com/");
    let parts: Vec<&str> = stripped.split('/').collect();
    if parts.len() >= 2 {
        return Ok((parts[0].to_string(), parts[1].to_string()));
    }
    anyhow::bail!("Could not parse GitHub repository from '{}'.", url)
}

fn is_skill_not_found_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("Could not find skill") && msg.contains("SKILL.md")
}

fn is_linggen_skills_repo(normalized_url: &str, owner: &str, repo: &str) -> bool {
    normalized_url == "https://github.com/linggen/skills"
        || (owner == "linggen" && repo == "skills")
}

fn build_github_zip_url(owner: &str, repo: &str, git_ref: &str) -> String {
    if git_ref.starts_with("refs/") {
        format!(
            "https://github.com/{}/{}/archive/{}.zip",
            owner, repo, git_ref
        )
    } else if git_ref.starts_with("heads/") || git_ref.starts_with("tags/") {
        format!(
            "https://github.com/{}/{}/archive/refs/{}.zip",
            owner, repo, git_ref
        )
    } else {
        format!(
            "https://github.com/{}/{}/archive/refs/heads/{}.zip",
            owner, repo, git_ref
        )
    }
}

#[derive(Deserialize)]
struct SkillsShResponse {
    skills: Vec<SkillsShSkill>,
}

#[derive(Clone, Deserialize)]
struct SkillsShSkill {
    id: String,
    #[serde(rename = "topSource")]
    top_source: String,
}

async fn search_skills_sh(query: &str) -> Result<Option<SkillsShSkill>> {
    let client = default_http_client()?;
    let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    let url = format!("https://skills.sh/api/search?q={}&limit=50", encoded);

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let payload: SkillsShResponse = resp.json().await?;

    if payload.skills.is_empty() {
        return Ok(None);
    }

    if let Some(found) = payload.skills.iter().find(|s| s.id == query) {
        return Ok(Some(found.clone()));
    }

    Ok(payload.skills.into_iter().next())
}

fn extract_skill_from_zip(
    zip_path: &Path,
    skill_name: &str,
    target_dir: &Path,
) -> Result<Option<String>> {
    let file = fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;

    // 1. Find the skill directory in the zip
    // GitHub zips have a root folder like "repo-name-branch-name/"
    let mut skill_root_in_zip = None;
    let mut skill_md_path_in_zip = None;
    let mut candidates: Vec<(String, PathBuf, String)> = Vec::new(); // (dir_name, root, SKILL.md path)

    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = file.name();

        // Look for SKILL.md or skill.md inside a directory named skill_name
        if !(name.ends_with("/SKILL.md") || name.ends_with("/skill.md")) {
            continue;
        }

        let path = Path::new(name);
        let Some(parent) = path.parent() else {
            continue;
        };

        let dir_name = parent
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        candidates.push((dir_name.clone(), parent.to_path_buf(), name.to_string()));

        if name.contains(&format!("/{}/", skill_name)) {
            skill_root_in_zip = Some(parent.to_path_buf());
            skill_md_path_in_zip = Some(name.to_string());
            break;
        }
    }

    if skill_root_in_zip.is_none() && !candidates.is_empty() {
        // Fallback: allow prefixed skill ids like "vendor-skill" to map to "skill" folder when unambiguous.
        let mut matches: Vec<&(String, PathBuf, String)> = candidates
            .iter()
            .filter(|(dir_name, _, _)| {
                !dir_name.is_empty() && skill_name.ends_with(&format!("-{}", dir_name))
            })
            .collect();

        matches.sort_by(|a, b| a.0.cmp(&b.0));
        matches.dedup_by(|a, b| a.0 == b.0);

        if matches.len() == 1 {
            let (_, root, md_path) = matches[0];
            skill_root_in_zip = Some(root.clone());
            skill_md_path_in_zip = Some(md_path.clone());
        }
    }

    let skill_root = skill_root_in_zip.ok_or_else(|| {
        let available: BTreeSet<String> = candidates
            .iter()
            .map(|(dir, _, _)| dir.clone())
            .filter(|s| !s.is_empty())
            .collect();
        let shown: Vec<String> = available.iter().take(10).cloned().collect();

        let mut msg = format!(
            "Could not find skill '{}' in the repository. Make sure it contains a SKILL.md file.",
            skill_name
        );
        if !shown.is_empty() {
            msg.push_str(&format!(
                " Available skills (by folder): {}{}",
                shown.join(", "),
                if available.len() > shown.len() { ", ..." } else { "" }
            ));
        }
        anyhow::anyhow!(msg)
    })?;

    // Read SKILL.md content for registry
    let mut skill_md_content = None;
    if let Some(path) = skill_md_path_in_zip {
        if let Ok(mut file) = archive.by_name(&path) {
            let mut content = String::new();
            if file.read_to_string(&mut content).is_ok() {
                skill_md_content = Some(content);
            }
        }
    }

    // 2. Extract files
    fs::create_dir_all(target_dir)?;

    let skill_root_str = skill_root.to_str().unwrap();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();

        if name.starts_with(skill_root_str) && !file.is_dir() {
            let rel_path = &name[skill_root_str.len()..].trim_start_matches('/');
            if rel_path.is_empty() {
                continue;
            }

            // Security check
            if rel_path.contains("..") || rel_path.starts_with('/') {
                continue;
            }

            let dest_path = target_dir.join(rel_path);
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let mut outfile = fs::File::create(&dest_path)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }

    Ok(skill_md_content)
}

fn detect_skills_in_zip(zip_path: &Path) -> Result<BTreeMap<String, String>> {
    let file = fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;

    // Map: skill_name -> zip path prefix for that skill directory
    let mut skills: BTreeMap<String, String> = BTreeMap::new();

    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = file.name();

        if !(name.ends_with("/SKILL.md") || name.ends_with("/skill.md")) {
            continue;
        }

        let path = Path::new(name);
        let Some(parent) = path.parent() else {
            continue;
        };

        let Some(skill_name_os) = parent.file_name() else {
            continue;
        };
        let Some(skill_name) = skill_name_os.to_str() else {
            continue;
        };

        // Only treat direct folders (i.e. .../<skill>/SKILL.md) as skills
        let Some(prefix) = parent.to_str() else {
            continue;
        };

        // Normalize to a prefix that matches zip file names (GitHub zips use forward slashes)
        let mut prefix = prefix.replace('\\', "/");
        if !prefix.ends_with('/') {
            prefix.push('/');
        }

        skills.entry(skill_name.to_string()).or_insert(prefix);
    }

    Ok(skills)
}

fn extract_dir_from_zip(zip_path: &Path, zip_prefix: &str, target_dir: &Path) -> Result<()> {
    let file = fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;

    fs::create_dir_all(target_dir)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        if file.is_dir() {
            continue;
        }

        let name = file.name().to_string();
        if !name.starts_with(zip_prefix) {
            continue;
        }

        let rel_path = &name[zip_prefix.len()..].trim_start_matches('/');
        if rel_path.is_empty() {
            continue;
        }

        // Security check
        if rel_path.contains("..") || rel_path.starts_with('/') {
            continue;
        }

        let dest_path = target_dir.join(rel_path);
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut outfile = fs::File::create(&dest_path)?;
        std::io::copy(&mut file, &mut outfile)?;
    }

    Ok(())
}

async fn record_install(
    client: &reqwest::Client,
    registry_url: &str,
    api_key: &str,
    url: &str,
    skill: &str,
    git_ref: &str,
    content: Option<String>,
) -> Result<bool> {
    let registry_endpoint = format!("{}/skills/install", registry_url.trim_end_matches('/'));

    let payload = SkillInstallRequest {
        url: url.to_string(),
        skill: skill.to_string(),
        git_ref: git_ref.to_string(),
        content,
        installer: "linggen-cli".to_string(),
        installer_version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    let resp = client
        .post(&registry_endpoint)
        .header("X-API-Key", api_key)
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let err_text = resp.text().await.unwrap_or_else(|_| "Unknown error".into());
        anyhow::bail!("Registry returned error: {}", err_text);
    }

    #[derive(Deserialize)]
    struct RegistryResponse {
        counted: bool,
    }

    let result: RegistryResponse = resp.json().await?;
    Ok(result.counted)
}
