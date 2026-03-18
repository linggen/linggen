use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;
use zip::ZipArchive;

const SKILLS_SH_API: &str = "https://skills.sh/api/search";
const CLAWHUB_API: &str = "https://clawhub.ai/api/v1";
const DEFAULT_SKILLS_REPO: &str = "https://github.com/linggen/skills";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSkill {
    #[serde(default)]
    pub skill_id: String,
    #[serde(alias = "skill")]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub install_count: u64,
    #[serde(default, alias = "ref")]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub source_registry: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkillScope {
    Project,
    Global,
}

impl Default for SkillScope {
    fn default() -> Self {
        Self::Global
    }
}

// ---------------------------------------------------------------------------
// ClawHub types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawHubSearchResponse {
    pub results: Vec<ClawHubSearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawHubSearchResult {
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default, alias = "displayName")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, alias = "updatedAt")]
    pub updated_at: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawHubScanResult {
    #[serde(default)]
    pub skill: Option<ClawHubScanSkill>,
    #[serde(default)]
    pub moderation: Option<ClawHubModeration>,
    #[serde(default)]
    pub security: Option<ClawHubSecurity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawHubScanSkill {
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default, alias = "displayName")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawHubModeration {
    #[serde(default, alias = "isPendingScan")]
    pub is_pending_scan: bool,
    #[serde(default, alias = "isMalwareBlocked")]
    pub is_malware_blocked: bool,
    #[serde(default, alias = "isSuspicious")]
    pub is_suspicious: bool,
    #[serde(default, alias = "isHiddenByMod")]
    pub is_hidden_by_mod: bool,
    #[serde(default, alias = "isRemoved")]
    pub is_removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawHubSecurity {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, alias = "hasWarnings")]
    pub has_warnings: bool,
    #[serde(default, alias = "hasScanResult")]
    pub has_scan_result: bool,
}

// ---------------------------------------------------------------------------
// Skill audit report (source-agnostic)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAuditReport {
    /// Total number of files in the skill
    pub file_count: usize,
    /// List of all file paths in the ZIP
    pub files: Vec<String>,
    /// Files flagged as potentially risky
    pub flagged_files: Vec<AuditFlag>,
    /// Risky patterns found in text files
    pub risky_patterns: Vec<AuditFlag>,
    /// ClawHub scan result (only if source is clawhub)
    #[serde(default)]
    pub clawhub_scan: Option<ClawHubScanResult>,
    /// Overall risk level: "clean", "warning", "danger"
    pub risk_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditFlag {
    pub file: String,
    pub reason: String,
    pub severity: String, // "info", "warning", "danger"
}

// ---------------------------------------------------------------------------
// Registry response envelope
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// HTTP client / community search
// ---------------------------------------------------------------------------

pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("linggen")
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")
}

/// Search community skills across all registries (skills.sh + ClawHub).
pub async fn search_community(query: &str) -> Result<Vec<MarketplaceSkill>> {
    let (sh_result, ch_result) = tokio::join!(
        search_skills_sh_community(query),
        search_clawhub(query),
    );

    let mut results = sh_result.unwrap_or_default();
    let mut seen: HashSet<String> = results.iter().map(|s| s.skill_id.to_lowercase()).collect();

    // Append ClawHub results, deduplicating by skill_id
    for skill in ch_result.unwrap_or_default() {
        let key = skill.skill_id.to_lowercase();
        if seen.insert(key) {
            results.push(skill);
        }
    }

    Ok(results)
}

/// Search skills.sh only.
async fn search_skills_sh_community(query: &str) -> Result<Vec<MarketplaceSkill>> {
    let client = http_client()?;
    let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    let url = format!("{}?q={}&limit=50", SKILLS_SH_API, encoded);

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }

    let payload: SkillsShResponse = resp.json().await?;
    let results = payload
        .skills
        .into_iter()
        .map(|s| {
            let skill_name = s.skill_id.clone().or(s.name.clone()).unwrap_or_else(|| s.id.clone());
            let source = s.source.as_deref().unwrap_or("");
            MarketplaceSkill {
                skill_id: s.id.clone(),
                name: skill_name,
                url: if source.is_empty() { String::new() } else { format!("https://github.com/{}", source) },
                description: None,
                install_count: s.installs.unwrap_or(0),
                git_ref: Some("main".to_string()),
                content: None,
                updated_at: None,
                source_registry: Some("skills.sh".into()),
            }
        })
        .collect();

    Ok(results)
}

