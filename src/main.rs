mod agent_manager;
#[allow(dead_code)]
mod check;
mod cli;
mod config;
mod credentials;
mod engine;
mod eval;
mod logging;
mod ollama;
mod openai;
mod paths;
mod prompts;
mod project_store;
mod server;
mod tui;
mod skills;
mod state_fs;
mod tui_client;
mod util;
mod workspace;

use crate::config::Config;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "ling", version)]
#[command(about = "Linggen — AI coding agent", long_about = None)]
struct Cli {
    /// Workspace root. If omitted, detects by walking up for .git.
    #[arg(long, global = true)]
    root: Option<std::path::PathBuf>,

    /// Port for the server
    #[arg(long, global = true)]
    port: Option<u16>,

    /// Web UI only, no TUI
    #[arg(long, default_value_t = false)]
    web: bool,

    /// Run as background daemon
    #[arg(short, long, default_value_t = false)]
    daemon: bool,

    /// Enable dev mode (proxy static assets from Vite dev server)
    #[arg(long, default_value_t = false)]
    dev: bool,

    #[command(subcommand)]
    cmd: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Stop background daemon
    Stop,
    /// Show agent server status
    Status,
    /// Diagnose installation health
    Doctor,
    /// Run eval tasks against the agent
    Eval {
        /// Filter tasks by name (substring match)
        #[arg(long)]
        filter: Option<String>,

        /// Override max iterations per task
        #[arg(long)]
        max_iters: Option<usize>,

        /// Per-task timeout in seconds (default 300)
        #[arg(long, default_value_t = 300)]
        timeout: u64,

        /// Override agent_id for all tasks
        #[arg(long)]
        agent: Option<String>,

        /// Print agent messages during execution
        #[arg(long, default_value_t = false)]
        verbose: bool,
    },
    /// Install all skills from linggen/skills repository
    Init {
        /// Install to global ~/.linggen/skills/ instead of project
        #[arg(long, default_value_t = false)]
        global: bool,
    },
    /// Install/update the ling binary to latest
    Install,
    /// Update the ling binary to latest
    Update,
    /// Manage skills
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
}

