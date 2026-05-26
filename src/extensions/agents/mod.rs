//! Disk loader for agent specs.
//!
//! Reads `<project>/agents/*.md` and `~/.linggen/agents/*.md`, parses
//! their YAML frontmatter into `engine::agent::AgentSpec`, and merges
//! the two directories (project overrides global by `agent_id`).
//! Production implementer of `engine::agent::AgentRegistry`.
//!
//! Mirrors `extensions::skills::SkillManager`: a thin, stateless
//! adapter that owns the `---` frontmatter splitter and the
//! directory-walking rules. The engine never touches `std::fs` for
//! agent loading — it goes through this module.

use crate::engine::agent::registry::AgentRegistry;
use crate::engine::agent::spec::{AgentSpec, AgentSpecFile};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

/// Parse the `---`-delimited YAML frontmatter at the top of an agent
/// markdown file, returning the typed spec and the trimmed body
/// (which the engine uses verbatim as the agent's system prompt).
pub fn parse_agent_markdown(content: &str) -> Result<(AgentSpec, String)> {
    if !content.starts_with("---") {
        anyhow::bail!("Agent spec must start with YAML frontmatter (---)");
    }
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        anyhow::bail!("Agent spec missing closing frontmatter delimiter (---)");
    }
    let spec: AgentSpec = serde_yml::from_str(parts[1])?;
    let system_prompt = parts[2].trim().to_string();
    Ok((spec, system_prompt))
}

/// Same as `parse_agent_markdown`, but reads the file at `path` and
/// annotates parse errors with the offending path.
pub fn parse_agent_file(path: &Path) -> Result<(AgentSpec, String)> {
    let content = fs::read_to_string(path)?;
    parse_agent_markdown(&content)
        .map_err(|e| anyhow::anyhow!("Agent spec at {:?} is invalid: {}", path, e))
}

fn normalize_agent_id(agent_id: &str) -> String {
    agent_id.trim().to_lowercase()
}

fn agent_specs_dir(project_root: &Path) -> PathBuf {
    project_root.join("agents")
}

/// Load every valid agent spec out of a single directory. Invalid
/// files are warned about and skipped; duplicate `agent_id`s within
/// the same directory keep the first one encountered (paths sorted
/// by file name).
fn load_specs_from_dir(agents_dir: &Path) -> Vec<AgentSpecFile> {
    if !agents_dir.exists() {
        return Vec::new();
    }

    let mut paths: Vec<PathBuf> = match fs::read_dir(agents_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("md"))
                        .unwrap_or(false)
            })
            .collect(),
        Err(err) => {
            warn!("Cannot read agents directory {}: {}", agents_dir.display(), err);
            return Vec::new();
        }
    };
    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    let mut seen = HashSet::new();
    let mut specs = Vec::new();
    for spec_path in paths {
        let (spec, system_prompt) = match parse_agent_file(&spec_path) {
            Ok(parsed) => parsed,
            Err(err) => {
                warn!(
                    "Skipping invalid agent spec {}: {}",
                    spec_path.display(),
                    err
                );
                continue;
            }
        };
        let raw_name = spec.name.trim();
        let fallback = spec_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("agent");
        let agent_id = normalize_agent_id(if raw_name.is_empty() {
            fallback
        } else {
            raw_name
        });
        if agent_id.is_empty() {
            warn!(
                "Skipping agent spec {}: resolved to empty agent id",
                spec_path.display()
            );
            continue;
        }
        if !seen.insert(agent_id.clone()) {
            warn!(
                "Skipping agent spec {}: duplicate agent id '{}' in agents directory {}",
                spec_path.display(),
                agent_id,
                agents_dir.display()
            );
            continue;
        }
        specs.push(AgentSpecFile {
            agent_id,
            spec,
            spec_path,
            system_prompt,
        });
    }

    specs
}

/// Layered global+project load. `~/.linggen/agents/` provides the
/// baseline; `<project>/agents/` overrides by `agent_id`. Sorted by
/// `agent_id` for stable ordering.
fn load_specs_for_project(project_root: &Path) -> Result<Vec<AgentSpecFile>> {
    let mut merged: HashMap<String, AgentSpecFile> = HashMap::new();

    let global_dir = crate::paths::global_agents_dir();
    for spec in load_specs_from_dir(&global_dir) {
        merged.insert(spec.agent_id.clone(), spec);
    }

    let project_dir = agent_specs_dir(project_root);
    if project_dir.is_dir() {
        for spec in load_specs_from_dir(&project_dir) {
            merged.insert(spec.agent_id.clone(), spec);
        }
    }

    let mut specs: Vec<AgentSpecFile> = merged.into_values().collect();
    specs.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    Ok(specs)
}

