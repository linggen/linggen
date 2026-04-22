pub mod marketplace;

use crate::engine::skill_tool::{SkillParamDef, SkillToolDef};
use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Built-in data tool auto-registered for every skill with `app`.
/// The agent calls it to refresh the skill's dashboard UI; the engine emits
/// a `ContentBlockUpdate` carrying the `page` payload, which the skill's
/// iframe picks up via `onContentBlock`. No shell command — `cmd` empty
/// makes this a data tool (see SkillToolDef::execute).
fn page_update_tool_def() -> SkillToolDef {
    let mut args = HashMap::new();
    args.insert(
        "body".to_string(),
        SkillParamDef {
            param_type: "array".to_string(),
            required: false,
            default: None,
            description:
                "Array of body widgets stacked vertically (main content). \
                 Widget shapes are defined in the skill's SKILL.md (e.g. `info`, \
                 `table`, `action-cards`, `progress`)."
                    .to_string(),
            items: None,
        },
    );
    args.insert(
        "top_bar".to_string(),
        SkillParamDef {
            param_type: "array".to_string(),
            required: false,
            default: None,
            description:
                "Array of compact metric widgets shown at the top of the dashboard. \
                 Each item: {\"widget\":\"custom\",\"data\":{...}}."
                    .to_string(),
            items: None,
        },
    );
    args.insert(
        "footer".to_string(),
        SkillParamDef {
            param_type: "object".to_string(),
            required: false,
            default: None,
            description:
                "Optional status line at the bottom. Shape: {\"text\":\"...\"}."
                    .to_string(),
            items: None,
        },
    );
    SkillToolDef {
        name: "PageUpdate".to_string(),
        description:
            "Refresh the skill's dashboard UI. Pass `top_bar`, `body`, and/or `footer` \
             as top-level arguments — omit any section you don't want to change \
             (previous values persist). At least one of the three must be provided \
             and non-empty. See the skill's SKILL.md for widget shapes."
                .to_string(),
        cmd: String::new(),
        endpoint: None,
        tier: None,
        args,
        returns: Some("ok".to_string()),
        timeout_ms: 1000,
        skill_name: None,
        skill_dir: None,
    }
}

/// Built-in skills available for one-click install from the linggen/skills repo.
#[derive(Debug, Clone, Serialize)]
pub struct BuiltInSkillInfo {
    pub name: String,
    pub description: String,
    pub installed: bool,
}

const GITHUB_CONTENTS_URL: &str = "https://api.github.com/repos/linggen/skills/contents/";
const GITHUB_RAW_URL: &str = "https://raw.githubusercontent.com/linggen/skills/main";
const CACHE_TTL: Duration = Duration::from_secs(600); // 10 min
const SKIP_DIRS: &[&str] = &[".claude", ".cursor", ".linggen", ".git"];

type BuiltInCache = Mutex<Option<(Instant, Vec<BuiltInSkillInfo>)>>;

fn builtin_cache() -> &'static BuiltInCache {
    static CACHE: OnceLock<BuiltInCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Clears the built-in skills cache so the next fetch hits GitHub.
pub async fn clear_builtin_cache() {
    let mut cache = builtin_cache().lock().await;
    *cache = None;
}

/// Fetches the list of built-in skills from the linggen/skills GitHub repo.
/// Results are cached for 10 minutes. On error, returns cached data or empty vec.
pub async fn fetch_builtin_skills() -> Vec<BuiltInSkillInfo> {
    // Check cache
    {
        let cache = builtin_cache().lock().await;
        if let Some((ts, ref skills)) = *cache {
            if ts.elapsed() < CACHE_TTL {
                return skills.clone();
            }
        }
    }

    // Fetch from GitHub
    match fetch_builtin_skills_inner().await {
        Ok(skills) => {
            let mut cache = builtin_cache().lock().await;
            *cache = Some((Instant::now(), skills.clone()));
            skills
        }
        Err(e) => {
            tracing::warn!(err = %e, "Failed to fetch built-in skills from GitHub");
            // Return stale cache if available
            let cache = builtin_cache().lock().await;
            if let Some((_, ref skills)) = *cache {
                return skills.clone();
            }
            vec![]
        }
    }
}