/// Search skills on ClawHub.
pub async fn search_clawhub(query: &str) -> Result<Vec<MarketplaceSkill>> {
    let client = http_client()?;
    let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    let url = format!("{}/search?q={}&limit=30&nonSuspiciousOnly=true", CLAWHUB_API, encoded);

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }

    let payload: ClawHubSearchResponse = resp.json().await?;
    let results = payload
        .results
        .into_iter()
        .filter_map(|r| {
            let slug = r.slug?;
            Some(MarketplaceSkill {
                skill_id: slug.clone(),
                name: r.display_name.unwrap_or_else(|| slug.clone()),
                url: format!("https://clawhub.ai/skills/{}", slug),
                description: r.summary,
                install_count: 0,
                git_ref: None,
                content: None,
                updated_at: r.updated_at.and_then(|ts| {
                    chrono::DateTime::from_timestamp_millis(ts.round() as i64)
                        .map(|dt| dt.to_rfc3339())
                }),
                source_registry: Some("clawhub".into()),
            })
        })
        .collect();

    Ok(results)
}

/// Validate that a slug contains only safe characters (alphanumeric, hyphens, underscores).
fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        anyhow::bail!("Slug cannot be empty");
    }
    if !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("Invalid slug '{}': only alphanumeric, hyphens, and underscores allowed", slug);
    }
    Ok(())
}

/// Fetch security scan info from ClawHub for a skill.
pub async fn fetch_clawhub_scan(slug: &str) -> Result<ClawHubScanResult> {
    validate_slug(slug)?;
    let client = http_client()?;
    let encoded_slug = url::form_urlencoded::byte_serialize(slug.as_bytes()).collect::<String>();
    let url = format!("{}/skills/{}/scan", CLAWHUB_API, encoded_slug);
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("ClawHub scan request failed: HTTP {}", resp.status());
    }
    let scan: ClawHubScanResult = resp.json().await?;
    Ok(scan)
}

// ---------------------------------------------------------------------------
// Install / delete
// ---------------------------------------------------------------------------

pub async fn install_skill(
    name: &str,
    repo_url: Option<&str>,
    git_ref: Option<&str>,
    target_dir: &Path,
    force: bool,
    source_registry: Option<&str>,
) -> Result<String> {
    // Route to ClawHub if source is clawhub
    if source_registry == Some("clawhub") {
        return install_from_clawhub(name, None, target_dir, force).await;
    }

    // Existing GitHub/skills.sh logic below (unchanged)
    let repo_url = repo_url.unwrap_or(DEFAULT_SKILLS_REPO);
    let git_ref = git_ref.unwrap_or("main");

    let normalized = normalize_github_url(repo_url)?;
    let (owner, repo) = parse_github_url(&normalized)?;

    // Check existing
    if target_dir.exists() {
        if force {
            fs::remove_dir_all(target_dir)?;
        } else {
            anyhow::bail!(
                "Skill '{}' already installed at {}. Use force to overwrite.",
                name,
                target_dir.display()
            );
        }
    }

    // Download ZIP
    let zip_url = build_github_zip_url(&owner, &repo, git_ref);
    let client = http_client()?;
    let temp_zip = download_to_temp(&client, &zip_url).await?;

    // Extract
    let result = extract_skill_from_zip(&temp_zip, name, &repo, target_dir);
    let _ = fs::remove_file(&temp_zip);

    match result {
        Ok(_) => Ok(format!(
            "Skill '{}' installed to {}",
            name,
            target_dir.display()
        )),
        Err(e) => {
            // If not found in default repo, try skills.sh fallback
            if e.to_string().contains("Could not find skill") && is_default_repo(&normalized) {
                if let Some(fallback) = search_skills_sh(name).await? {
                    let fallback_source = fallback.source.as_deref().unwrap_or("");
                    if fallback_source != "linggen/skills" {
                        let fallback_repo =
                            format!("https://github.com/{}", fallback_source);
                        // Recompute target_dir using fallback id if it differs from original name.
                        let fallback_target = if fallback.id != name {
                            target_dir.parent().unwrap_or(target_dir).join(&fallback.id)
                        } else {
                            target_dir.to_path_buf()
                        };
                        return install_skill_inner(
                            &fallback.id,
                            &fallback_repo,
                            "main",
                            &fallback_target,
                        )
                        .await;
                    }
                }
            }
            Err(e)
        }
    }
}

