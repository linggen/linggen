use crate::config::Config;
use crate::skills::marketplace::{self, SkillScope};
use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum SkillsAction {
    Add {
        name: String,
        repo: Option<String>,
        git_ref: Option<String>,
        global: bool,
        force: bool,
        source: Option<String>,
    },
    Remove {
        name: String,
        global: bool,
    },
    List,
    Search {
        query: String,
    },
}

pub async fn run(action: SkillsAction, _config: &Config) -> Result<()> {
    match action {
        SkillsAction::Add {
            name,
            repo,
            git_ref,
            global,
            force,
            source,
        } => {
            let scope = if global {
                SkillScope::Global
            } else {
                SkillScope::Project
            };
            let project_root = if !global {
                Some(crate::workspace::resolve_workspace_root(None)?)
            } else {
                None
            };
            let target_dir =
                marketplace::skill_target_dir(&name, scope, project_root.as_deref())?;

            let msg = marketplace::install_skill(
                &name,
                repo.as_deref(),
                git_ref.as_deref(),
                &target_dir,
                force,
                source.as_deref(),
            )
            .await?;
            println!("{}", msg);
        }
        SkillsAction::Remove { name, global } => {
            let scope = if global {
                SkillScope::Global
            } else {
                SkillScope::Project
            };
            let project_root = if !global {
                Some(crate::workspace::resolve_workspace_root(None)?)
            } else {
                None
            };
            let target_dir =
                marketplace::skill_target_dir(&name, scope, project_root.as_deref())?;

            let msg = marketplace::delete_skill(&name, &target_dir)?;
            println!("{}", msg);
        }
        SkillsAction::List => {
            println!("Installed skills:\n");
            let mut found = false;

            let dirs_to_scan: Vec<(PathBuf, &str)> = [
                Some((crate::paths::global_skills_dir(), "global")),
                crate::workspace::resolve_workspace_root(None)
                    .ok()
                    .map(|ws| (ws.join(".linggen/skills"), "project")),
            ]
            .into_iter()
            .flatten()
            .collect();

            for (dir, source) in dirs_to_scan {
                if !dir.exists() {
                    continue;
                }
                let entries = match std::fs::read_dir(&dir) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?");

                    // Check if it's a skill directory (has SKILL.md) or a .md file
                    let is_skill = if path.is_dir() {
                        path.join("SKILL.md").exists()
                    } else {
                        path.extension().map_or(false, |e| e == "md")
                    };

                    if is_skill {
                        println!("  {:30} ({})", name, source);
                        found = true;
                    }
                }
            }

            if !found {
                println!("  (none)");
            }
        }
        SkillsAction::Search { query } => {
            println!("Searching community skills for '{}' ...\n", query);
            let results = marketplace::search_community(&query).await?;
            if results.is_empty() {
                println!("  No results found.");
                return Ok(());
            }
            println!(
                "  {:<30} {:<50} {}",
                "NAME", "URL", "DESCRIPTION"
            );
            println!("  {}", "-".repeat(100));
            for skill in &results {
                let desc = skill
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>();
                println!(
                    "  {:<30} {:<50} {}",
                    skill.name, skill.url, desc
                );
            }
            println!("\n  {} result(s)", results.len());
        }
    }

    Ok(())
}