#[derive(Subcommand, Debug)]
enum SkillsAction {
    /// Install a skill
    Add {
        /// Skill name
        name: String,

        /// GitHub repository URL
        #[arg(long)]
        repo: Option<String>,

        /// Git ref (branch/tag)
        #[arg(long, alias = "ref")]
        git_ref: Option<String>,

        /// Install globally (~/.linggen/skills/)
        #[arg(long, default_value_t = false)]
        global: bool,

        /// Overwrite existing installation
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Remove an installed skill
    Remove {
        /// Skill name
        name: String,

        /// Remove from global scope
        #[arg(long, default_value_t = false)]
        global: bool,
    },
    /// List installed skills
    List,
    /// Search the marketplace
    Search {
        /// Search query
        query: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let (config, config_path) = Config::load_with_path().unwrap_or_else(|e| {
        eprintln!("Warning: failed to load config, using defaults: {e}");
        (Config::default(), None)
    });

    let cli = Cli::parse();
    let global_root = cli.root;
    let global_port = cli.port;

    // Lightweight subcommands — no tracing/AgentManager needed.
    match &cli.cmd {
        Some(Command::Doctor) => {
            return cli::doctor::run(&config, config_path.as_deref()).await;
        }
        Some(Command::Stop) => {
            return cli::daemon::stop_agent().await;
        }
        Some(Command::Status) => {
            return cli::daemon::status(&config, config_path.as_deref()).await;
        }
        Some(Command::Init { global }) => {
            let root = if *global { None } else { global_root.clone() };
            return cli::init::run(*global, root).await;
        }
        Some(Command::Install) | Some(Command::Update) => {
            return cli::self_update::run().await;
        }
        Some(Command::Skills { action }) => {
            let sa = match action {
                SkillsAction::Add {
                    name,
                    repo,
                    git_ref,
                    global,
                    force,
                } => cli::skills_cmd::SkillsAction::Add {
                    name: name.clone(),
                    repo: repo.clone(),
                    git_ref: git_ref.clone(),
                    global: *global,
                    force: *force,
                },
                SkillsAction::Remove { name, global } => cli::skills_cmd::SkillsAction::Remove {
                    name: name.clone(),
                    global: *global,
                },
                SkillsAction::List => cli::skills_cmd::SkillsAction::List,
                SkillsAction::Search { query } => cli::skills_cmd::SkillsAction::Search {
                    query: query.clone(),
                },
            };
            return cli::skills_cmd::run(sa, &config).await;
        }
        _ => {}
    }

    // Daemon mode: spawn self in background and exit
    if cli.daemon {
        return cli::daemon::start_agent(&config, global_port, global_root).await;
    }

    // Full commands — need tracing.
    // Suppress stdout logging in TUI mode — ratatui owns the terminal.
    let will_run_tui = !cli.web && cli.cmd.is_none();
    let log_dir = match logging::setup_tracing_with_settings(logging::LoggingSettings {
        level: config.logging.level.as_deref(),
        directory: config.logging.directory.as_deref(),
        retention_days: config.logging.retention_days,
        suppress_stdout: will_run_tui,
    }) {
        Ok(path) => Some(path),
        Err(err) => {
            eprintln!("Failed to initialize logging: {err}");
            None
        }
    };

    match cli.cmd {
        Some(Command::Eval {
            filter,
            max_iters,
            timeout,
            agent,
            verbose: _,
        }) => {
            let ws_root = workspace::resolve_workspace_root(global_root)?;
            let eval_cfg = eval::EvalConfig {
                ws_root,
                filter,
                max_iters,
                timeout,
                agent_override: agent,
            };
            let summary = eval::run_eval(eval_cfg).await?;
            if summary.failed > 0 {
                std::process::exit(1);
            }
        }

        // Default: bare `ling` → TUI + embedded server (or web-only with --web)
        None => {
            let ws_root = workspace::resolve_workspace_root(global_root)?;
            let port = global_port.unwrap_or(config.server.port);

            // Install/update built-in agent specs to ~/.linggen/agents/
            if let Err(e) = cli::init::install_default_agents() {
                tracing::warn!("Failed to install default agents: {e}");
            }

            let store = Arc::new(project_store::ProjectStore::new());
            let skill_manager = Arc::new(skills::SkillManager::new());
            let config_dir = config_path
                .as_ref()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()));
            let (manager, rx) =
                agent_manager::AgentManager::new(config, config_dir, store, skill_manager.clone());

            let _ = skill_manager.load_all(Some(&ws_root)).await;

            // Auto-install built-in skills if none found
            if skill_manager.list_skills().await.is_empty() {
                let sm = skill_manager.clone();
                let ws = ws_root.clone();
                tokio::spawn(async move {
                    tracing::info!("No skills found, auto-installing built-in skills...");
                    if let Err(e) = auto_install_builtin_skills().await {
                        tracing::warn!("Auto-install skills failed: {e}");
                        return;
                    }
                    let _ = sm.load_all(Some(&ws)).await;
                    tracing::info!("Built-in skills installed");
                });
            }

            let _ = manager.get_or_create_project(ws_root.clone()).await?;

            if cli.web {
                // Web UI only (foreground, no TUI)
                tracing::info!("--- Linggen Agent Startup ---");
                if let Some(path) = config_path.as_ref() {
                    tracing::info!("Config File: {}", path.display());
                } else {
                    tracing::info!("Config File: (default)");
                }
                tracing::info!("Workspace Root: {}", ws_root.display());
                tracing::info!("Server Port: {}", port);
                if let Some(dir) = log_dir.as_ref() {
                    tracing::info!("Log Directory: {}", dir.display());
                }

                let config_snap = manager.get_config_snapshot().await;
                tracing::info!("Max Tool Iterations: {}", config_snap.agent.max_iters);

                {
                    let models_guard = manager.models.read().await;
                    let models = models_guard.list_models();
                    tracing::info!("Configured Models ({}):", models.len());
                    for m in models {
                        tracing::info!(
                            "  - ID: {}, Provider: {}, Model: {}, URL: {}",
                            m.id, m.provider, m.model, m.url
                        );
                    }
                }

                let agents = manager.list_agents(&ws_root).await?;
                tracing::info!("Active Agents ({}):", agents.len());
                for a in agents {
                    tracing::info!("  - Name: {}, Tools: {:?}", a.name, a.tools);
                }
                tracing::info!("------------------------------");

                server::start_server(manager, skill_manager, port, cli.dev, rx).await?;
            } else {
                // TUI + embedded server (default)
                let handle =
                    server::prepare_server(manager, skill_manager, port, cli.dev, rx).await?;
                let result =
                    tui::run_tui(handle.port, ws_root.to_string_lossy().to_string()).await;
                handle.task.abort();
                result?;
            }
        }

        // Already handled above
        Some(Command::Doctor)
        | Some(Command::Stop)
        | Some(Command::Status)
        | Some(Command::Init { .. })
        | Some(Command::Install)
        | Some(Command::Update)
        | Some(Command::Skills { .. }) => unreachable!(),
    }

    Ok(())
}

async fn auto_install_builtin_skills() -> Result<()> {
    let target = crate::paths::global_skills_dir();
    let zip_url =
        skills::marketplace::build_github_zip_url("linggen", "skills", "main");
    let client = skills::marketplace::http_client()?;
    let temp_zip = skills::marketplace::download_to_temp(&client, &zip_url).await?;
    let result = skills::marketplace::extract_all_skills_from_zip(&temp_zip, &target);
    let _ = std::fs::remove_file(&temp_zip);
    result?;
    Ok(())
}