async fn install_skill_inner(
    name: &str,
    repo_url: &str,
    git_ref: &str,
    target_dir: &Path,
) -> Result<String> {
    let normalized = normalize_github_url(repo_url)?;
    let (owner, repo) = parse_github_url(&normalized)?;
    let zip_url = build_github_zip_url(&owner, &repo, git_ref);
    let client = http_client()?;
    let temp_zip = download_to_temp(&client, &zip_url).await?;

    let result = extract_skill_from_zip(&temp_zip, name, &repo, target_dir);
    let _ = fs::remove_file(&temp_zip);

    result?;
    Ok(format!(
        "Skill '{}' installed to {}",
        name,
        target_dir.display()
    ))
}

/// Install a skill directly from ClawHub.
pub async fn install_from_clawhub(
    slug: &str,
    version: Option<&str>,
    target_dir: &Path,
    force: bool,
) -> Result<String> {
    // Check existing
    if target_dir.exists() {
        if force {
            fs::remove_dir_all(target_dir)?;
        } else {
            anyhow::bail!(
                "Skill '{}' already installed at {}. Use force to overwrite.",
                slug,
                target_dir.display()
            );
        }
    }

    // Build download URL (encode params to prevent injection)
    validate_slug(slug)?;
    let encoded_slug = url::form_urlencoded::byte_serialize(slug.as_bytes()).collect::<String>();
    let mut download_url = format!("{}/download?slug={}", CLAWHUB_API, encoded_slug);
    if let Some(v) = version {
        let encoded_v = url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>();
        download_url.push_str(&format!("&version={}", encoded_v));
    }

    let client = http_client()?;
    let temp_zip = download_to_temp(&client, &download_url).await?;

    // Extract to a temp directory first, then move only the target skill to target_dir.
    // This prevents stray dirs from multi-skill ZIPs polluting the parent.
    let temp_dir = tempfile::tempdir().context("Failed to create temp dir for extraction")?;
    let result = extract_all_skills_from_zip(&temp_zip, temp_dir.path());
    let _ = fs::remove_file(&temp_zip);

    let installed = result?;
    if installed.is_empty() {
        anyhow::bail!("No skill found in ClawHub ZIP for '{}'", slug);
    }

    // Pick the first extracted skill (ClawHub ZIPs typically contain exactly one)
    let extracted_name = &installed[0];
    let extracted_dir = temp_dir.path().join(extracted_name);

    fs::create_dir_all(target_dir)?;
    // Move contents from extracted dir to target_dir
    copy_dir_all(&extracted_dir, target_dir)?;

    Ok(format!(
        "Skill '{}' installed from ClawHub to {}",
        slug,
        target_dir.display()
    ))
}

pub fn delete_skill(name: &str, target_dir: &Path) -> Result<String> {
    if !target_dir.exists() {
        anyhow::bail!("Skill '{}' not found at {}", name, target_dir.display());
    }
    fs::remove_dir_all(target_dir)?;
    Ok(format!("Skill '{}' deleted from {}", name, target_dir.display()))
}

pub fn skill_target_dir(name: &str, scope: SkillScope, project_root: Option<&Path>) -> Result<PathBuf> {
    match scope {
        SkillScope::Project => {
            let root = project_root
                .ok_or_else(|| anyhow::anyhow!("Project root required for project-scoped install"))?;
            Ok(root.join(".linggen/skills").join(name))
        }
        SkillScope::Global => {
            Ok(crate::paths::global_skills_dir().join(name))
        }
    }
}

