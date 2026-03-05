use crate::skills::marketplace;
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Default agent specs shipped with linggen.
const DEFAULT_AGENTS: &[(&str, &str)] = &[
    ("ling.md", include_str!("../../agents/ling.md")),
    ("coder.md", include_str!("../../agents/coder.md")),
    ("debugger.md", include_str!("../../agents/debugger.md")),
    ("explorer.md", include_str!("../../agents/explorer.md")),
    ("general.md", include_str!("../../agents/general.md")),
    ("linggen-guide.md", include_str!("../../agents/linggen-guide.md")),
];

pub async fn run(global: bool, root: Option<PathBuf>) -> Result<()> {
    let target_dir = if global {
        crate::paths::global_skills_dir()
    } else {
        let ws = root
            .or_else(|| crate::workspace::resolve_workspace_root(None).ok())
            .ok_or_else(|| anyhow::anyhow!("Could not determine workspace root"))?;
        ws.join(".linggen/skills")
    };

    println!("Installing skills to {} ...", target_dir.display());

    let (owner, repo) = ("linggen", "skills");
    let zip_url = marketplace::build_github_zip_url(owner, repo, "main");
    let client = marketplace::http_client()?;
    let temp_zip = marketplace::download_to_temp(&client, &zip_url)
        .await
        .context("Failed to download skills repository")?;

    let installed = marketplace::extract_all_skills_from_zip(&temp_zip, &target_dir)
        .context("Failed to extract skills")?;

    let _ = std::fs::remove_file(&temp_zip);

    if installed.is_empty() {
        println!("No skills found in repository.");
    } else {
        println!("Installed {} skills:", installed.len());
        for name in &installed {
            println!("  - {}", name);
        }
    }

    // Install default agent specs to ~/.linggen/agents/ if they don't already exist.
    install_default_agents()?;

    Ok(())
}

/// Install (or update) built-in agent specs to `~/.linggen/agents/`.
/// Always overwrites to keep agents in sync with the binary version.
pub fn install_default_agents() -> Result<()> {
    let agents_dir = crate::paths::global_agents_dir();
    std::fs::create_dir_all(&agents_dir)?;

    for (filename, content) in DEFAULT_AGENTS {
        let dest = agents_dir.join(filename);
        std::fs::write(&dest, content)?;
    }

    Ok(())
}
