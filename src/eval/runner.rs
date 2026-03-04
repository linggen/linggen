use crate::agent_manager::AgentManager;
use crate::config::Config;
use crate::eval::grader::run_grader;
use crate::eval::report::save_transcript;
use crate::eval::{EvalConfig, EvalResult, EvalTaskDef};
use crate::skills::SkillManager;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

pub async fn run_single_task(
    eval_cfg: &EvalConfig,
    task_dir: &PathBuf,
    task_def: &EvalTaskDef,
    store: Arc<crate::project_store::ProjectStore>,
) -> Result<EvalResult> {
    let start = std::time::Instant::now();
    eprint!("  {} ...", task_def.name);

    // 1. Create tmpdir (sanitize task name to prevent path traversal)
    let ts = crate::util::now_ts_secs();
    let safe_name: String = task_def
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let tmpdir = std::env::temp_dir().join(format!("linggen-eval-{}-{}", safe_name, ts));
    std::fs::create_dir_all(&tmpdir)?;

    // 2. Setup workspace — either via setup_script or the default copy+git-init flow
    if let Some(ref setup_script) = task_def.setup_script {
        // Run custom setup script (e.g. clone a real repo, checkout a commit)
        let script_path = task_dir.join(setup_script);
        if !script_path.exists() {
            anyhow::bail!(
                "setup_script not found: {}",
                script_path.display()
            );
        }
        let setup_output = std::process::Command::new("bash")
            .arg(&script_path)
            .current_dir(&tmpdir)
            .env("EVAL_WORKSPACE", &tmpdir)
            .env("EVAL_TASK_NAME", &task_def.name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()?;
        if !setup_output.status.success() {
            let stderr = String::from_utf8_lossy(&setup_output.stderr);
            anyhow::bail!(
                "setup_script failed (exit {}): {}",
                setup_output.status.code().unwrap_or(-1),
                stderr
            );
        }
    } else {
        // Default flow: copy workspace/ contents, git init
        let workspace_src = task_dir.join("workspace");
        if workspace_src.exists() {
            copy_dir_recursive(&workspace_src, &tmpdir)?;
        }

        let git_init = std::process::Command::new("git")
            .args(["init"])
            .current_dir(&tmpdir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        if let Ok(status) = git_init {
            if status.success() {
                let _ = std::process::Command::new("git")
                    .args(["add", "-A"])
                    .current_dir(&tmpdir)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                let _ = std::process::Command::new("git")
                    .args(["commit", "-m", "eval baseline", "--allow-empty"])
                    .current_dir(&tmpdir)
                    .env("GIT_AUTHOR_NAME", "eval")
                    .env("GIT_AUTHOR_EMAIL", "eval@linggen")
                    .env("GIT_COMMITTER_NAME", "eval")
                    .env("GIT_COMMITTER_EMAIL", "eval@linggen")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }
    }

    // 3. Copy agents/ from ws_root into tmpdir so AgentManager finds specs
    let agents_src = eval_cfg.ws_root.join("agents");
    if agents_src.exists() {
        let agents_dst = tmpdir.join("agents");
        copy_dir_recursive(&agents_src, &agents_dst)?;
    }

    // 5. Create SkillManager, AgentManager (store is shared across tasks)
    let (config, _config_path) =
        Config::load_with_path().unwrap_or_else(|_| (Config::default(), None));
    let skill_manager = Arc::new(SkillManager::new());
    let (manager, _rx) = AgentManager::new(config.clone(), None, store.clone(), skill_manager.clone());

    // 6. Get or create agent
    let agent = manager
        .get_or_create_agent(&tmpdir, &task_def.agent_id)
        .await?;

    // 7. Configure engine
    let max_iters = task_def.max_iters.unwrap_or(config.agent.max_iters);
    let timeout_secs = task_def.timeout_secs.unwrap_or(eval_cfg.timeout);

    {
        let mut engine = agent.lock().await;
        engine.set_task(task_def.prompt.clone());
        engine.cfg.max_iters = max_iters;
    }

    // 8. Run agent loop with timeout
    let outcome = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        async {
            let mut engine = agent.lock().await;
            engine.run_agent_loop(Some("eval")).await
        },
    )
    .await;

    let (outcome_kind, iterations_used) = {
        let engine = agent.lock().await;
        let iters = engine.context_records.len();
        match &outcome {
            Ok(Ok(crate::engine::AgentOutcome::Patch(_))) => ("patch".to_string(), iters),
            Ok(Ok(crate::engine::AgentOutcome::Task(_))) => ("task".to_string(), iters),
            Ok(Ok(crate::engine::AgentOutcome::Plan(_))) => ("plan".to_string(), iters),
            Ok(Ok(crate::engine::AgentOutcome::PlanApproved(_))) => ("plan_approved".to_string(), iters),
            Ok(Ok(crate::engine::AgentOutcome::PlanModeRequested { .. })) => ("plan_mode_requested".to_string(), iters),
            Ok(Ok(crate::engine::AgentOutcome::None)) => ("none".to_string(), iters),
            Ok(Err(_)) => ("error".to_string(), iters),
            Err(_) => ("timeout".to_string(), iters),
        }
    };

    // 9. Run grader
    let grade_script_name = task_def
        .grade_script
        .as_deref()
        .unwrap_or("grade.sh");
    let grade_script = task_dir.join(grade_script_name);

    let grade_result = run_grader(&grade_script, &tmpdir, &task_def.name, task_def.grade_timeout_secs).await?;

    let duration = start.elapsed();

    // 10. On failure, save transcript
    let transcript_path = if !grade_result.passed {
        let engine = agent.lock().await;
        save_transcript(
            &eval_cfg.ws_root,
            &task_def.name,
            &EvalResult {
                task_name: task_def.name.clone(),
                agent_id: task_def.agent_id.clone(),
                passed: false,
                exit_code: grade_result.exit_code,
                grader_output: grade_result.output.clone(),
                iterations_used,
                duration,
                outcome_kind: outcome_kind.clone(),
                transcript_path: None,
            },
            &engine.context_records,
            &engine.chat_history,
        )
    } else {
        None
    };

    let status_str = if grade_result.passed { "PASS" } else { "FAIL" };
    eprintln!(" {} ({:.1}s)", status_str, duration.as_secs_f64());

    // 11. Cleanup tmpdir and remove ephemeral project from store
    let _ = store.remove_project(&tmpdir.to_string_lossy());
    let _ = std::fs::remove_dir_all(&tmpdir);

    Ok(EvalResult {
        task_name: task_def.name.clone(),
        agent_id: task_def.agent_id.clone(),
        passed: grade_result.passed,
        exit_code: grade_result.exit_code,
        grader_output: grade_result.output,
        iterations_used,
        duration,
        outcome_kind,
        transcript_path,
    })
}