/// Resolve the actual filesystem directory for a skill based on its loaded source.
/// Unlike `skill_target_dir` (which only knows Project/Global), this handles
/// Compat sources (Claude, Codex) correctly.
pub fn skill_dir_for_source(
    name: &str,
    source: &super::SkillSource,
    project_root: Option<&Path>,
) -> Result<PathBuf> {
    match source {
        super::SkillSource::Global => Ok(crate::paths::global_skills_dir().join(name)),
        super::SkillSource::Project => {
            let root = project_root
                .ok_or_else(|| anyhow::anyhow!("Project root required for project-scoped skill"))?;
            // Check all project skill dirs to find which one actually contains this skill.
            for dir_name in &[".linggen/skills", ".claude/skills", ".codex/skills"] {
                let candidate = root.join(dir_name).join(name);
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
            // Fallback to the canonical location.
            Ok(root.join(".linggen/skills").join(name))
        }
        super::SkillSource::Compat { label } => {
            // Find the matching compat dir by label.
            for (dir, compat_label) in crate::paths::compat_skills_dirs() {
                if compat_label == label.as_str() {
                    return Ok(dir.join(name));
                }
            }
            anyhow::bail!("Unknown compat source '{}'", label)
        }
    }
}

/// Move a project skill to the global `~/.linggen/skills/` directory.
pub fn move_skill_to_global(
    name: &str,
    source: &super::SkillSource,
    project_root: Option<&Path>,
) -> Result<String> {
    let src_dir = skill_dir_for_source(name, source, project_root)?;
    if !src_dir.exists() {
        anyhow::bail!("Skill '{}' not found at {}", name, src_dir.display());
    }
    let dest_dir = crate::paths::global_skills_dir().join(name);
    if dest_dir.exists() {
        anyhow::bail!(
            "Skill '{}' already exists in global at {}",
            name,
            dest_dir.display()
        );
    }
    copy_dir_all(&src_dir, &dest_dir)?;
    fs::remove_dir_all(&src_dir)?;
    Ok(format!("Moved '{}' to {}", name, dest_dir.display()))
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// GitHub URL helpers
// ---------------------------------------------------------------------------

pub fn normalize_github_url(url: &str) -> Result<String> {
    let url = url.trim().trim_end_matches(".git").trim_end_matches('/');

    if url.starts_with("https://github.com/") {
        Ok(url.to_string())
    } else if url.starts_with("git@github.com:") {
        let repo = url.trim_start_matches("git@github.com:");
        Ok(format!("https://github.com/{}", repo))
    } else if !url.contains("://") && url.contains('/') {
        Ok(format!("https://github.com/{}", url))
    } else if url.contains("github.com") {
        Ok(url.to_string())
    } else {
        anyhow::bail!("Only GitHub repositories are supported: {}", url)
    }
}

pub fn parse_github_url(url: &str) -> Result<(String, String)> {
    let stripped = url.trim_start_matches("https://github.com/");
    let parts: Vec<&str> = stripped.split('/').collect();
    if parts.len() >= 2 {
        return Ok((parts[0].to_string(), parts[1].to_string()));
    }
    anyhow::bail!("Could not parse GitHub repository from '{}'", url)
}

pub(crate) fn build_github_zip_url(owner: &str, repo: &str, git_ref: &str) -> String {
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

fn is_default_repo(normalized_url: &str) -> bool {
    normalized_url == DEFAULT_SKILLS_REPO
}

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

pub(crate) async fn download_to_temp(client: &reqwest::Client, url: &str) -> Result<PathBuf> {
    let tmp = tempfile::NamedTempFile::new().context("Failed to create temp file")?;
    // Persist immediately so the path is stable; caller is responsible for cleanup.
    let tmp_path = tmp.into_temp_path();

    let max_attempts = 3;
    let mut last_error = None;

    for attempt in 0..max_attempts {
        match client.get(url).send().await {
            Ok(r) if r.status().is_success() => {
                let bytes = r.bytes().await.context("Failed to read response")?;
                fs::write(&tmp_path, &bytes).context("Failed to write temp file")?;
                let kept = tmp_path.keep().map_err(|e| anyhow::anyhow!("tempfile keep error: {}", e))?;
                return Ok(kept);
            }
            Ok(r) if attempt < max_attempts - 1 && is_retryable_status(r.status()) => {
                let delay = Duration::from_secs(1 << attempt);
                tracing::warn!(
                    status = %r.status(),
                    attempt = attempt + 1,
                    "Download returned retryable status, retrying..."
                );
                tokio::time::sleep(delay).await;
            }
            Ok(r) => {
                last_error = Some(format!("HTTP {}", r.status()));
                break;
            }
            Err(e) if attempt < max_attempts - 1 => {
                let delay = Duration::from_secs(1 << attempt);
                tracing::warn!(err = %e, attempt = attempt + 1, "Network error, retrying...");
                tokio::time::sleep(delay).await;
                last_error = Some(e.to_string());
            }
            Err(e) => {
                last_error = Some(e.to_string());
                break;
            }
        }
    }

    anyhow::bail!(
        "Download failed after {} attempts: {} - {}",
        max_attempts,
        url,
        last_error.unwrap_or_else(|| "Unknown".into())
    )
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::SERVICE_UNAVAILABLE
        || status == reqwest::StatusCode::GATEWAY_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

// ---------------------------------------------------------------------------
// ZIP extraction
// ---------------------------------------------------------------------------

fn extract_skill_from_zip(
    zip_path: &Path,
    skill_name: &str,
    repo_name: &str,
    target_dir: &Path,
) -> Result<()> {
    let file = fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;

    let mut skill_root_in_zip = None;
    let mut candidates: Vec<(String, PathBuf, String)> = Vec::new();
    let mut root_skill_md_candidate: Option<(String, PathBuf, String)> = None;

    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = file.name().to_string();

        if !(name.ends_with("/SKILL.md") || name.ends_with("/skill.md")) {
            continue;
        }

        let path = Path::new(&name);
        let Some(parent) = path.parent() else {
            continue;
        };

        let dir_name = parent
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        candidates.push((dir_name.clone(), parent.to_path_buf(), name.clone()));

        let normal_count = path
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count();
        if normal_count == 2 {
            root_skill_md_candidate =
                Some((dir_name.clone(), parent.to_path_buf(), name.clone()));
        }

        // Match directory name against skill name, normalizing hyphens/underscores
        let norm_skill = skill_name.replace('-', "_");
        let norm_dir = dir_name.replace('-', "_");
        if norm_dir == norm_skill || name.contains(&format!("/{}/", skill_name)) {
            skill_root_in_zip = Some(parent.to_path_buf());
            break;
        }
    }

    // Fallback: prefixed skill names
    if skill_root_in_zip.is_none() && !candidates.is_empty() {
        let matches: Vec<&(String, PathBuf, String)> = candidates
            .iter()
            .filter(|(dir_name, _, _)| {
                !dir_name.is_empty() && skill_name.ends_with(&format!("-{}", dir_name))
            })
            .collect();

        if matches.len() == 1 {
            skill_root_in_zip = Some(matches[0].1.clone());
        }
    }

    // Fallback: root SKILL.md
    if skill_root_in_zip.is_none() {
        if let Some((root_dir_name, root, _)) = &root_skill_md_candidate {
            let has_only_root = candidates
                .iter()
                .all(|(dir_name, _, _)| dir_name == root_dir_name);

            if skill_name == repo_name || candidates.len() == 1 || has_only_root {
                skill_root_in_zip = Some(root.clone());
            }
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
                " Available skills: {}{}",
                shown.join(", "),
                if available.len() > shown.len() {
                    ", ..."
                } else {
                    ""
                }
            ));
        }
        anyhow::anyhow!(msg)
    })?;

    // Extract files
    fs::create_dir_all(target_dir)?;
    let skill_root_str = skill_root.to_str().unwrap();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();

        if name.starts_with(skill_root_str) && !file.is_dir() {
            let rel_path = name[skill_root_str.len()..].trim_start_matches('/');
            if !is_safe_zip_path(rel_path, target_dir) {
                continue;
            }

            let dest = target_dir.join(rel_path);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }

            let mut outfile = fs::File::create(&dest)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }

    Ok(())
}

