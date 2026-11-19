use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
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
#[derive(Default)]
pub enum SkillScope {
    Project,
    #[default]
    Global,
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

// ---------------------------------------------------------------------------
// HTTP client / community search
// ---------------------------------------------------------------------------

pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("linggen")
        .connect_timeout(crate::mirror::CONNECT_TIMEOUT)
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")
}

/// Search community skills across all registries (skills.sh + ClawHub).
/// Results are interleaved from both sources (each pre-sorted by relevance),
/// so neither source dominates the top of the list.
pub async fn search_community(query: &str) -> Result<Vec<MarketplaceSkill>> {
    let (sh_result, ch_result) =
        tokio::join!(search_skills_sh_community(query), search_clawhub(query),);
    merge_sources(sh_result, ch_result)
}

/// Both sources' answers as one list. A source that could not be reached is
/// not an empty answer: with both down the search fails and names them; with
/// one down the other's results stand and the outage is logged.
fn merge_sources(
    sh_result: Result<Vec<MarketplaceSkill>>,
    ch_result: Result<Vec<MarketplaceSkill>>,
) -> Result<Vec<MarketplaceSkill>> {
    let (sh_skills, ch_skills) = match (sh_result, ch_result) {
        (Err(sh), Err(ch)) => {
            anyhow::bail!("No skill source could be reached — skills.sh: {sh:#}; ClawHub: {ch:#}")
        }
        (sh, ch) => (
            sh.unwrap_or_else(|e| {
                tracing::warn!("skills.sh could not be reached: {e:#}");
                Vec::new()
            }),
            ch.unwrap_or_else(|e| {
                tracing::warn!("ClawHub could not be reached: {e:#}");
                Vec::new()
            }),
        ),
    };

    // Interleave results from both sources, deduplicating by skill_id.
    let mut results = Vec::with_capacity(sh_skills.len() + ch_skills.len());
    let mut seen = HashSet::new();
    let mut sh_iter = sh_skills.into_iter();
    let mut ch_iter = ch_skills.into_iter();
    loop {
        let sh_next = sh_iter.next();
        let ch_next = ch_iter.next();
        if sh_next.is_none() && ch_next.is_none() {
            break;
        }
        for skill in [sh_next, ch_next].into_iter().flatten() {
            let key = skill.skill_id.to_lowercase();
            if seen.insert(key) {
                results.push(skill);
            }
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
        anyhow::bail!("skills.sh answered HTTP {}", resp.status());
    }

    let payload: SkillsShResponse = resp.json().await?;
    let results = payload
        .skills
        .into_iter()
        .map(|s| {
            let skill_name = s
                .skill_id
                .clone()
                .or(s.name.clone())
                .unwrap_or_else(|| s.id.clone());
            let source = s.source.as_deref().unwrap_or("");
            MarketplaceSkill {
                skill_id: s.id.clone(),
                name: skill_name,
                url: if source.is_empty() {
                    String::new()
                } else {
                    format!("https://github.com/{}", source)
                },
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
    let url = format!(
        "{}/search?q={}&limit=30&nonSuspiciousOnly=true",
        CLAWHUB_API, encoded
    );

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("ClawHub answered HTTP {}", resp.status());
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
    if !slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "Invalid slug '{}': only alphanumeric, hyphens, and underscores allowed",
            slug
        );
    }
    Ok(())
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

    refuse_unforced(name, target_dir, force)?;

    // Download ZIP
    let zip_url = build_github_zip_url(&owner, &repo, git_ref);
    let client = http_client()?;
    let temp_zip = download_to_temp(&client, &zip_url).await?;

    let result = install_named_from_zip(&temp_zip, name, &repo, target_dir);
    let _ = fs::remove_file(&temp_zip);

    match result {
        Ok(_) => {
            Ok(format!(
                "Skill '{}' installed to {}",
                name,
                target_dir.display()
            ))
        }
        Err(e) => {
            // If not found in default repo, try skills.sh fallback
            if e.to_string().contains("Could not find skill") && is_default_repo(&normalized) {
                if let Some(fallback) = search_skills_sh(name).await? {
                    let fallback_source = fallback.source.as_deref().unwrap_or("");
                    if fallback_source != "linggen/skills" {
                        let fallback_repo = format!("https://github.com/{}", fallback_source);
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

    let result = install_named_from_zip(&temp_zip, name, &repo, target_dir);
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
    refuse_unforced(slug, target_dir, force)?;

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
    let result = install_first_from_zip(&temp_zip, slug, target_dir);
    let _ = fs::remove_file(&temp_zip);
    result?;

    Ok(format!(
        "Skill '{}' installed from ClawHub to {}",
        slug,
        target_dir.display()
    ))
}

/// An installed skill is replaced only when asked — refused before anything
/// is fetched.
fn refuse_unforced(name: &str, target_dir: &Path, force: bool) -> Result<()> {
    if target_dir.exists() && !force {
        anyhow::bail!(
            "Skill '{}' already installed at {}. Use force to overwrite.",
            name,
            target_dir.display()
        );
    }
    Ok(())
}

/// Install the skill `name` from a downloaded repo ZIP into `target_dir`.
fn install_named_from_zip(zip: &Path, name: &str, repo: &str, target_dir: &Path) -> Result<()> {
    let staging = staging_beside(target_dir)?;
    let new = staging.path().join("new");
    extract_skills_from_zip(zip, &new, SkillPick::Named { skill: name, repo })?;
    swap_in(staging, &new, target_dir)
}

/// Install the first skill of a ClawHub ZIP (they hold one) into `target_dir`.
fn install_first_from_zip(zip: &Path, slug: &str, target_dir: &Path) -> Result<()> {
    let staging = staging_beside(target_dir)?;
    let all = staging.path().join("all");
    let installed = extract_skills_from_zip(zip, &all, SkillPick::All)?;
    let Some(first) = installed.first() else {
        anyhow::bail!("No skill found in ClawHub ZIP for '{}'", slug);
    };
    swap_in(staging, &all.join(first), target_dir)
}

/// A folder beside `target_dir` for the new copy: on the same disk, so the
/// swap is a rename. Its dot-name, and the copy sitting one level down in
/// it, keep the skill loader from reading it as a skill.
fn staging_beside(target_dir: &Path) -> Result<tempfile::TempDir> {
    let parent = target_dir
        .parent()
        .context("The skill folder has no parent")?;
    fs::create_dir_all(parent)?;
    tempfile::Builder::new()
        .prefix(".installing-")
        .tempdir_in(parent)
        .context("Failed to create the install folder")
}

/// Put the whole new copy `new` in place of `target_dir`, carrying the
/// installed skill's state over ([`state_paths`]). A failure leaves the
/// installed skill as it was; if even putting it back fails, the staging
/// folder holding it is kept and named in the error.
///
/// The skill's install script (`install:` in its manifest) runs in place,
/// as the last step of the swap: a script that fails fails the install, and
/// the new copy goes back out — nothing half-installed is left.
fn swap_in(staging: tempfile::TempDir, new: &Path, target_dir: &Path) -> Result<()> {
    if !target_dir.exists() {
        fs::rename(new, target_dir).context("Failed to put the skill in place")?;
        let Err(error) = install_script(target_dir) else {
            return Ok(());
        };
        let _ = fs::remove_dir_all(target_dir);
        return Err(error);
    }
    let old = staging.path().join("old");
    let mut carried = Vec::new();
    let swapped = carry_state(target_dir, new, &mut carried)
        .and_then(|()| {
            fs::rename(target_dir, &old)?;
            fs::rename(new, target_dir)
        })
        .map_err(|e| anyhow::Error::from(e).context("Failed to replace the skill"));
    let Err(error) = swapped.and_then(|()| install_script(target_dir)) else {
        return Ok(());
    };
    // The new copy, if it went in, goes back where it came from first.
    let new_out = if old.exists() && target_dir.exists() && !new.exists() {
        fs::rename(target_dir, new)
    } else {
        Ok(())
    };
    if let Err(back) = new_out.and_then(|()| put_back(target_dir, new, &old, &carried)) {
        let kept = staging.keep();
        anyhow::bail!(
            "Failed to replace the skill ({error:#}), and putting it back failed too ({back}): it is kept at {}",
            kept.display()
        );
    }
    Err(error)
}

/// The skill's install script, if its manifest names one.
fn install_script(skill_dir: &Path) -> Result<()> {
    super::skills::run_install_script(skill_dir).map(|_| ())
}

/// What of an installed skill is the person's, kept across an update: its
/// `data/`, and every path its manifest keeps as state (`cloud.save`) that
/// stays inside the skill folder. Outermost first; a path inside another is
/// carried with it.
fn state_paths(skill_dir: &Path) -> Vec<PathBuf> {
    let declared =
        super::skills::load_skill_from_path(skill_dir, super::skills::SkillSource::Global)
            .ok()
            .flatten()
            .and_then(|skill| skill.cloud)
            .and_then(|cloud| cloud.save)
            .map(|save| {
                save.paths()
                    .into_iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
    let mut paths: Vec<PathBuf> = std::iter::once(PathBuf::from("data"))
        .chain(declared)
        .filter(|p| {
            p.components().count() > 0 && p.components().all(|c| matches!(c, Component::Normal(_)))
        })
        .collect();
    paths.sort_by_key(|p| p.components().count());
    let mut outermost: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !outermost.iter().any(|kept| path.starts_with(kept)) {
            outermost.push(path);
        }
    }
    outermost
}

/// Move the installed skill's state into the new copy, over the archive's
/// own; `carried` lists what has moved, so it can be put back.
fn carry_state(old: &Path, new: &Path, carried: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for rel in state_paths(old) {
        let (from, to) = (old.join(&rel), new.join(&rel));
        if !from.exists() {
            continue;
        }
        if to.is_dir() {
            fs::remove_dir_all(&to)?;
        } else if to.exists() {
            fs::remove_file(&to)?;
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&from, &to)?;
        carried.push(rel);
    }
    Ok(())
}

/// Undo a swap that did not finish: the installed skill back at
/// `target_dir`, and its state back in it.
fn put_back(target_dir: &Path, new: &Path, old: &Path, carried: &[PathBuf]) -> std::io::Result<()> {
    if old.exists() && !target_dir.exists() {
        fs::rename(old, target_dir)?;
    }
    for rel in carried.iter().rev() {
        fs::rename(new.join(rel), target_dir.join(rel))?;
    }
    Ok(())
}

pub fn delete_skill(name: &str, target_dir: &Path) -> Result<String> {
    if !target_dir.exists() {
        anyhow::bail!("Skill '{}' not found at {}", name, target_dir.display());
    }
    fs::remove_dir_all(target_dir)?;
    Ok(format!(
        "Skill '{}' deleted from {}",
        name,
        target_dir.display()
    ))
}

pub fn skill_target_dir(
    name: &str,
    scope: SkillScope,
    project_root: Option<&Path>,
) -> Result<PathBuf> {
    match scope {
        SkillScope::Project => {
            let root = project_root.ok_or_else(|| {
                anyhow::anyhow!("Project root required for project-scoped install")
            })?;
            Ok(root.join(".linggen/skills").join(name))
        }
        SkillScope::Global => Ok(crate::paths::global_skills_dir().join(name)),
    }
}

/// Resolve the actual filesystem directory for a skill based on its loaded source.
/// Unlike `skill_target_dir` (which only knows Project/Global), this handles
/// Compat sources (Claude, Codex) correctly.
pub fn skill_dir_for_source(
    name: &str,
    source: &super::skills::SkillSource,
    project_root: Option<&Path>,
) -> Result<PathBuf> {
    match source {
        super::skills::SkillSource::Global => Ok(crate::paths::global_skills_dir().join(name)),
        super::skills::SkillSource::Project => {
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
        super::skills::SkillSource::Compat { label } => {
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
    source: &super::skills::SkillSource,
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

pub fn build_github_zip_url(owner: &str, repo: &str, git_ref: &str) -> String {
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

pub async fn download_to_temp(client: &reqwest::Client, url: &str) -> Result<PathBuf> {
    let tmp = tempfile::NamedTempFile::new().context("Failed to create temp file")?;
    // Persist immediately so the path is stable; caller is responsible for cleanup.
    let tmp_path = tmp.into_temp_path();

    // GitHub first, then linggen.dev's mirror (crate::mirror) — the same
    // download from anywhere GitHub can't be reached. A definitive answer (a
    // 404) is final; an unreachable round is tried again with a backoff.
    let max_attempts = 3;
    let mut last_error = String::new();
    for attempt in 0..max_attempts {
        match crate::mirror::get_bytes(client, url).await {
            Ok(bytes) => {
                fs::write(&tmp_path, &bytes).context("Failed to write temp file")?;
                let kept = tmp_path
                    .keep()
                    .map_err(|e| anyhow::anyhow!("tempfile keep error: {}", e))?;
                return Ok(kept);
            }
            Err(e) => {
                let definitive = e
                    .downcast_ref::<crate::mirror::HttpStatus>()
                    .is_some_and(|h| !is_retryable_status(h.status));
                last_error = e.to_string();
                if definitive || attempt == max_attempts - 1 {
                    break;
                }
                let delay = Duration::from_secs(1 << attempt);
                tracing::warn!(err = %e, attempt = attempt + 1, "Download failed, retrying...");
                tokio::time::sleep(delay).await;
            }
        }
    }

    anyhow::bail!("Download failed: {} - {}", url, last_error)
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::SERVICE_UNAVAILABLE
        || status == reqwest::StatusCode::GATEWAY_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

// ---------------------------------------------------------------------------
// ZIP extraction
// ---------------------------------------------------------------------------

/// Which skills to take out of a ZIP.
pub enum SkillPick<'a> {
    /// Every skill, each into `<target>/<dir name>/`. A lone SKILL.md at
    /// the archive root makes the whole archive one skill, in `<target>/_root/`.
    All,
    /// One named skill, straight into `target`. `repo` lets a repo-root
    /// SKILL.md stand for a skill named after the repo.
    Named { skill: &'a str, repo: &'a str },
}

/// A `<dir>/SKILL.md` entry: the skill's directory name, its root inside
/// the archive, the entry's full name, and how deep the entry sits.
struct SkillMd {
    dir_name: String,
    root: PathBuf,
    entry: String,
    depth: usize,
}

/// Extract skills from a ZIP archive. Returns the extracted skill directory
/// names (for `Named`, the one matched).
pub fn extract_skills_from_zip(
    zip_path: &Path,
    target: &Path,
    pick: SkillPick,
) -> Result<Vec<String>> {
    let file = fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    let (found, bare_root) = scan_skill_mds(&mut archive)?;

    match pick {
        SkillPick::Named { skill, repo } => {
            let md = resolve_named_skill(&found, skill, repo)?;
            fs::create_dir_all(target)?;
            copy_zip_dir(&mut archive, md.root.to_str().unwrap_or(""), target)?;
            Ok(vec![md.dir_name.clone()])
        }
        SkillPick::All => {
            let nested: Vec<&SkillMd> = found.iter().filter(|m| !m.dir_name.is_empty()).collect();
            // Root-level SKILL.md — all ZIP contents belong to one skill.
            // Extract into a "_root" subdirectory; the caller renames as needed.
            if bare_root && nested.is_empty() {
                let dir_name = "_root".to_string();
                let target_dir = target.join(&dir_name);
                fs::create_dir_all(&target_dir)?;
                copy_zip_dir(&mut archive, "", &target_dir)?;
                return Ok(vec![dir_name]);
            }
            // Deduplicate by dir name (first occurrence wins).
            let mut seen = BTreeSet::new();
            let mut installed = Vec::new();
            for md in nested {
                if !seen.insert(md.dir_name.clone()) {
                    continue;
                }
                let target_dir = target.join(&md.dir_name);
                fs::create_dir_all(&target_dir)?;
                copy_zip_dir(&mut archive, md.root.to_str().unwrap_or(""), &target_dir)?;
                installed.push(md.dir_name.clone());
            }
            Ok(installed)
        }
    }
}

/// Every `<dir>/SKILL.md` (or `skill.md`) in the archive, in archive order,
/// plus whether a bare `SKILL.md` sits at the archive root.
fn scan_skill_mds(archive: &mut ZipArchive<fs::File>) -> Result<(Vec<SkillMd>, bool)> {
    let mut found = Vec::new();
    let mut bare_root = false;
    for i in 0..archive.len() {
        let name = archive.by_index(i)?.name().to_string();
        if name == "SKILL.md" || name == "skill.md" {
            bare_root = true;
            continue;
        }
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
        let depth = path
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count();
        found.push(SkillMd {
            dir_name,
            root: parent.to_path_buf(),
            entry: name.clone(),
            depth,
        });
    }
    Ok((found, bare_root))
}

/// Find the one skill named `skill`: by directory name (hyphens and
/// underscores alike) or path segment, then a unique `<prefix>-<dir>` match,
/// then a repo-root SKILL.md when nothing else can be meant.
fn resolve_named_skill<'a>(found: &'a [SkillMd], skill: &str, repo: &str) -> Result<&'a SkillMd> {
    let norm_skill = skill.replace('-', "_");
    let segment = format!("/{}/", skill);
    if let Some(md) = found
        .iter()
        .find(|m| m.dir_name.replace('-', "_") == norm_skill || m.entry.contains(&segment))
    {
        return Ok(md);
    }

    // Fallback: prefixed skill names
    let prefixed: Vec<&SkillMd> = found
        .iter()
        .filter(|m| !m.dir_name.is_empty() && skill.ends_with(&format!("-{}", m.dir_name)))
        .collect();
    if prefixed.len() == 1 {
        return Ok(prefixed[0]);
    }

    // Fallback: root SKILL.md
    if let Some(root) = found.iter().rev().find(|m| m.depth == 2) {
        let has_only_root = found.iter().all(|m| m.dir_name == root.dir_name);
        if skill == repo || found.len() == 1 || has_only_root {
            return Ok(root);
        }
    }

    let available: BTreeSet<String> = found
        .iter()
        .map(|m| m.dir_name.clone())
        .filter(|s| !s.is_empty())
        .collect();
    let shown: Vec<String> = available.iter().take(10).cloned().collect();
    let mut msg = format!(
        "Could not find skill '{}' in the repository. Make sure it contains a SKILL.md file.",
        skill
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
    Err(anyhow::anyhow!(msg))
}

/// Copy every file under `root/` in the archive into `target_dir`, keeping
/// the relative layout. An empty `root` copies the whole archive.
fn copy_zip_dir(archive: &mut ZipArchive<fs::File>, root: &str, target_dir: &Path) -> Result<()> {
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let rel_path = if root.is_empty() {
            name.as_str()
        } else {
            // `root/` — a sibling like `root-extra/` is another skill.
            match name.strip_prefix(root) {
                Some(rest) if rest.starts_with('/') => rest.trim_start_matches('/'),
                _ => continue,
            }
        };
        if !is_safe_zip_path(rel_path, target_dir) {
            continue;
        }
        let dest = target_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = fs::File::create(&dest)?;
        std::io::copy(&mut entry, &mut outfile)?;
    }
    Ok(())
}

/// Validate a relative path from a ZIP entry is safe to extract under `base_dir`.
fn is_safe_zip_path(rel_path: &str, base_dir: &Path) -> bool {
    if rel_path.is_empty() || rel_path.starts_with('/') {
        return false;
    }
    // Reject any path component that is ".." to prevent traversal.
    // Using a string check is conservative but safe regardless of whether
    // base_dir is absolute or relative.
    if Path::new(rel_path)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return false;
    }
    // Double-check: the joined path must still start with the base.
    let dest = base_dir.join(rel_path);
    let canonical: PathBuf = dest.components().collect();
    let base_canonical: PathBuf = base_dir.components().collect();
    canonical.starts_with(&base_canonical)
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

    // ---- extract_skills_from_zip ----

    fn write_zip(dir: &Path, files: &[&str]) -> PathBuf {
        use std::io::Write;
        let path = dir.join("skills.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for name in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(name.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    const REPO: &[&str] = &[
        "skills-main/README.md",
        "skills-main/foo/SKILL.md",
        "skills-main/foo/scripts/a.js",
        "skills-main/foo-extra/SKILL.md",
        "skills-main/foo-extra/b.js",
    ];

    #[test]
    fn a_named_pick_takes_only_that_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), REPO);
        let out = tmp.path().join("foo");
        let got = extract_skills_from_zip(
            &zip,
            &out,
            SkillPick::Named {
                skill: "foo",
                repo: "skills",
            },
        )
        .unwrap();
        assert_eq!(got, vec!["foo".to_string()]);
        assert!(out.join("SKILL.md").is_file());
        assert!(out.join("scripts/a.js").is_file());
        // A sibling whose name only starts the same is another skill.
        assert!(!out.join("-extra").exists() && !out.join("b.js").exists());
    }

    #[test]
    fn a_named_pick_that_matches_nothing_lists_what_is_there() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), REPO);
        let err = extract_skills_from_zip(
            &zip,
            &tmp.path().join("x"),
            SkillPick::Named {
                skill: "nope",
                repo: "skills",
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Available skills: foo, foo-extra"), "{err}");
    }

    #[test]
    fn a_repo_root_skill_answers_to_the_repo_name() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), &["tool-main/SKILL.md", "tool-main/run.sh"]);
        let out = tmp.path().join("tool");
        extract_skills_from_zip(
            &zip,
            &out,
            SkillPick::Named {
                skill: "tool",
                repo: "tool",
            },
        )
        .unwrap();
        assert!(out.join("run.sh").is_file());
    }

    #[test]
    fn pick_all_takes_every_skill_into_its_own_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), REPO);
        let out = tmp.path().join("all");
        let got = extract_skills_from_zip(&zip, &out, SkillPick::All).unwrap();
        assert_eq!(got, vec!["foo".to_string(), "foo-extra".to_string()]);
        assert!(out.join("foo/scripts/a.js").is_file());
        assert!(out.join("foo-extra/b.js").is_file());
        assert!(!out.join("foo/-extra").exists());
    }

    #[test]
    fn pick_all_with_a_bare_root_skill_lands_in_root_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), &["SKILL.md", "lib/x.py"]);
        let out = tmp.path().join("all");
        let got = extract_skills_from_zip(&zip, &out, SkillPick::All).unwrap();
        assert_eq!(got, vec!["_root".to_string()]);
        assert!(out.join("_root/lib/x.py").is_file());
    }

    // ---- searching ----

    fn found(id: &str) -> MarketplaceSkill {
        MarketplaceSkill {
            skill_id: id.into(),
            name: id.into(),
            url: String::new(),
            description: None,
            install_count: 0,
            git_ref: None,
            content: None,
            updated_at: None,
            source_registry: None,
        }
    }

    #[test]
    fn sources_that_could_not_be_reached_are_not_an_empty_answer() {
        let down = || Err(anyhow::anyhow!("connection refused"));
        let err = merge_sources(down(), down()).unwrap_err().to_string();
        assert!(err.contains("skills.sh") && err.contains("ClawHub"), "{err}");
        let one = merge_sources(down(), Ok(vec![found("a")])).unwrap();
        assert_eq!(one.len(), 1, "the other source's results stand");
        assert!(merge_sources(Ok(vec![]), Ok(vec![])).unwrap().is_empty(), "no match is empty");
    }

    // ---- install scripts ----

    fn write_zip_with(dir: &Path, files: &[(&str, &str)]) -> PathBuf {
        use std::io::Write;
        let path = dir.join("scripted.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, body) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    const FAILING_SCRIPT: &[(&str, &str)] = &[
        ("skills-main/foo/SKILL.md", "---\nname: foo\ndescription: d\ninstall: install.sh\n---\nBody"),
        ("skills-main/foo/install.sh", "echo half >> \"$SKILL_DIR/half.txt\"\nexit 3\n"),
    ];

    #[test]
    fn a_failed_install_script_fails_a_first_install_and_leaves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip_with(tmp.path(), FAILING_SCRIPT);
        let skills = tmp.path().join("skills");
        let dir = skills.join("foo");
        let err = install_named_from_zip(&zip, "foo", "skills", &dir).unwrap_err();
        assert!(format!("{err:#}").contains("Install script failed"), "{err:#}");
        assert!(!dir.exists(), "no half-installed skill");
        assert_eq!(fs::read_dir(&skills).unwrap().count(), 0, "nothing left beside it");
    }

    #[test]
    fn a_failed_install_script_leaves_an_update_as_it_was() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip_with(tmp.path(), FAILING_SCRIPT);
        let skills = tmp.path().join("skills");
        let dir = installed_foo(&skills);
        assert!(install_named_from_zip(&zip, "foo", "skills", &dir).is_err());
        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), "old");
        assert_eq!(fs::read_to_string(dir.join("data/state.json")).unwrap(), "{\"mine\":1}");
        assert!(!dir.join("half.txt").exists());
        let names: Vec<_> = fs::read_dir(&skills).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("foo")]);
    }

    #[test]
    fn a_passing_install_script_runs_in_the_installed_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip_with(tmp.path(), &[
            ("skills-main/foo/SKILL.md", "---\nname: foo\ndescription: d\ninstall: install.sh\n---\nBody"),
            ("skills-main/foo/install.sh", "echo ok > \"$SKILL_DIR/ran.txt\"\n"),
        ]);
        let dir = tmp.path().join("skills/foo");
        install_named_from_zip(&zip, "foo", "skills", &dir).unwrap();
        assert!(dir.join("ran.txt").is_file());
    }

    // ---- replacing an installed skill ----

    /// An installed `foo` with the person's state in `data/`.
    fn installed_foo(skills: &Path) -> PathBuf {
        let dir = skills.join("foo");
        fs::create_dir_all(dir.join("data")).unwrap();
        fs::write(dir.join("SKILL.md"), "old").unwrap();
        fs::write(dir.join("data/state.json"), "{\"mine\":1}").unwrap();
        dir
    }

    #[test]
    fn an_update_replaces_the_skill_and_keeps_its_data() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), REPO);
        let skills = tmp.path().join("skills");
        let dir = installed_foo(&skills);
        install_named_from_zip(&zip, "foo", "skills", &dir).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("SKILL.md")).unwrap(),
            "skills-main/foo/SKILL.md"
        );
        assert!(dir.join("scripts/a.js").is_file());
        assert_eq!(
            fs::read_to_string(dir.join("data/state.json")).unwrap(),
            "{\"mine\":1}"
        );
        // Nothing of the install is left beside the skill.
        let names: Vec<_> = fs::read_dir(&skills)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("foo")]);
    }

    #[test]
    fn a_failed_update_leaves_the_installed_skill_as_it_was() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), REPO);
        let skills = tmp.path().join("skills");
        let dir = installed_foo(&skills);
        assert!(install_named_from_zip(&zip, "nope", "skills", &dir).is_err());
        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), "old");
        assert_eq!(
            fs::read_to_string(dir.join("data/state.json")).unwrap(),
            "{\"mine\":1}"
        );
        let names: Vec<_> = fs::read_dir(&skills)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("foo")]);
    }

    #[test]
    fn a_clawhub_update_keeps_the_data_and_a_failed_one_keeps_everything() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), &["SKILL.md", "lib/x.py"]);
        let skills = tmp.path().join("skills");
        let dir = installed_foo(&skills);
        install_first_from_zip(&zip, "foo", &dir).unwrap();
        assert!(dir.join("lib/x.py").is_file());
        assert_eq!(
            fs::read_to_string(dir.join("SKILL.md")).unwrap(),
            "SKILL.md"
        );
        assert!(dir.join("data/state.json").is_file());

        let empty = tmp.path().join("empty.zip");
        zip::ZipWriter::new(fs::File::create(&empty).unwrap())
            .finish()
            .unwrap();
        assert!(install_first_from_zip(&empty, "foo", &dir).is_err());
        assert!(dir.join("lib/x.py").is_file());
        assert!(dir.join("data/state.json").is_file());
    }

    /// State the manifest marks as kept (`cloud.save`) outside `data/` is
    /// carried over too.
    #[test]
    fn an_update_keeps_what_the_manifest_marks_as_state() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), REPO);
        let skills = tmp.path().join("skills");
        let dir = installed_foo(&skills);
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: foo\ndescription: d\ncloud:\n  save: [config.json, ../outside.json]\n---\nBody",
        )
        .unwrap();
        fs::write(dir.join("config.json"), "mine").unwrap();
        fs::write(dir.join("scratch.txt"), "not state").unwrap();
        install_named_from_zip(&zip, "foo", "skills", &dir).unwrap();
        assert_eq!(fs::read_to_string(dir.join("config.json")).unwrap(), "mine");
        assert!(dir.join("data/state.json").is_file());
        assert!(!dir.join("scratch.txt").exists(), "only state is carried");
    }

    #[test]
    fn a_first_install_creates_the_skills_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), REPO);
        let dir = tmp.path().join("new/skills/foo");
        install_named_from_zip(&zip, "foo", "skills", &dir).unwrap();
        assert!(dir.join("SKILL.md").is_file());
    }

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
        assert_eq!(
            result,
            PathBuf::from("/tmp/my-project/.linggen/skills/my-skill")
        );
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