#[derive(Deserialize)]
struct GitHubContentEntry {
    name: String,
    #[serde(rename = "type")]
    entry_type: String,
}

async fn fetch_builtin_skills_inner() -> Result<Vec<BuiltInSkillInfo>> {
    let client = crate::skills::marketplace::http_client()?;

    // List repo contents
    let entries: Vec<GitHubContentEntry> = client
        .get(GITHUB_CONTENTS_URL)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let global_skills_dir = crate::paths::global_skills_dir();

    let dirs: Vec<&GitHubContentEntry> = entries
        .iter()
        .filter(|e| e.entry_type == "dir" && !SKIP_DIRS.contains(&e.name.as_str()))
        .collect();

    // Fetch all SKILL.md files concurrently.
    let fetches = dirs.iter().map(|entry| {
        let client = &client;
        let url = format!("{}/{}/SKILL.md", GITHUB_RAW_URL, entry.name);
        async move {
            let resp = client.get(&url).send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let text = resp.text().await.ok()?;
            let (name, description) = parse_frontmatter_meta(&text)?;
            Some((entry.name.clone(), name, description))
        }
    });
    let results = futures_util::future::join_all(fetches).await;

    let skills = results
        .into_iter()
        .flatten()
        .map(|(dir_name, name, description)| {
            let installed = global_skills_dir.join(&dir_name).join("SKILL.md").exists();
            BuiltInSkillInfo { name, description, installed }
        })
        .collect();

    Ok(skills)
}

/// Run a skill's install script if declared in frontmatter.
/// The script runs with `$SKILL_DIR` set to the skill directory.
/// Returns Ok(Some(output)) if a script ran, Ok(None) if no script declared.
pub fn run_install_script(skill_dir: &Path) -> Result<Option<String>> {
    let skill_md = skill_dir.join("SKILL.md");
    let text = std::fs::read_to_string(&skill_md)?;

    if !text.starts_with("---") {
        return Ok(None);
    }
    let parts: Vec<&str> = text.splitn(3, "---").collect();
    if parts.len() < 3 {
        return Ok(None);
    }

    #[derive(Deserialize)]
    struct InstallMeta {
        #[serde(default)]
        install: Option<String>,
    }

    let meta: InstallMeta = match serde_yml::from_str(parts[1]) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };

    let script_path = match meta.install {
        Some(ref p) => skill_dir.join(p),
        None => return Ok(None),
    };

    if !script_path.exists() {
        anyhow::bail!(
            "Install script not found: {}",
            script_path.display()
        );
    }

    tracing::info!(skill_dir = %skill_dir.display(), script = %script_path.display(), "Running skill install script");

    let output = std::process::Command::new("bash")
        .arg(&script_path)
        .env("SKILL_DIR", skill_dir)
        .current_dir(skill_dir)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Install script failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(Some(stdout))
}

/// Extract `name` and `description` from YAML frontmatter in a SKILL.md file.
fn parse_frontmatter_meta(text: &str) -> Option<(String, String)> {
    if !text.starts_with("---") {
        return None;
    }
    let parts: Vec<&str> = text.splitn(3, "---").collect();
    if parts.len() < 3 {
        return None;
    }

    #[derive(Deserialize)]
    struct Meta {
        name: String,
        description: String,
    }

    let meta: Meta = serde_yml::from_str(parts[1]).ok()?;
    Some((meta.name, meta.description))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// How to launch: "web" (serve static files), "bash" (run script), "url" (open URL).
    pub launcher: String,
    /// Entry point: filename (web/bash) or URL (url launcher).
    pub entry: String,
    /// Suggested panel width in pixels.
    #[serde(default)]
    pub width: Option<u32>,
    /// Suggested panel height in pixels.
    #[serde(default)]
    pub height: Option<u32>,
}


