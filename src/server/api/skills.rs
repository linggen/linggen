//! Skill listing + skill-file CRUD endpoints.

use crate::extensions::skills::Skill;
use crate::server::{ServerEvent, ServerState};
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::{canonical_project_root, ProjectQuery};

/// `?brief=1` drops each skill's `content` — the whole SKILL.md body.
///
/// A caller that only wants to know WHICH skills exist pays 192 KB for 3 KB of
/// names otherwise, because the bodies are 72% of the response. That is merely
/// wasteful on loopback and genuinely breaks a phone, where the same JSON has
/// to be gzipped, chunked and reassembled across a WebRTC data channel.
#[derive(Deserialize)]
pub(crate) struct SkillListQuery {
    /// Taken as a string, not a bool: serde only accepts a literal `true` for
    /// the latter, so `?brief=1` — which is what a caller actually writes —
    /// comes back as a 400 instead of a listing.
    #[serde(default)]
    brief: Option<String>,
}

impl SkillListQuery {
    fn is_brief(&self) -> bool {
        matches!(self.brief.as_deref(), Some("1" | "true" | "yes" | ""))
    }
}

pub(crate) async fn list_skills(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<SkillListQuery>,
) -> impl IntoResponse {
    let mut skills: Vec<Skill> = state.skills.list_skills().await;
    if q.is_brief() {
        for skill in &mut skills {
            skill.content = String::new();
        }
    }
    Json(skills).into_response()
}

/// What changed in the skills on disk — the one difference between the
/// paths that end in [`skills_changed`].
pub(crate) enum SkillsChange<'a> {
    /// Added, rewritten, moved or reloaded: their missions are read again.
    OnDisk,
    /// This skill was removed: what the user kept for its missions goes too.
    Removed(&'a str),
}

/// The one follow-up after the skills on disk changed, whoever changed them:
/// read them again, have every engine read its skill list again at its next
/// turn (`mark_skills_changed` — no session's engine is dropped, so none
/// loses what it holds and a running turn stays the session's), drop the
/// project's cached agents, pick up the missions the skills ship, and tell
/// the pages. Every path that changes skills ends here; the test
/// `skills_are_reloaded_only_by_the_one_follow_up` fails on a second copy,
/// and `a_change_of_skills_keeps_every_live_engine` on a follow-up that
/// drops engines again.
pub(crate) async fn skills_changed(
    state: &ServerState,
    project_root: Option<&std::path::Path>,
    change: SkillsChange<'_>,
) -> anyhow::Result<()> {
    let loaded = state.skills.load_all(project_root).await;
    if let Err(err) = &loaded {
        tracing::warn!("Failed to reload skills: {err}");
    }
    if let Some(root) = project_root {
        let _ = state.manager.invalidate_agent_cache(&root.to_path_buf(), None).await;
    }
    state.manager.mark_skills_changed();
    match change {
        SkillsChange::OnDisk => state.missions.reload(),
        SkillsChange::Removed(name) => state.missions.forget_skill(name),
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
    loaded
}

/// Reload skills from disk (`skills_changed`).
pub(crate) async fn reload_skills(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let project_root = body.get("project_root").and_then(|v| v.as_str());
    let root_path = project_root.map(std::path::Path::new);
    if let Err(err) = skills_changed(&state, root_path, SkillsChange::OnDisk).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

// ---------------------------------------------------------------------------
// Skill-file CRUD (mirrors agent-file endpoints)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct SkillFileQuery {
    project_root: String,
    path: String,
}

#[derive(Deserialize)]
pub(crate) struct UpsertSkillFileRequest {
    project_root: String,
    path: String,
    content: String,
}

#[derive(Deserialize)]
pub(crate) struct DeleteSkillFileRequest {
    project_root: String,
    path: String,
}

#[derive(Serialize)]
struct SkillFileListItem {
    name: String,
    path: String,
    source: String,
}

#[derive(Serialize)]
struct SkillFileResponse {
    path: String,
    content: String,
    valid: bool,
    error: Option<String>,
}

const PROJECT_SKILL_PREFIXES: &[&str] = &[".linggen/skills/", ".claude/skills/", ".codex/skills/"];

fn normalize_skill_md_path(path: &str) -> Result<String, String> {
    let raw = path.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err("path is required".to_string());
    }
    if raw.starts_with('/') || raw.contains("..") {
        return Err("path must be a relative markdown path under a skills/ directory".to_string());
    }
    // Accept paths already under any of the 3 project skill dirs
    let rel = if PROJECT_SKILL_PREFIXES.iter().any(|p| raw.starts_with(p)) {
        raw
    } else {
        format!(".linggen/skills/{}", raw)
    };
    if !rel.to_ascii_lowercase().ends_with(".md") {
        return Err("skill files must end with .md".to_string());
    }
    if !rel
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '_' || c == '.')
    {
        return Err("path contains unsupported characters".to_string());
    }
    let suffix = PROJECT_SKILL_PREFIXES
        .iter()
        .find_map(|p| rel.strip_prefix(p))
        .unwrap_or("");
    if suffix.is_empty() || suffix.split('/').any(|seg| seg.is_empty()) {
        return Err("invalid skill markdown path".to_string());
    }
    Ok(rel)
}

