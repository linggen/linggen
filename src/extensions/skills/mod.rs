pub use crate::extensions::marketplace;

use crate::engine::skill_tool::{SkillParamDef, SkillToolDef};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
            // Shape varies per skill — keep param_type permissive so the LLM
            // can pick whichever the active skill teaches.
            param_type: "array".to_string(),
            required: false,
            default: None,
            description:
                "Optional top-of-dashboard section. Shape is defined by the active \
                 skill's SKILL.md."
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
                "Optional footer section. Shape is defined by the active skill's \
                 SKILL.md."
                    .to_string(),
            items: None,
        },
    );
    args.insert(
        "body_patch".to_string(),
        SkillParamDef {
            // Some skills use an array of selector patches (`[{match, widget}]`),
            // others use a single object keyed by section (`{section, cards}`).
            // Leave param_type as `array` for back-compat with existing
            // selector-style skills; the renderer normalizes either shape.
            param_type: "array".to_string(),
            required: false,
            default: None,
            description:
                "Partial body update — preserves widgets the patch doesn't touch. \
                 Shape is defined by the active skill's SKILL.md (some skills use \
                 selector patches, others key by section)."
                    .to_string(),
            items: None,
        },
    );
    SkillToolDef {
        name: "PageUpdate".to_string(),
        description:
            "Refresh the skill's dashboard UI. Pass `top_bar`, `body`, `footer`, \
             and/or `body_patch` as top-level arguments — omit any section you \
             don't want to change (previous values persist). At least one must be \
             provided and non-empty. **All payload shapes (widgets, patch entries, \
             section keys) are defined by the active skill's SKILL.md — follow that \
             contract, not a default schema.**"
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

/// GitHub-derived metadata for a single built-in skill. Cached for `CACHE_TTL`
/// because GitHub responses are stable; the `installed` flag (which depends on
/// local filesystem state) is recomputed per request from this metadata.
#[derive(Debug, Clone)]
struct BuiltInRemote {
    dir_name: String,
    name: String,
    description: String,
}

type BuiltInCache = Mutex<Option<(Instant, Vec<BuiltInRemote>)>>;

fn builtin_cache() -> &'static BuiltInCache {
    static CACHE: OnceLock<BuiltInCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Clears the built-in skills cache so the next fetch hits GitHub.
pub async fn clear_builtin_cache() {
    let mut cache = builtin_cache().lock().await;
    *cache = None;
}

fn project_installed(remotes: &[BuiltInRemote]) -> Vec<BuiltInSkillInfo> {
    let global_skills_dir = crate::paths::global_skills_dir();
    remotes
        .iter()
        .map(|r| BuiltInSkillInfo {
            name: r.name.clone(),
            description: r.description.clone(),
            installed: global_skills_dir.join(&r.dir_name).join("SKILL.md").exists(),
        })
        .collect()
}

/// Fetches the list of built-in skills from the linggen/skills GitHub repo.
/// GitHub metadata is cached for 10 minutes; the `installed` flag is recomputed
/// from the local filesystem on each call so install/uninstall reflects
/// immediately. On GitHub fetch error, returns stale cache or empty vec.
pub async fn fetch_builtin_skills() -> Vec<BuiltInSkillInfo> {
    // Check cache for the GitHub metadata.
    {
        let cache = builtin_cache().lock().await;
        if let Some((ts, ref remotes)) = *cache {
            if ts.elapsed() < CACHE_TTL {
                return project_installed(remotes);
            }
        }
    }

    // Fetch from GitHub
    match fetch_builtin_skills_inner().await {
        Ok(remotes) => {
            let mut cache = builtin_cache().lock().await;
            *cache = Some((Instant::now(), remotes.clone()));
            project_installed(&remotes)
        }
        Err(e) => {
            tracing::warn!(err = %e, "Failed to fetch built-in skills from GitHub");
            // Return stale cache if available, with installed flag refreshed.
            let cache = builtin_cache().lock().await;
            if let Some((_, ref remotes)) = *cache {
                return project_installed(remotes);
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

async fn fetch_builtin_skills_inner() -> Result<Vec<BuiltInRemote>> {
    let client = super::marketplace::http_client()?;

    // List repo contents
    let entries: Vec<GitHubContentEntry> = client
        .get(GITHUB_CONTENTS_URL)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

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
            Some(BuiltInRemote { dir_name: entry.name.clone(), name, description })
        }
    });
    let results = futures_util::future::join_all(fetches).await;

    Ok(results.into_iter().flatten().collect())
}

/// Run a skill's install script if declared in frontmatter.
/// The script runs with `$SKILL_DIR` set to the skill directory.
/// Returns Ok(Some(output)) if a script ran, Ok(None) if no script declared.
pub fn run_install_script(skill_dir: &Path) -> Result<Option<String>> {
    let skill_md = skill_dir.join("SKILL.md");
    let text = std::fs::read_to_string(&skill_md)?;

    let Some(yaml) = crate::extensions::frontmatter::split(&text).0 else {
        return Ok(None);
    };

    #[derive(Deserialize)]
    struct InstallMeta {
        #[serde(default)]
        install: Option<String>,
    }

    let meta: InstallMeta = match serde_yml::from_str(yaml) {
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

    let output = crate::extensions::script::sync_command(
        crate::extensions::script::Invocation::File(&script_path),
        skill_dir,
        &[("SKILL_DIR", skill_dir.as_os_str())],
    )
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
    crate::extensions::frontmatter::parse_meta(text)
}

/// Runtime skill record types live in `engine::skill` — re-exported
/// for callers that import them through this module. Permission grant
/// types live in `engine::permission`.
pub use crate::engine::permission::{Grants as SkillPermission, PathGrant};
pub use crate::engine::skill::{AppConfig, Skill, SkillSource};

use crate::extensions::frontmatter::deserialize_string_or_vec;

fn default_user_invocable() -> bool {
    true
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
    #[serde(default, rename = "allow-skills", deserialize_with = "deserialize_string_or_vec")]
    allow_skills: Option<Vec<String>>,
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
    /// Optional starting cwd for sessions invoking this skill. Tilde-expandable
    /// (e.g. `~/.linggen`). When set, sessions created by this skill's app
    /// launcher (or the slash-command activation path) start at this folder
    /// instead of the user's home_path. Mirrors the `cwd:` field in mission
    /// frontmatter. When omitted, defaults to the user's `home_path` (or
    /// whatever the iframe's session-create call passes).
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    install: Option<String>,
}

pub struct SkillLoader {
    skills: Mutex<HashMap<String, Skill>>,
    triggers: Mutex<HashMap<String, String>>,
}

impl SkillLoader {
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
    /// Each entry is loaded through [`load_skill_from_path`], so the
    /// file-vs-dir branching and `skill_dir` plumbing stay in one place.
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
            match load_skill_from_path(&path, source.clone()) {
                Ok(Some(skill)) => {
                    // Compat entries (~/.claude/skills/, ~/.codex/skills/)
                    // are typically thin per-host stubs pointing back to the
                    // canonical Global bundle at ~/.linggen/skills/. If the
                    // Global copy is already loaded, don't let the stub
                    // shadow it — the stub dir lacks scripts/, references/,
                    // etc., so web-launcher apps would 404 on bundle files.
                    if matches!(skill.source, SkillSource::Compat { .. }) {
                        if let Some(existing) = skills.get(&skill.name) {
                            if matches!(existing.source, SkillSource::Global) {
                                continue;
                            }
                        }
                    }
                    skills.insert(skill.name.clone(), skill);
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Failed to load skill");
                }
            }
        }
        Ok(())
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

}

/// `SkillRegistry` impl — the engine reaches `SkillLoader` only through
/// this trait so it never imports an extension type.
#[async_trait::async_trait]
impl crate::engine::skill::registry::SkillRegistry for SkillLoader {
    async fn get(&self, name: &str) -> Option<Skill> {
        self.get_skill(name).await
    }

    async fn list_metadata(&self) -> Vec<(String, String)> {
        self.list_skills()
            .await
            .into_iter()
            .filter(|s| !s.disable_model_invocation)
            .map(|s| (s.name, s.description))
            .collect()
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

/// Parse a SKILL.md text into a [`Skill`]. Pure function — no I/O. Use
/// [`load_skill_from_path`] when you have a filesystem path.
pub fn parse_skill_text(text: &str, source: SkillSource) -> Result<Skill> {
    let (yaml_opt, body) = crate::extensions::frontmatter::split(text);
    let yaml = match yaml_opt {
        Some(y) => y,
        None if text.starts_with("---") => {
            anyhow::bail!("Skill missing closing frontmatter delimiter")
        }
        None => anyhow::bail!("Skill must start with YAML frontmatter"),
    };
    let frontmatter: SkillFrontmatter = serde_yml::from_str(yaml)?;
    let content = body.trim().to_string();

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
        allow_skills: frontmatter.allow_skills,
        model: frontmatter.model,
        context: frontmatter.context,
        agent: frontmatter.agent,
        trigger: frontmatter.trigger,
        app: frontmatter.app,
        permission: frontmatter.permission,
        cwd: frontmatter.cwd,
        install: frontmatter.install,
        skill_dir: None,
    })
}

/// Read a SKILL.md from `path` and parse it into a [`Skill`].
///
/// Accepts either:
/// - a `.md` file (`skill_dir` set to its parent directory), or
/// - a directory containing `SKILL.md` (`skill_dir` set to the directory).
///
/// Returns `Ok(None)` when the path doesn't look like a skill (no `.md`
/// extension and no `SKILL.md` inside). Use this from any callsite that
/// needs to load a skill from disk — keeps the file/dir branching and the
/// `skill_dir` plumbing in one place.
pub fn load_skill_from_path(path: &Path, source: SkillSource) -> Result<Option<Skill>> {
    if path.is_file() && path.extension().map_or(false, |e| e == "md") {
        let text = std::fs::read_to_string(path)?;
        let mut skill = parse_skill_text(&text, source)?;
        if let Some(parent) = path.parent() {
            attach_skill_dir(&mut skill, parent.to_path_buf());
        }
        return Ok(Some(skill));
    }
    if path.is_dir() {
        let skill_md = path.join("SKILL.md");
        if skill_md.exists() {
            let text = std::fs::read_to_string(&skill_md)?;
            let mut skill = parse_skill_text(&text, source)?;
            attach_skill_dir(&mut skill, path.to_path_buf());
            return Ok(Some(skill));
        }
    }
    Ok(None)
}

/// Stamp the resolved skill directory onto the skill *and* every tool def it
/// declared, so dispatch can find the skill's daemon without a second lookup.
fn attach_skill_dir(skill: &mut Skill, dir: PathBuf) {
    for tool_def in &mut skill.tool_defs {
        tool_def.skill_dir = Some(dir.clone());
    }
    skill.skill_dir = Some(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager() -> SkillLoader {
        SkillLoader::new()
    }

    #[test]
    fn test_parse_skill_valid() {
        let text = r#"---
name: test-skill
description: A test skill
---
This is the skill content."#;
        let skill = parse_skill_text(text, SkillSource::Global).unwrap();
        assert_eq!(skill.name, "test-skill");
        assert_eq!(skill.description, "A test skill");
        assert_eq!(skill.content, "This is the skill content.");
        assert!(skill.user_invocable); // default true
    }

    #[test]
    fn test_parse_skill_no_frontmatter() {
        let err = parse_skill_text("no frontmatter here", SkillSource::Global).unwrap_err();
        assert!(err.to_string().contains("YAML frontmatter"));
    }

    #[test]
    fn test_parse_skill_missing_closing() {
        let err = parse_skill_text("---\nname: x\ndescription: y\n", SkillSource::Global)
            .unwrap_err();
        assert!(err.to_string().contains("closing frontmatter"));
    }

    #[test]
    fn test_parse_skill_with_trigger() {
        let text = r#"---
name: commit
description: Commit helper
trigger: "/commit"
---
Help commit."#;
        let skill = parse_skill_text(text, SkillSource::Project).unwrap();
        assert_eq!(skill.trigger.as_deref(), Some("/commit"));
    }

    #[test]
    fn test_parse_skill_with_all_optional_fields() {
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
        let skill = parse_skill_text(text, SkillSource::Global).unwrap();
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
        let mgr = make_manager();
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
        let skill = parse_skill_text(text, SkillSource::Global).unwrap();
        assert_eq!(skill.name, "arcade-game");
        let app = skill.app.unwrap();
        assert_eq!(app.launcher, "web");
        assert_eq!(app.entry, "index.html");
        assert_eq!(app.width, Some(800));
        assert_eq!(app.height, Some(600));
    }

    #[test]
    fn test_parse_skill_without_app_config() {
        let text = r#"---
name: normal-skill
description: A normal skill
---
No app."#;
        let skill = parse_skill_text(text, SkillSource::Global).unwrap();
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