/// A skill's binding for a capability it claims to `provide`. The
/// engine owns the capability's tool names / schemas / tiers (see
/// `engine::capabilities`); this struct tells the engine *where* on the
/// skill's daemon each tool is served, how to autostart it, and where
/// to probe for health.
///
/// One `CapabilityImpl` per capability in the skill's `implements:`
/// block. Example (`skills/memory/SKILL.md`):
///
/// ```yaml
/// provides: [memory]
/// implements:
///   memory:
///     base_url: http://127.0.0.1:9888
///     autostart: "ling-mem start"
///     healthcheck: /api/health
///     tools:
///       Memory_add:    /api/memory/add
///       Memory_search: /api/memory/search
///       ...
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CapabilityImpl {
    /// Root of the skill's HTTP surface. Engine concatenates this with
    /// the tool's path to form the dispatch URL. Include the scheme,
    /// host, and port (e.g. `http://127.0.0.1:9888`).
    pub base_url: String,
    /// Command the engine runs when the daemon isn't reachable on the
    /// first call. Parsed with shell-split semantics. The first token
    /// is resolved against `$SKILL_DIR/bin/` first, then `$PATH`.
    #[serde(default)]
    pub autostart: Option<String>,
    /// Path that returns 200 when the daemon is healthy. Reserved for
    /// the engine's future liveness probe; not consumed today.
    #[serde(default = "default_capability_healthcheck")]
    pub healthcheck: String,
    /// Map from capability tool name (as defined by the engine's
    /// capability contract) → path on the daemon. E.g.
    /// `Memory_search: /api/memory/search`.
    #[serde(default)]
    pub tools: std::collections::HashMap<String, String>,
}

fn default_capability_healthcheck() -> String {
    "/api/health".to_string()
}

/// Permission request declared by a skill. See permission-spec.md → Skill invocation.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SkillPermission {
    /// Required mode: "read", "edit", or "admin".
    pub mode: String,
    /// Paths to grant the mode on. E.g. ["/", "~/workspace"].
    #[serde(default)]
    pub paths: Vec<String>,
    /// Session policy preset when the skill is active: "interactive" | "strict"
    /// | "trusted" | "sandbox". Defaults to "strict" (silently deny out-of-scope
    /// actions — the skill declared its scope, enforce it without user prompts).
    /// Skills that legitimately need to reach beyond their declared paths can
    /// set this to "interactive" to fall back to per-action user prompts.
    #[serde(default)]
    pub policy: Option<String>,
    /// Warning message shown to user before approval.
    #[serde(default)]
    pub warning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source: SkillSource,
    #[serde(default)]
    pub tool_defs: Vec<SkillToolDef>,
    #[serde(default)]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub disable_model_invocation: bool,
    #[serde(default = "default_user_invocable")]
    pub user_invocable: bool,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub app: Option<AppConfig>,
    /// Permission request — if set, user is prompted to approve before skill runs.
    #[serde(default)]
    pub permission: Option<SkillPermission>,
    /// Install script path relative to skill directory. Run once on install.
    #[serde(default)]
    pub install: Option<String>,
    /// Capabilities this skill implements (e.g. `["memory"]`). Linggen core
    /// routes built-in tool families (e.g. `Memory.*`) to whichever installed
    /// skill provides the capability. See `doc/skill-spec.md` and
    /// `doc/memory-spec.md`.
    #[serde(default)]
    pub provides: Option<Vec<String>>,
    /// Bindings for capabilities the skill `provides:`. Keyed by the
    /// capability's name (`"memory"`). One entry per capability this
    /// skill implements. The engine owns each capability's canonical tool
    /// contract (names, schemas, tiers — see `engine::capabilities`); this
    /// map tells the engine *where* on the skill's daemon each tool is
    /// served. Absent on skills that don't implement any capability.
    #[serde(default)]
    pub implements: Option<std::collections::HashMap<String, CapabilityImpl>>,
    /// Filesystem path to the skill directory (set at load time, not serialized to clients).
    #[serde(skip)]
    pub skill_dir: Option<std::path::PathBuf>,
}

fn default_user_invocable() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum SkillSource {
    Global,
    Project,
    Compat { label: String },
}

