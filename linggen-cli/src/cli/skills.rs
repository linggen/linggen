use anyhow::Result;
use colored::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::Path;
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
    _force: bool,
    registry_url: String,
    api_key: Option<String>,
    no_record: bool,
) -> Result<()> {
    let normalized_url = normalize_github_url(&repo_url)?;
    let (owner, repo) = parse_github_url(&normalized_url)?;

    println!(
        "{}",
        format!(
            "🔧 Installing skill: {} from {} (ref: {})",
            skill_name, normalized_url, git_ref
        )
        .cyan()
    );

    // 1. Determine target directory
    let local_claude = Path::new(".claude");
    let target_dir = if local_claude.exists() && local_claude.is_dir() {
        local_claude.join("skills").join(&skill_name)
    } else {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
            .join(".claude")
            .join("skills")
            .join(&skill_name)
    };

    println!(
        "{}",
        format!("📂 Target directory: {}", target_dir.display()).dimmed()
    );

    // 2. Download zipball
    let zip_url = format!(
        "https://codeload.github.com/{}/{}/zip/{}",
        owner, repo, git_ref
    );
    println!("{}", format!("⬇️  Downloading from GitHub...").dimmed());

    let client = default_http_client()?;
    let temp_zip = download_to_temp(&client, &zip_url, None).await?;

    // 3. Extract selectively
    println!("{}", format!("📦 Extracting skill...").dimmed());
    let skill_content = extract_skill_from_zip(&temp_zip, &skill_name, &target_dir)?;

    println!(
        "{}",
        format!("✅ Skill installed to {}", target_dir.display()).green()
    );

    // 4. Record install in registry
    if !no_record {
        if let Some(key) = api_key {
            // Check local cooldown
            if let Ok(last_install) = get_last_install_time(&skill_name) {
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

            println!(
                "{}",
                format!("📝 Recording install in registry...").dimmed()
            );
            match record_install(
                &client,
                &registry_url,
                &key,
                &normalized_url,
                &skill_name,
                &git_ref,
                skill_content,
            )
            .await
            {
                Ok(counted) => {
                    if counted {
                        println!("{}", "✨ Install recorded and counted!".green());
                        let _ = save_install_time(&skill_name);
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

    Ok(())
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

    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = file.name();

        // Look for SKILL.md or skill.md inside a directory named skill_name
        if (name.ends_with("/SKILL.md") || name.ends_with("/skill.md"))
            && name.contains(&format!("/{}/", skill_name))
        {
            let path = Path::new(name);
            if let Some(parent) = path.parent() {
                skill_root_in_zip = Some(parent.to_path_buf());
                skill_md_path_in_zip = Some(name.to_string());
                break;
            }
        }
    }

    let skill_root = skill_root_in_zip.ok_or_else(|| {
        anyhow::anyhow!(
            "Could not find skill '{}' in the repository. Make sure it contains a SKILL.md file.",
            skill_name
        )
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
    if target_dir.exists() {
        fs::remove_dir_all(target_dir)?;
    }
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