/// Validate a relative path from a ZIP entry is safe to extract under `base_dir`.
fn is_safe_zip_path(rel_path: &str, base_dir: &Path) -> bool {
    if rel_path.is_empty() || rel_path.starts_with('/') {
        return false;
    }
    // Reject any component that is ".."
    let dest = base_dir.join(rel_path);
    let canonical: PathBuf = dest.components().collect();
    let base_canonical: PathBuf = base_dir.components().collect();
    canonical.starts_with(&base_canonical)
}

/// Extract all skills from a ZIP archive into `target_base_dir/<name>/`.
/// Returns the list of installed skill directory names.
pub(crate) fn extract_all_skills_from_zip(
    zip_path: &Path,
    target_base_dir: &Path,
) -> Result<Vec<String>> {
    let file = fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;

    // First pass: find all SKILL.md files and their parent dirs.
    let mut skill_roots: Vec<(String, PathBuf)> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if !(name.ends_with("/SKILL.md") || name.ends_with("/skill.md")) {
            continue;
        }
        let path = Path::new(&name);
        let Some(parent) = path.parent() else {
            continue;
        };
        let dir_name = parent
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if dir_name.is_empty() {
            continue;
        }
        skill_roots.push((dir_name, parent.to_path_buf()));
    }

    // Deduplicate by dir_name (first occurrence wins).
    let mut seen = BTreeSet::new();
    skill_roots.retain(|(name, _)| seen.insert(name.clone()));

    // Second pass: extract files for each skill.
    let mut installed = Vec::new();
    for (dir_name, skill_root) in &skill_roots {
        let skill_root_str = skill_root.to_str().unwrap_or("");
        let target_dir = target_base_dir.join(dir_name);
        fs::create_dir_all(&target_dir)?;

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let entry_name = entry.name().to_string();
            if !entry_name.starts_with(skill_root_str) || entry.is_dir() {
                continue;
            }
            let rel_path = entry_name[skill_root_str.len()..].trim_start_matches('/');
            if !is_safe_zip_path(rel_path, &target_dir) {
                continue;
            }
            let dest = target_dir.join(rel_path);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut outfile)?;
        }

        installed.push(dir_name.clone());
    }

    Ok(installed)
}