/// Deserialize `allowed-tools` as either a single string ("Bash") or a list (["Bash", "Read"]).
fn deserialize_string_or_vec<'de, D>(deserializer: D) -> std::result::Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Str(String),
        Vec(Vec<String>),
    }
    let opt: Option<StringOrVec> = Option::deserialize(deserializer)?;
    Ok(match opt {
        Some(StringOrVec::Str(s)) => Some(s.split(',').map(|s| s.trim().to_string()).collect()),
        Some(StringOrVec::Vec(v)) => Some(v),
        None => None,
    })
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    #[serde(default)]
    tools: Vec<SkillToolDef>,
    #[serde(default, rename = "argument-hint")]
    argument_hint: Option<String>,
    #[serde(default, rename = "disable-model-invocation")]
    disable_model_invocation: bool,
    #[serde(default = "default_user_invocable", rename = "user-invocable")]
    user_invocable: bool,
    #[serde(default, rename = "allowed-tools", deserialize_with = "deserialize_string_or_vec")]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
    #[serde(default)]
    app: Option<AppConfig>,
    #[serde(default)]
    permission: Option<SkillPermission>,
    #[serde(default)]
    install: Option<String>,
    #[serde(default)]
    provides: Option<Vec<String>>,
    #[serde(default)]
    implements: Option<std::collections::HashMap<String, CapabilityImpl>>,
}

pub struct SkillManager {
    skills: Mutex<HashMap<String, Skill>>,
    triggers: Mutex<HashMap<String, String>>,
}