pub(crate) async fn list_skill_files_api(Query(query): Query<ProjectQuery>) -> impl IntoResponse {
    let root = canonical_project_root(&query.project_root);
    let mut items: Vec<SkillFileListItem> = Vec::new();

    for prefix in PROJECT_SKILL_PREFIXES {
        let skills_dir = root.join(prefix);
        if !skills_dir.exists() {
            continue;
        }
        let entries = match std::fs::read_dir(&skills_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md") {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(path.as_path())
                    .to_string_lossy()
                    .to_string();
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                items.push(SkillFileListItem {
                    name,
                    path: rel,
                    source: "project".to_string(),
                });
            }
        }
    }

    items.sort_by(|a, b| a.name.cmp(&b.name));
    Json(items).into_response()
}

pub(crate) async fn get_skill_file_api(Query(query): Query<SkillFileQuery>) -> impl IntoResponse {
    let root = canonical_project_root(&query.project_root);
    let rel = match normalize_skill_md_path(&query.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let full_path = root.join(&rel);
    let content = match std::fs::read_to_string(&full_path) {
        Ok(content) => content,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let valid = content.starts_with("---")
        && content.splitn(3, "---").count() >= 3
        && serde_norway::from_str::<serde_norway::Value>(content.split("---").nth(1).unwrap_or(""))
            .is_ok();
    Json(SkillFileResponse {
        path: rel,
        content,
        valid,
        error: if valid {
            None
        } else {
            Some("Invalid YAML frontmatter".to_string())
        },
    })
    .into_response()
}

pub(crate) async fn upsert_skill_file_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<UpsertSkillFileRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    let rel = match normalize_skill_md_path(&req.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    if !req.content.starts_with("---") {
        return (
            StatusCode::BAD_REQUEST,
            "Skill must start with YAML frontmatter",
        )
            .into_response();
    }
    let full_path = root.join(&rel);
    if let Some(parent) = full_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
        }
    }
    if let Err(err) = std::fs::write(&full_path, &req.content) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    let _ = skills_changed(&state, Some(&root), SkillsChange::OnDisk).await;
    Json(serde_json::json!({ "path": rel })).into_response()
}

pub(crate) async fn delete_skill_file_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<DeleteSkillFileRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    let rel = match normalize_skill_md_path(&req.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let full_path = root.join(&rel);
    if !full_path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(err) = std::fs::remove_file(&full_path) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    let _ = skills_changed(&state, Some(&root), SkillsChange::OnDisk).await;
    StatusCode::OK.into_response()
}

#[cfg(test)]
mod tests {
    /// Every `.rs` file under `dir`, with its path.
    fn sources(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("src dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("source");
                out.push((path.display().to_string(), text));
            }
        }
    }

    /// The lock on `skills_changed`: outside it, the daemon's skills are read
    /// only at boot (`main.rs`, before any session runs). A handler that
    /// reloads them itself would leave live engines on the old list.
    #[test]
    fn skills_are_reloaded_only_by_the_one_follow_up() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&src, &mut files);
        let needle = [".load_all", "("].concat();
        let mut sites = Vec::new();
        for (path, text) in &files {
            let n = text.matches(needle.as_str()).count();
            if n > 0 {
                sites.push((path.rsplit("/src/").next().unwrap_or(path).to_string(), n));
            }
        }
        sites.sort();
        assert_eq!(
            sites,
            [("main.rs".to_string(), 2), ("server/api/skills.rs".to_string(), 1)],
            "reload skills through `skills_changed`"
        );
    }

    /// A change of skills tells live engines to read their list again
    /// (`AgentManager::mark_skills_changed`); it never drops them. Dropping
    /// them took from every session what its engine held — what the person
    /// declined, its prompt — and left a running turn on an engine no
    /// lookup finds (engine side: `a_change_of_skills_keeps_every_live_engine`).
    #[test]
    fn a_change_of_skills_keeps_every_live_engine() {
        let text = include_str!("skills.rs");
        let start = text.find("pub(crate) async fn skills_changed(").expect("the follow-up");
        let body = &text[start..];
        let body = &body[..body.find("\n}\n").expect("its end")];
        assert!(body.contains("mark_skills_changed()"), "the follow-up must mark the change");
        assert!(
            !body.contains(["session_", "engines"].concat().as_str()),
            "the follow-up must not drop the session engines"
        );
    }
}