// ---------------------------------------------------------------------------
// Skill audit (source-agnostic)
// ---------------------------------------------------------------------------

/// Risky patterns to scan for in skill files (pre-compiled).
struct RiskyPattern {
    re: Regex,
    reason: &'static str,
    severity: &'static str,
}

static RISKY_PATTERNS: LazyLock<Vec<RiskyPattern>> = LazyLock::new(|| {
    let defs: &[(&str, &str, &str)] = &[
        ("curl.*\\|.*sh", "Pipe to shell execution", "danger"),
        ("curl.*\\|.*bash", "Pipe to shell execution", "danger"),
        ("wget.*\\|.*sh", "Pipe to shell execution", "danger"),
        ("eval\\s*\\(", "Dynamic code evaluation", "danger"),
        ("eval\\s+\"", "Dynamic code evaluation", "danger"),
        ("exec\\s*\\(", "Process execution", "warning"),
        ("base64.*decode", "Base64 decode (possible obfuscation)", "warning"),
        ("\\$\\{?GITHUB_TOKEN", "GitHub token access", "warning"),
        ("\\$\\{?ANTHROPIC_API_KEY", "API key access", "warning"),
        ("\\$\\{?OPENAI_API_KEY", "API key access", "warning"),
        ("\\$\\{?AWS_SECRET", "AWS credential access", "danger"),
        ("\\$\\{?SSH_", "SSH credential access", "warning"),
        ("/etc/passwd", "System file access", "danger"),
        ("rm\\s+-rf\\s+/", "Root filesystem deletion", "danger"),
        ("chmod\\s+777", "Overly permissive file permissions", "warning"),
        ("nc\\s+-", "Netcat (possible reverse shell)", "danger"),
        ("\\bsudo\\b", "Sudo usage", "warning"),
    ];
    defs.iter()
        .filter_map(|(pat, reason, severity)| {
            Regex::new(pat).ok().map(|re| RiskyPattern { re, reason, severity })
        })
        .collect()
});

/// Suspicious file extensions that shouldn't normally be in a skill.
const SUSPICIOUS_EXTENSIONS: &[&str] = &[
    ".exe", ".dll", ".so", ".dylib", ".bin", ".dat",
    ".pyc", ".class", ".jar", ".war",
];