impl SkillManager {
    pub fn new() -> Self {
        Self {
            skills: Mutex::new(HashMap::new()),
            triggers: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load_all(&self, project_root: Option<&Path>) -> Result<()> {
        let mut skills = self.skills.lock().await;
        skills.clear();

        // 1. Load Global Skills (~/.linggen/skills/)
        {
            let global_dir = crate::paths::global_skills_dir();
            let _ = self
                .load_from_dir_nested(&global_dir, SkillSource::Global, &mut *skills)
                .await;
        }

        // 2. Load Compat Skills (~/.claude/skills/, ~/.codex/skills/)
        for (compat_dir, label) in crate::paths::compat_skills_dirs() {
            let source = SkillSource::Compat { label: label.to_string() };
            let _ = self
                .load_from_dir_nested(&compat_dir, source, &mut *skills)
                .await;
        }

        // 3. Load Project Skills — highest priority
        //    Scan both .linggen/skills/ and .claude/skills/ (compat) in the project root.
        //    Skip if the project root is the home directory (those are already loaded as Global).
        let home_dir = dirs::home_dir();
        if let Some(root) = project_root {
            let is_home = home_dir.as_deref() == Some(root);
            if !is_home {
                for dir_name in &[".claude/skills", ".codex/skills", ".linggen/skills"] {
                    let project_dir = root.join(dir_name);
                    let _ = self
                        .load_from_dir_nested(&project_dir, SkillSource::Project, &mut *skills)
                        .await;
                }
            }
        }

        // Rebuild trigger index from skills with trigger field set.
        let mut triggers = self.triggers.lock().await;
        triggers.clear();
        for (name, skill) in skills.iter() {
            if let Some(trigger) = &skill.trigger {
                let trigger = trigger.trim().to_string();
                if !trigger.is_empty() {
                    triggers.insert(trigger, name.clone());
                }
            }
        }

        Ok(())
    }

    /// Load skills from a directory, supporting both flat .md files and
    /// subdirectories containing SKILL.md (e.g. `skills/<name>/SKILL.md`).
    async fn load_from_dir_nested(
        &self,
        dir: &Path,
        source: SkillSource,
        skills: &mut HashMap<String, Skill>,
    ) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "md") {
                // Flat .md file
                let text = std::fs::read_to_string(&path)?;
                if let Ok(mut skill) = self.parse_skill(&text, source.clone()) {
                    if let Some(parent) = path.parent() {
                        skill.skill_dir = Some(parent.to_path_buf());
                        for tool_def in &mut skill.tool_defs {
                            tool_def.skill_dir = Some(parent.to_path_buf());
                        }
                    }
                    skills.insert(skill.name.clone(), skill);
                }
            } else if path.is_dir() {
                // Subdirectory: look for SKILL.md inside
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    let text = std::fs::read_to_string(&skill_md)?;
                    if let Ok(mut skill) = self.parse_skill(&text, source.clone()) {
                        skill.skill_dir = Some(path.clone());
                        for tool_def in &mut skill.tool_defs {
                            tool_def.skill_dir = Some(path.clone());
                        }
                        skills.insert(skill.name.clone(), skill);
                    }
                }
            }
        }
        Ok(())
    }

    fn parse_skill(&self, text: &str, source: SkillSource) -> Result<Skill> {
        if !text.starts_with("---") {
            anyhow::bail!("Skill must start with YAML frontmatter");
        }
        let parts: Vec<&str> = text.splitn(3, "---").collect();
        if parts.len() < 3 {
            anyhow::bail!("Skill missing closing frontmatter delimiter");
        }
        let frontmatter: SkillFrontmatter = serde_yml::from_str(parts[1])?;
        let content = parts[2].trim().to_string();

        // Skills with an `app` automatically receive the built-in PageUpdate
        // data tool — the agent calls it to refresh the dashboard UI. Skills
        // without an app don't get it (no UI to update).
        let mut tool_defs = frontmatter.tools;
        if frontmatter.app.is_some() {
            tool_defs.push(page_update_tool_def());
        }

        Ok(Skill {
            name: frontmatter.name,
            description: frontmatter.description,
            content,
            source,
            tool_defs,
            argument_hint: frontmatter.argument_hint,
            disable_model_invocation: frontmatter.disable_model_invocation,
            user_invocable: frontmatter.user_invocable,
            allowed_tools: frontmatter.allowed_tools,
            model: frontmatter.model,
            context: frontmatter.context,
            agent: frontmatter.agent,
            trigger: frontmatter.trigger,
            app: frontmatter.app,
            permission: frontmatter.permission,
            install: frontmatter.install,
            provides: frontmatter.provides,
            implements: frontmatter.implements,
            skill_dir: None,
        })
    }

    /// Match a user message against registered triggers.
    /// Returns (skill_name, remaining_input) if a trigger prefix matches.
    /// Triggers are sorted longest-first so longer prefixes win.
    pub async fn match_trigger(&self, input: &str) -> Option<(String, String)> {
        let triggers = self.triggers.lock().await;
        if triggers.is_empty() {
            return None;
        }
        // Sort triggers longest-first for greedy matching.
        let mut sorted: Vec<(&String, &String)> = triggers.iter().collect();
        sorted.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        for (prefix, skill_name) in sorted {
            if input.starts_with(prefix.as_str()) {
                let remaining = input[prefix.len()..].trim_start().to_string();
                return Some((skill_name.clone(), remaining));
            }
        }
        None
    }

    pub async fn get_skill(&self, name: &str) -> Option<Skill> {
        let skills = self.skills.lock().await;
        skills.get(name).cloned()
    }

    pub async fn list_skills(&self) -> Vec<Skill> {
        let skills = self.skills.lock().await;
        skills.values().cloned().collect()
    }

    /// Test helper: seed the internal skill map so integration tests in
    /// other modules (e.g. `engine::memory::dispatch`) can exercise paths
    /// that require an installed skill without going through the full
    /// `load_all` directory-scan flow.
    #[cfg(test)]
    pub(crate) async fn insert_for_test(&self, skill: Skill) {
        self.skills.lock().await.insert(skill.name.clone(), skill);
    }

    /// Resolve a capability name (e.g. `"memory"`) to the single skill that
    /// advertises `provides: [<capability>]`. Resolution is deterministic:
    /// among skills claiming the capability, a Project-sourced skill beats
    /// Compat, which beats Global; ties break alphabetically by name.
    /// This way, a user's per-project override always wins over a globally
    /// installed default, regardless of HashMap iteration order. Multi-
    /// provider disambiguation (e.g. user-selected active slot) lands with
    /// the marketplace UI; for now, installing two providers of the same
    /// tier is resolved by name.
    pub async fn active_provider(&self, capability: &str) -> Option<Skill> {
        let skills = self.skills.lock().await;
        skills
            .values()
            .filter(|s| {
                s.provides
                    .as_ref()
                    .map(|list| list.iter().any(|c| c == capability))
                    .unwrap_or(false)
            })
            .min_by_key(|s| (source_priority(&s.source), s.name.clone()))
            .cloned()
    }
}