/// Production loader. Stateless — every call re-reads from disk so
/// edits to `agents/*.md` show up without a restart, matching the
/// behavior of `SkillManager` (which has its own caching layer but
/// also supports refresh).
#[derive(Default)]
pub struct AgentSpecLoader;

impl AgentSpecLoader {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AgentRegistry for AgentSpecLoader {
    async fn list(&self, project_root: &Path) -> Result<Vec<AgentSpecFile>> {
        let project_root = crate::util::resolve_path(project_root);
        load_specs_for_project(&project_root)
    }

    async fn find(&self, project_root: &Path, agent_id: &str) -> Result<Option<AgentSpecFile>> {
        let wanted = normalize_agent_id(agent_id);
        let project_root = crate::util::resolve_path(project_root);
        Ok(load_specs_for_project(&project_root)?
            .into_iter()
            .find(|entry| entry.agent_id == wanted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(prefix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!("linggen-{prefix}-{}-{ts}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp root");
        dir
    }

    fn valid_agent_md(name: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: test agent\ntools: [Read]\npolicy: []\n---\n\nYou are {name}.\n"
        )
    }

    #[test]
    fn parse_agent_markdown_valid() {
        let md = r#"---
name: ling
description: General-purpose assistant
tools:
  - Read
  - Write
  - Bash
---
You are a helpful assistant."#;
        let (spec, prompt) = parse_agent_markdown(md).unwrap();
        assert_eq!(spec.name, "ling");
        assert_eq!(spec.description, "General-purpose assistant");
        assert_eq!(spec.tools, vec!["Read", "Write", "Bash"]);
        assert_eq!(prompt, "You are a helpful assistant.");
    }

    #[test]
    fn parse_agent_markdown_missing_frontmatter() {
        let md = "Just a regular markdown file";
        let err = parse_agent_markdown(md).unwrap_err();
        assert!(err.to_string().contains("must start with YAML frontmatter"));
    }

    #[test]
    fn parse_agent_markdown_missing_closing_delimiter() {
        let md = "---\nname: test\ndescription: test\ntools: []\n";
        let err = parse_agent_markdown(md).unwrap_err();
        assert!(err.to_string().contains("missing closing frontmatter"));
    }

    #[test]
    fn parse_agent_markdown_ignores_unknown_fields() {
        let md = r#"---
name: coder
description: Implementation agent
tools: [Read, Write, Edit]
---
Write code."#;
        let (spec, _) = parse_agent_markdown(md).unwrap();
        assert_eq!(spec.name, "coder");
        assert_eq!(spec.tools, vec!["Read", "Write", "Edit"]);
    }

    #[test]
    fn parse_agent_markdown_with_personality() {
        let md = r#"---
name: ling
description: Personal assistant
tools: ["*"]
personality: |
  Concise and direct.
  Confident but honest.
---
You are Ling."#;
        let (spec, prompt) = parse_agent_markdown(md).unwrap();
        assert_eq!(spec.name, "ling");
        assert!(spec.personality.is_some());
        assert!(spec.personality.as_ref().unwrap().contains("Concise and direct"));
        assert_eq!(prompt, "You are Ling.");
    }

    #[test]
    fn parse_agent_markdown_without_personality() {
        let md = r#"---
name: test
description: No personality
tools: [Read]
---
Prompt."#;
        let (spec, _) = parse_agent_markdown(md).unwrap();
        assert!(spec.personality.is_none());
    }

    #[test]
    fn parse_agent_markdown_ignores_unknown_frontmatter_fields() {
        let md = r#"---
name: ling
description: Lead agent
tools: [Read, Glob]
idle_prompt: "Some old field"
idle_interval_secs: 60
---
You are the lead."#;
        let (spec, _) = parse_agent_markdown(md).unwrap();
        assert_eq!(spec.name, "ling");
    }

    #[test]
    fn load_specs_skips_invalid_files_and_duplicates() {
        let root = temp_root("agent-specs");
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).expect("create agents dir");

        fs::write(agents_dir.join("a.md"), valid_agent_md("alpha")).expect("write alpha");
        fs::write(agents_dir.join("bad.md"), "this is not frontmatter").expect("write bad");
        fs::write(agents_dir.join("z.md"), valid_agent_md("alpha")).expect("write duplicate");

        let specs = load_specs_for_project(&root).expect("load agent specs");
        assert!(
            !specs.iter().any(|s| s.spec_path.ends_with("bad.md")),
            "invalid file should be skipped"
        );
        let alpha_specs: Vec<_> = specs.iter().filter(|s| s.agent_id == "alpha").collect();
        assert_eq!(alpha_specs.len(), 1, "duplicate agent_id should be deduplicated");
        assert!(alpha_specs[0].spec_path.ends_with("a.md") || alpha_specs[0].spec_path.ends_with("z.md"));

        let _ = fs::remove_dir_all(&root);
    }
}