/// Audit a downloaded skill ZIP before extraction.
pub fn audit_skill_zip(zip_path: &Path) -> Result<SkillAuditReport> {
    let file = fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;

    let mut files = Vec::new();
    let mut flagged_files = Vec::new();
    let mut risky_patterns = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();

        if entry.is_dir() {
            continue;
        }

        files.push(name.clone());

        // Check for suspicious file extensions
        let lower = name.to_lowercase();
        for ext in SUSPICIOUS_EXTENSIONS {
            if lower.ends_with(ext) {
                flagged_files.push(AuditFlag {
                    file: name.clone(),
                    reason: format!("Suspicious file type: {}", ext),
                    severity: "warning".into(),
                });
            }
        }

        // Check file size (flag files > 1MB)
        if entry.size() > 1_048_576 {
            flagged_files.push(AuditFlag {
                file: name.clone(),
                reason: format!("Large file: {} bytes", entry.size()),
                severity: "info".into(),
            });
        }

        // Scan text files for risky patterns
        let is_text = lower.ends_with(".sh")
            || lower.ends_with(".bash")
            || lower.ends_with(".py")
            || lower.ends_with(".js")
            || lower.ends_with(".ts")
            || lower.ends_with(".md")
            || lower.ends_with(".yaml")
            || lower.ends_with(".yml")
            || lower.ends_with(".toml")
            || lower.ends_with(".json");

        if is_text && entry.size() < 524_288 {
            let mut content = String::new();
            if std::io::Read::read_to_string(&mut entry, &mut content).is_ok() {
                for rp in RISKY_PATTERNS.iter() {
                    if rp.re.is_match(&content) {
                        risky_patterns.push(AuditFlag {
                            file: name.clone(),
                            reason: rp.reason.to_string(),
                            severity: rp.severity.to_string(),
                        });
                    }
                }
            }
        }
    }

    let has_danger = flagged_files.iter().chain(risky_patterns.iter()).any(|f| f.severity == "danger");
    let has_warning = flagged_files.iter().chain(risky_patterns.iter()).any(|f| f.severity == "warning");
    let risk_level = if has_danger {
        "danger"
    } else if has_warning {
        "warning"
    } else {
        "clean"
    }
    .to_string();

    Ok(SkillAuditReport {
        file_count: files.len(),
        files,
        flagged_files,
        risky_patterns,
        clawhub_scan: None,
        risk_level,
    })
}

// ---------------------------------------------------------------------------
// skills.sh fallback
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SkillsShResponse {
    skills: Vec<SkillsShSkill>,
}

#[derive(Clone, Deserialize)]
struct SkillsShSkill {
    id: String,
    #[serde(default, alias = "skillId")]
    skill_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, alias = "topSource")]
    source: Option<String>,
    #[serde(default)]
    installs: Option<u64>,
}