/// Priority for deterministic capability resolution. Lower wins.
fn source_priority(src: &SkillSource) -> u8 {
    match src {
        SkillSource::Project => 0,
        SkillSource::Compat { .. } => 1,
        SkillSource::Global => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager() -> SkillManager {
        SkillManager::new()
    }

    #[test]
    fn test_parse_skill_valid() {
        let mgr = make_manager();
        let text = r#"---
name: test-skill
description: A test skill
---
This is the skill content."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        assert_eq!(skill.name, "test-skill");
        assert_eq!(skill.description, "A test skill");
        assert_eq!(skill.content, "This is the skill content.");
        assert!(skill.user_invocable); // default true
    }

    #[test]
    fn test_parse_skill_no_frontmatter() {
        let mgr = make_manager();
        let err = mgr
            .parse_skill("no frontmatter here", SkillSource::Global)
            .unwrap_err();
        assert!(err.to_string().contains("YAML frontmatter"));
    }

    #[test]
    fn test_parse_skill_missing_closing() {
        let mgr = make_manager();
        let err = mgr
            .parse_skill("---\nname: x\ndescription: y\n", SkillSource::Global)
            .unwrap_err();
        assert!(err.to_string().contains("closing frontmatter"));
    }

    #[test]
    fn test_parse_skill_with_provides() {
        let mgr = make_manager();
        let text = r#"---
name: linggen-memory
description: Semantic memory
provides: [memory]
---
Body."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        assert_eq!(skill.provides.as_deref(), Some(&["memory".to_string()][..]));
    }

    #[test]
    fn test_parse_skill_with_implements() {
        let mgr = make_manager();
        let text = r#"---
name: memory
description: Semantic memory
provides: [memory]
implements:
  memory:
    base_url: http://127.0.0.1:9888
    autostart: "ling-mem start"
    healthcheck: /api/health
    tools:
      Memory_add:    /api/memory/add
      Memory_search: /api/memory/search
---
Body."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        let impls = skill.implements.as_ref().expect("implements block parsed");
        let mem = impls.get("memory").expect("memory binding present");
        assert_eq!(mem.base_url, "http://127.0.0.1:9888");
        assert_eq!(mem.autostart.as_deref(), Some("ling-mem start"));
        assert_eq!(mem.healthcheck, "/api/health");
        assert_eq!(mem.tools.get("Memory_add").map(String::as_str), Some("/api/memory/add"));
        assert_eq!(mem.tools.get("Memory_search").map(String::as_str), Some("/api/memory/search"));
    }

    #[test]
    fn test_parse_skill_implements_healthcheck_defaults() {
        let mgr = make_manager();
        let text = r#"---
name: mem-min
description: Minimal implements block
provides: [memory]
implements:
  memory:
    base_url: http://127.0.0.1:9999
---
Body."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        let mem = skill.implements.unwrap().remove("memory").unwrap();
        assert_eq!(mem.healthcheck, "/api/health");
    }

    #[tokio::test]
    async fn test_active_provider_returns_matching_skill() {
        let mgr = make_manager();
        let text = r#"---
name: mem-impl
description: impl
provides: [memory]
---
Body."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        mgr.skills.lock().await.insert(skill.name.clone(), skill);
        assert_eq!(
            mgr.active_provider("memory").await.map(|s| s.name),
            Some("mem-impl".to_string())
        );
        assert!(mgr.active_provider("search").await.is_none());
    }

    #[test]
    fn test_parse_skill_with_trigger() {
        let mgr = make_manager();
        let text = r#"---
name: commit
description: Commit helper
trigger: "/commit"
---
Help commit."#;
        let skill = mgr.parse_skill(text, SkillSource::Project).unwrap();
        assert_eq!(skill.trigger.as_deref(), Some("/commit"));
    }

    #[test]
    fn test_parse_skill_with_all_optional_fields() {
        let mgr = make_manager();
        let text = r#"---
name: advanced
description: Advanced skill
user-invocable: false
disable-model-invocation: true
argument-hint: "project name"
allowed-tools: [Read, Write]
model: gpt-4
context: my-context
agent: coder
---
Content."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        assert!(!skill.user_invocable);
        assert!(skill.disable_model_invocation);
        assert_eq!(skill.argument_hint.as_deref(), Some("project name"));
        assert_eq!(skill.allowed_tools, Some(vec!["Read".to_string(), "Write".to_string()]));
        assert_eq!(skill.model.as_deref(), Some("gpt-4"));
        assert_eq!(skill.context.as_deref(), Some("my-context"));
        assert_eq!(skill.agent.as_deref(), Some("coder"));
    }

    #[tokio::test]
    async fn test_match_trigger_empty() {
        let mgr = make_manager();
        assert!(mgr.match_trigger("hello").await.is_none());
    }

    #[tokio::test]
    async fn test_match_trigger_basic() {
        let mgr = make_manager();
        {
            let mut triggers = mgr.triggers.lock().await;
            triggers.insert("/commit".to_string(), "commit-skill".to_string());
        }
        let result = mgr.match_trigger("/commit fix bug").await;
        assert!(result.is_some());
        let (name, remaining) = result.unwrap();
        assert_eq!(name, "commit-skill");
        assert_eq!(remaining, "fix bug");
    }

    #[tokio::test]
    async fn test_match_trigger_longest_wins() {
        let mgr = make_manager();
        {
            let mut triggers = mgr.triggers.lock().await;
            triggers.insert("/c".to_string(), "short".to_string());
            triggers.insert("/commit".to_string(), "long".to_string());
        }
        let result = mgr.match_trigger("/commit message").await;
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "long");
    }

    #[tokio::test]
    async fn test_match_trigger_no_match() {
        let mgr = make_manager();
        {
            let mut triggers = mgr.triggers.lock().await;
            triggers.insert("/commit".to_string(), "commit-skill".to_string());
        }
        assert!(mgr.match_trigger("hello world").await.is_none());
    }

    #[tokio::test]
    async fn test_load_from_dir_nested() {
        let dir = tempfile::tempdir().unwrap();

        // Create a flat .md skill
        let flat_skill = r#"---
name: flat-skill
description: Flat skill
---
Flat content."#;
        std::fs::write(dir.path().join("flat.md"), flat_skill).unwrap();

        // Create a nested skill
        let nested_dir = dir.path().join("nested-skill");
        std::fs::create_dir(&nested_dir).unwrap();
        let nested_skill = r#"---
name: nested-skill
description: Nested skill
---
Nested content."#;
        std::fs::write(nested_dir.join("SKILL.md"), nested_skill).unwrap();

        let mgr = make_manager();
        let mut skills = std::collections::HashMap::new();
        mgr.load_from_dir_nested(dir.path(), SkillSource::Global, &mut skills)
            .await
            .unwrap();

        assert_eq!(skills.len(), 2);
        assert!(skills.contains_key("flat-skill"));
        assert!(skills.contains_key("nested-skill"));
    }

    #[test]
    fn test_parse_skill_with_app_config() {
        let mgr = make_manager();
        let text = r#"---
name: arcade-game
description: Retro arcade games
app:
  launcher: web
  entry: index.html
  width: 800
  height: 600
---
Play games."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        assert_eq!(skill.name, "arcade-game");
        let app = skill.app.unwrap();
        assert_eq!(app.launcher, "web");
        assert_eq!(app.entry, "index.html");
        assert_eq!(app.width, Some(800));
        assert_eq!(app.height, Some(600));
    }

    #[test]
    fn test_parse_skill_without_app_config() {
        let mgr = make_manager();
        let text = r#"---
name: normal-skill
description: A normal skill
---
No app."#;
        let skill = mgr.parse_skill(text, SkillSource::Global).unwrap();
        assert!(skill.app.is_none());
    }

    #[test]
    fn test_parse_frontmatter_meta() {
        let text = "---\nname: my-skill\ndescription: A sample skill\n---\nContent here.";
        let (name, desc) = parse_frontmatter_meta(text).unwrap();
        assert_eq!(name, "my-skill");
        assert_eq!(desc, "A sample skill");
    }

    #[test]
    fn test_parse_frontmatter_meta_no_frontmatter() {
        assert!(parse_frontmatter_meta("no frontmatter").is_none());
    }

    #[test]
    fn test_parse_frontmatter_meta_missing_closing() {
        assert!(parse_frontmatter_meta("---\nname: x\ndescription: y\n").is_none());
    }
}
