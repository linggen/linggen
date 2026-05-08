use crate::eval::{EvalResult, EvalSummary};
use std::path::Path;

pub fn print_summary(ws_root: &Path, summary: &EvalSummary) {
    eprintln!();
    eprintln!("=== Eval Results ===");
    eprintln!();
    eprintln!(
        "  {:<24} {:<10} {:<8} {:<7} {}",
        "Task", "Agent", "Result", "Iters", "Duration"
    );
    eprintln!(
        "  {:<24} {:<10} {:<8} {:<7} {}",
        "---", "---", "---", "---", "---"
    );

    for r in &summary.results {
        let status = if r.passed { "PASS" } else { "FAIL" };
        let duration = format!("{:.1}s", r.duration.as_secs_f64());
        eprintln!(
            "  {:<24} {:<10} {:<8} {:<7} {}",
            r.task_name, r.agent_id, status, r.iterations_used, duration
        );
    }

    eprintln!();
    eprintln!(
        "  Total: {}   Passed: {}   Failed: {}   Duration: {:.1}s",
        summary.total,
        summary.passed,
        summary.failed,
        summary.total_duration.as_secs_f64()
    );

    let failures: Vec<&EvalResult> = summary.results.iter().filter(|r| !r.passed).collect();
    if !failures.is_empty() {
        eprintln!();
        eprintln!("Failed:");
        for f in failures {
            let output_preview = if f.grader_output.chars().count() > 200 {
                let truncated: String = f.grader_output.chars().take(200).collect();
                format!("{}...", truncated)
            } else {
                f.grader_output.clone()
            };
            eprintln!("  {}: {}", f.task_name, output_preview);
            if let Some(ref path) = f.transcript_path {
                if let Ok(rel) = path.strip_prefix(ws_root) {
                    eprintln!("    Transcript: {}", rel.display());
                } else {
                    eprintln!("    Transcript: {}", path.display());
                }
            }
        }
    }

    eprintln!();
}

pub fn save_transcript(
    ws_root: &Path,
    task_name: &str,
    result: &EvalResult,
    context_records: &[crate::engine::ContextRecord],
    chat_history: &[crate::message::ChatMessage],
) -> Option<std::path::PathBuf> {
    let results_dir = ws_root
        .join("evals")
        .join("results")
        .join(task_name);

    if std::fs::create_dir_all(&results_dir).is_err() {
        return None;
    }

    let timestamp = chrono_timestamp();
    let filename = format!("{}.json", timestamp);
    let path = results_dir.join(&filename);

    let transcript = serde_json::json!({
        "task_name": result.task_name,
        "agent_id": result.agent_id,
        "passed": result.passed,
        "exit_code": result.exit_code,
        "grader_output": result.grader_output,
        "iterations_used": result.iterations_used,
        "duration_secs": result.duration.as_secs_f64(),
        "outcome_kind": result.outcome_kind,
        "context_records": context_records,
        "chat_history": chat_history,
    });

    match serde_json::to_string_pretty(&transcript) {
        Ok(json) => {
            if std::fs::write(&path, json).is_ok() {
                Some(path)
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

fn chrono_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // Format as ISO-ish timestamp without a chrono dependency
    let secs = now.as_secs();
    // Simple epoch-based filename
    format!("{}", secs)
}