async fn search_skills_sh(query: &str) -> Result<Option<SkillsShSkill>> {
    let client = http_client()?;
    let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    let url = format!("{}?q={}&limit=50", SKILLS_SH_API, encoded);

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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- normalize_github_url ----

    #[test]
    fn test_normalize_https_url() {
        let result = normalize_github_url("https://github.com/linggen/skills").unwrap();
        assert_eq!(result, "https://github.com/linggen/skills");
    }

    #[test]
    fn test_normalize_https_url_with_git_suffix() {
        let result = normalize_github_url("https://github.com/linggen/skills.git").unwrap();
        assert_eq!(result, "https://github.com/linggen/skills");
    }

    #[test]
    fn test_normalize_https_url_with_trailing_slash() {
        let result = normalize_github_url("https://github.com/linggen/skills/").unwrap();
        assert_eq!(result, "https://github.com/linggen/skills");
    }

    #[test]
    fn test_normalize_shorthand() {
        let result = normalize_github_url("linggen/skills").unwrap();
        assert_eq!(result, "https://github.com/linggen/skills");
    }

    #[test]
    fn test_normalize_git_ssh() {
        let result = normalize_github_url("git@github.com:linggen/skills").unwrap();
        assert_eq!(result, "https://github.com/linggen/skills");
    }

    #[test]
    fn test_normalize_git_ssh_with_git_suffix() {
        let result = normalize_github_url("git@github.com:linggen/skills.git").unwrap();
        assert_eq!(result, "https://github.com/linggen/skills");
    }

    #[test]
    fn test_normalize_unsupported_url() {
        let err = normalize_github_url("https://gitlab.com/foo/bar").unwrap_err();
        assert!(err.to_string().contains("Only GitHub"));
    }

    // ---- parse_github_url ----

    #[test]
    fn test_parse_github_url_basic() {
        let (owner, repo) = parse_github_url("https://github.com/linggen/skills").unwrap();
        assert_eq!(owner, "linggen");
        assert_eq!(repo, "skills");
    }

    #[test]
    fn test_parse_github_url_with_extra_path() {
        let (owner, repo) =
            parse_github_url("https://github.com/linggen/skills/tree/main/foo").unwrap();
        assert_eq!(owner, "linggen");
        assert_eq!(repo, "skills");
    }

    #[test]
    fn test_parse_github_url_invalid() {
        let err = parse_github_url("https://github.com/onlyowner").unwrap_err();
        assert!(err.to_string().contains("Could not parse"));
    }

    // ---- build_github_zip_url ----

    #[test]
    fn test_build_zip_url_branch_name() {
        let url = build_github_zip_url("linggen", "skills", "main");
        assert_eq!(
            url,
            "https://github.com/linggen/skills/archive/refs/heads/main.zip"
        );
    }

    #[test]
    fn test_build_zip_url_full_ref() {
        let url = build_github_zip_url("linggen", "skills", "refs/heads/develop");
        assert_eq!(
            url,
            "https://github.com/linggen/skills/archive/refs/heads/develop.zip"
        );
    }

    #[test]
    fn test_build_zip_url_heads_prefix() {
        let url = build_github_zip_url("linggen", "skills", "heads/main");
        assert_eq!(
            url,
            "https://github.com/linggen/skills/archive/refs/heads/main.zip"
        );
    }

    #[test]
    fn test_build_zip_url_tags_prefix() {
        let url = build_github_zip_url("linggen", "skills", "tags/v1.0");
        assert_eq!(
            url,
            "https://github.com/linggen/skills/archive/refs/tags/v1.0.zip"
        );
    }

    // ---- skill_target_dir ----

    #[test]
    fn test_skill_target_dir_project() {
        let root = Path::new("/tmp/my-project");
        let result = skill_target_dir("my-skill", SkillScope::Project, Some(root)).unwrap();
        assert_eq!(result, PathBuf::from("/tmp/my-project/.linggen/skills/my-skill"));
    }

    #[test]
    fn test_skill_target_dir_project_no_root() {
        let err = skill_target_dir("my-skill", SkillScope::Project, None).unwrap_err();
        assert!(err.to_string().contains("Project root required"));
    }

    #[test]
    fn test_skill_target_dir_global() {
        let result = skill_target_dir("my-skill", SkillScope::Global, None).unwrap();
        let expected = crate::paths::global_skills_dir().join("my-skill");
        assert_eq!(result, expected);
    }

    // ---- SkillScope ----

    #[test]
    fn test_skill_scope_default() {
        assert_eq!(SkillScope::default(), SkillScope::Global);
    }

    #[test]
    fn test_skill_scope_serde() {
        let json = serde_json::to_string(&SkillScope::Global).unwrap();
        assert_eq!(json, "\"global\"");
        let parsed: SkillScope = serde_json::from_str("\"project\"").unwrap();
        assert_eq!(parsed, SkillScope::Project);
    }

    // ---- MarketplaceSkill serde ----

    #[test]
    fn test_marketplace_skill_serde() {
        let skill = MarketplaceSkill {
            skill_id: "my-skill".into(),
            name: "my-skill".into(),
            url: "https://github.com/linggen/skills".into(),
            description: Some("Memory skill".into()),
            install_count: 42,
            git_ref: Some("main".into()),
            content: None,
            updated_at: None,
            source_registry: Some("skills.sh".into()),
        };
        let json = serde_json::to_string(&skill).unwrap();
        let parsed: MarketplaceSkill = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.skill_id, "my-skill");
        assert_eq!(parsed.install_count, 42);
    }
}
