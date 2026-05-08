pub mod grader;
pub mod report;
pub mod runner;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Parsed from task.toml
#[derive(Debug, Deserialize)]
pub struct EvalTaskDef {
    pub name: String,
    #[allow(dead_code)]
    pub description: String,
    pub agent_id: String,
    pub prompt: String,
    #[serde(default)]
    pub max_iters: Option<usize>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub grade_script: Option<String>,
    #[serde(default)]
    pub setup_script: Option<String>,
    #[serde(default)]
    pub grade_timeout_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct EvalResult {
    pub task_name: String,
    pub agent_id: String,
    pub passed: bool,
    pub exit_code: i32,
    pub grader_output: String,
    pub iterations_used: usize,
    pub duration: Duration,
    pub outcome_kind: String,
    pub transcript_path: Option<PathBuf>,
}

pub struct EvalSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<EvalResult>,
    pub total_duration: Duration,
}

pub struct EvalConfig {
    pub ws_root: PathBuf,
    pub filter: Option<String>,
    pub max_iters: Option<usize>,
    pub timeout: u64,
    pub agent_override: Option<String>,
}

fn discover_tasks(evals_dir: &PathBuf, filter: Option<&str>) -> Result<Vec<(PathBuf, EvalTaskDef)>> {
    let tasks_dir = evals_dir.join("tasks");
    if !tasks_dir.exists() {
        anyhow::bail!(
            "Eval tasks directory not found: {}",
            tasks_dir.display()
        );
    }

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&tasks_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();

    let mut tasks = Vec::new();
    for dir in entries {
        let dir_name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        if let Some(f) = filter {
            if !dir_name.contains(f) {
                continue;
            }
        }

        let toml_path = dir.join("task.toml");
        if !toml_path.exists() {
            tracing::warn!("Skipping {}: no task.toml found", dir.display());
            continue;
        }

        let content = std::fs::read_to_string(&toml_path)?;
        let task_def: EvalTaskDef = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", toml_path.display(), e))?;

        tasks.push((dir, task_def));
    }

    Ok(tasks)
}

pub async fn run_eval(eval_cfg: EvalConfig) -> Result<EvalSummary> {
    let evals_dir = eval_cfg.ws_root.join("evals");
    let tasks = discover_tasks(&evals_dir, eval_cfg.filter.as_deref())?;

    if tasks.is_empty() {
        eprintln!("No eval tasks found.");
        return Ok(EvalSummary {
            total: 0,
            passed: 0,
            failed: 0,
            results: Vec::new(),
            total_duration: Duration::ZERO,
        });
    }

    eprintln!("Found {} eval task(s)\n", tasks.len());

    let total_start = std::time::Instant::now();
    let mut results = Vec::new();

    for (task_dir, mut task_def) in tasks {
        if let Some(ref agent_override) = eval_cfg.agent_override {
            task_def.agent_id = agent_override.clone();
        }
        if let Some(iters) = eval_cfg.max_iters {
            task_def.max_iters = Some(iters);
        }

        let result = runner::run_single_task(
            &eval_cfg,
            &task_dir,
            &task_def,
        )
        .await;

        match result {
            Ok(r) => results.push(r),
            Err(e) => {
                eprintln!("  ERROR running {}: {}", task_def.name, e);
                results.push(EvalResult {
                    task_name: task_def.name.clone(),
                    agent_id: task_def.agent_id.clone(),
                    passed: false,
                    exit_code: -1,
                    grader_output: format!("Runner error: {}", e),
                    iterations_used: 0,
                    duration: Duration::ZERO,
                    outcome_kind: "error".to_string(),
                    transcript_path: None,
                });
            }
        }
    }

    let total_duration = total_start.elapsed();
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;

    let summary = EvalSummary {
        total: results.len(),
        passed,
        failed,
        results,
        total_duration,
    };

    report::print_summary(&eval_cfg.ws_root, &summary);

    Ok(summary)
}
