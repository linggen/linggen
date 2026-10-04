//! `/api/migration/scope/*` — the scope migration as a REVIEW, not a write
//! (`doc/scope-index-spec.md` § Migration).
//!
//! `review` backs up the store and computes proposals — a row, its current
//! scope → a proposed one, a proposed hook, a proposed index flag — into a
//! sidecar (`memory/.scope-migration.json`). Nothing in the store changes
//! until the person accepts a proposal in the console (or the CLI):
//! `accept` applies the accepted rows' fields, `skip` sets proposals aside,
//! `hooks` records model-written hooks on the proposals (still not the rows),
//! and `apply_schema` — a separate accept — runs the gated v1→v2 step that
//! drops `contexts`/`tags`.
//!
//! The generic rules live here: a core row loses its scope; an app row (tagged
//! with an installed skill's name and written by that skill's own session)
//! moves to `~/.linggen/skills/<name>`; a preference or decision without a
//! hook gets a drafted one; a user-stated preference is proposed for the
//! index. Rules about one machine's projects (which rows belong to which
//! repo) are data, read from `memory/scope-migration-rules.json`.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::memory::{scope, AccountScope, Filters, Memory, MemoryPatch, SortOrder, Tier};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/migration/scope/review", post(review))
        .route("/api/migration/scope/accept", post(accept))
        .route("/api/migration/scope/skip", post(skip))
        .route("/api/migration/scope/hooks", post(set_hooks))
        .route("/api/migration/scope/apply_schema", post(apply_schema))
}

// ── Sidecar ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrationState {
    pub created_at: Option<DateTime<Utc>>,
    /// Where `review` copied the store before computing anything.
    pub backup_dir: Option<String>,
    /// When the gated v1→v2 drop ran (accepted by the person).
    pub schema_applied_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub proposals: Vec<Proposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Proposal {
    pub id: String,
    pub content: String,
    pub r#type: String,
    pub from: String,
    pub tier: String,
    /// The row's legacy `contexts`, snapshotted at review time — the drop
    /// erases them.
    #[serde(default)]
    pub contexts: Vec<String>,
    pub current_cwd: Option<String>,
    /// Set when the scope should change; `proposed_cwd` then holds the new
    /// value (`None` = no scope).
    pub cwd_change: bool,
    pub proposed_cwd: Option<String>,
    pub proposed_hook: Option<String>,
    /// `draft` (cut from the content), `model` (written by a model through
    /// `hooks`), or empty when no hook is proposed.
    #[serde(default)]
    pub hook_source: String,
    pub proposed_indexed: Option<bool>,
    pub reasons: Vec<String>,
    /// `pending`, `accepted` or `skipped`.
    pub status: String,
    pub decided_at: Option<DateTime<Utc>>,
}

fn sidecar(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join(".scope-migration.json")
}

pub(crate) async fn load(data_dir: &Path) -> MigrationState {
    let Ok(bytes) = tokio::fs::read(sidecar(data_dir)).await else {
        return MigrationState::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

async fn save(data_dir: &Path, state: &MigrationState) -> Result<(), ApiError> {
    let path = sidecar(data_dir);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| ApiError::internal(e.into()))?;
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| ApiError::internal(e.into()))?;
    tokio::fs::rename(tmp, path)
        .await
        .map_err(|e| ApiError::internal(e.into()))
}

// ── Rules ───────────────────────────────────────────────────────────────────

/// One machine-specific rule: which rows belong to which directory. Every
/// field set must hold; the first matching rule wins.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rule {
    pub name: String,
    /// The proposed scope (`~/…` or absolute).
    pub to: String,
    /// The row's scope is strictly under this dir.
    #[serde(default)]
    pub under: Option<String>,
    /// The row's scope is exactly this dir.
    #[serde(default)]
    pub at: Option<String>,
    /// The row has no scope.
    #[serde(default)]
    pub unscoped: bool,
    /// The row's legacy contexts include one of these.
    #[serde(default)]
    pub contexts_any: Vec<String>,
    /// The row is one of these types.
    #[serde(default)]
    pub types: Vec<String>,
    /// The row's content contains one of these (case-insensitive).
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

fn rules_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join("scope-migration-rules.json")
}

async fn load_rules(data_dir: &Path) -> Result<Rules, ApiError> {
    match tokio::fs::read(rules_path(data_dir)).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::bad_request(format!("scope-migration-rules.json: {e}"))),
        Err(_) => Ok(Rules::default()),
    }
}

/// Everything `propose` reads besides the rows.
pub struct Inputs<'a> {
    pub home: &'a Path,
    /// Row id → legacy contexts.
    pub contexts: &'a HashMap<String, Vec<String>>,
    /// Session id → the skill it was bound to.
    pub session_skill: &'a HashMap<String, String>,
    /// Installed skills (names of `~/.linggen/skills/*`).
    pub installed: &'a [String],
    pub rules: &'a Rules,
}

/// The proposal for one row, or `None` when nothing about it would change.
pub fn propose(row: &Memory, inp: &Inputs) -> Option<Proposal> {
    let contexts = inp.contexts.get(&row.id).cloned().unwrap_or_default();
    let mut reasons = Vec::new();
    let target = scope_target(row, &contexts, inp, &mut reasons);
    let cwd_change = match &target {
        Some(t) => *t != row.cwd,
        None => false,
    };
    let wants_hook = matches!(row.r#type.as_str(), "preference" | "decision") && row.hook.is_none();
    let proposed_hook = wants_hook.then(|| draft_hook(&row.content));
    if wants_hook {
        reasons.push("preference/decision without a hook (drafted)".into());
    }
    let wants_index = row.r#type.as_str() == "preference"
        && row.origin.as_str() == "user"
        && row.tier != Tier::Core
        && !row.indexed;
    if wants_index {
        reasons.push("a preference the user stated: proposed for its scope's index".into());
    }
    if !cwd_change && !wants_hook && !wants_index {
        return None;
    }
    Some(Proposal {
        id: row.id.clone(),
        content: row.content.clone(),
        r#type: row.r#type.as_str().into(),
        from: row.origin.as_str().into(),
        tier: row.tier.as_str().into(),
        contexts,
        current_cwd: row.cwd.clone(),
        cwd_change,
        proposed_cwd: if cwd_change {
            target.flatten()
        } else {
            row.cwd.clone()
        },
        proposed_hook,
        hook_source: if wants_hook {
            "draft".into()
        } else {
            String::new()
        },
        proposed_indexed: wants_index.then_some(true),
        reasons,
        status: "pending".into(),
        decided_at: None,
    })
}

/// Where a row should be filed, if a rule says: `Some(None)` = no scope.
fn scope_target(
    row: &Memory,
    contexts: &[String],
    inp: &Inputs,
    reasons: &mut Vec<String>,
) -> Option<Option<String>> {
    if row.tier == Tier::Core {
        if row.cwd.is_some() {
            reasons.push("core rows carry no scope".into());
            return Some(None);
        }
        return None;
    }
    if let Some(app) = app_of(row, contexts, inp) {
        let dir = inp.home.join(scope::SKILLS_REL).join(&app);
        reasons.push(format!(
            "an {app} app row, written by the {app} skill's own session"
        ));
        return Some(Some(dir.to_string_lossy().to_string()));
    }
    for rule in &inp.rules.rules {
        if let Some(to) = rule_matches(rule, row, contexts, inp.home) {
            reasons.push(format!("rule: {}", rule.name));
            return Some(Some(to));
        }
    }
    None
}

/// The installed skill a row belongs to: tagged with its name AND written by
/// a session bound to it.
fn app_of(row: &Memory, contexts: &[String], inp: &Inputs) -> Option<String> {
    let skill = inp.session_skill.get(row.source_session.as_deref()?)?;
    (contexts.contains(skill) && inp.installed.contains(skill)).then(|| skill.clone())
}

fn rule_matches(rule: &Rule, row: &Memory, contexts: &[String], home: &Path) -> Option<String> {
    let to = scope::expand(&rule.to, home)?.to_string_lossy().to_string();
    if row.cwd.as_deref() == Some(to.as_str()) {
        return None;
    }
    let cwd = row.cwd.as_deref().map(PathBuf::from);
    let path = |p: &Option<String>| p.as_deref().and_then(|p| scope::expand(p, home));
    if let Some(under) = path(&rule.under) {
        if !cwd
            .as_ref()
            .is_some_and(|c| c.starts_with(&under) && *c != under)
        {
            return None;
        }
    }
    if let Some(at) = path(&rule.at) {
        if cwd.as_ref() != Some(&at) {
            return None;
        }
    }
    if rule.unscoped && cwd.is_some() {
        return None;
    }
    if !rule.contexts_any.is_empty() && !contexts.iter().any(|c| rule.contexts_any.contains(c)) {
        return None;
    }
    if !rule.types.is_empty() && !rule.types.iter().any(|t| t == row.r#type.as_str()) {
        return None;
    }
    if !rule.keywords.is_empty() {
        let text = row.content.to_lowercase();
        if !rule
            .keywords
            .iter()
            .any(|k| text.contains(&k.to_lowercase()))
        {
            return None;
        }
    }
    Some(to)
}

/// A mechanical first hook: the content's opening clause, one line, ≤ 80
/// chars. A model rewrites these through `hooks`; the person edits either.
pub fn draft_hook(content: &str) -> String {
    let flat = content.split_whitespace().collect::<Vec<_>>().join(" ");
    // Drop a leading label ("Hanli, 2026-09-15: …", "Workflow rule (…): …").
    let body = match flat.find(": ") {
        Some(i) if i <= 60 => flat[i + 2..].to_string(),
        _ => flat.clone(),
    };
    let cut = [". ", "。", "; ", "；", " — "]
        .iter()
        .filter_map(|sep| body.find(sep))
        .filter(|&i| i >= 12)
        .min()
        .unwrap_or(body.len());
    let head = body[..cut].trim_end_matches(['.', '。']).to_string();
    if head.chars().count() <= 80 {
        return head;
    }
    format!("{}…", head.chars().take(79).collect::<String>().trim_end())
}

// ── Inputs from disk ────────────────────────────────────────────────────────

/// Session id → skill, read from `<data_dir>/sessions/<id>/session.yaml`.
fn session_skills(data_dir: &Path, ids: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for id in ids {
        let path = data_dir.join("sessions").join(id).join("session.yaml");
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let skill = text
            .lines()
            .find_map(|l| l.strip_prefix("skill:"))
            .map(|v| v.trim().trim_matches(['"', '\'']).to_string());
        if let Some(s) = skill.filter(|s| !s.is_empty()) {
            out.insert(id.clone(), s);
        }
    }
    out
}

fn installed_skills(home: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(home.join(scope::SKILLS_REL)) else {
        return Vec::new();
    };
    rd.filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect()
}

/// Copy the store directory and its sidecar version to
/// `memory/backups/scope-migration-<ts>/`. A plain file copy: LanceDB data
/// files are immutable once written, and a manifest copied mid-commit only
/// leaves the copy one version behind.
fn backup(data_dir: &Path) -> std::io::Result<PathBuf> {
    let mem = data_dir.join("memory");
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let dest = mem.join("backups").join(format!("scope-migration-{stamp}"));
    copy_dir(&mem.join("memory.lancedb"), &dest.join("memory.lancedb"))?;
    let version = mem.join("SCHEMA_VERSION");
    if version.exists() {
        std::fs::copy(version, dest.join("SCHEMA_VERSION"))?;
    }
    Ok(dest)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

// ── Handlers ────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct ReviewRequest {
    /// Recompute (with a fresh backup) even if a review exists. Decisions
    /// already made are kept for rows that are still proposed.
    #[serde(default)]
    pub refresh: bool,
}

async fn review(
    State(state): State<SharedState>,
    Json(req): Json<ReviewRequest>,
) -> Result<Response, ApiError> {
    let mut current = load(&state.data_dir).await;
    if current.created_at.is_none() || req.refresh {
        current = compute(&state, current).await?;
        save(&state.data_dir, &current).await?;
    }
    Ok(ok(view(&state, &current)))
}

async fn compute(
    state: &SharedState,
    previous: MigrationState,
) -> Result<MigrationState, ApiError> {
    let data_dir = state.data_dir.clone();
    let backup_dir = tokio::task::spawn_blocking(move || backup(&data_dir))
        .await
        .map_err(|e| ApiError::internal(e.into()))?
        .map_err(|e| ApiError::internal(anyhow::anyhow!("backing up the store: {e}")))?;
    let rows = state
        .store
        .list(
            &Filters {
                account: AccountScope::Owner,
                ..Default::default()
            },
            SortOrder::Newest,
            usize::MAX,
            0,
        )
        .await?;
    // Snapshot the legacy tags once more from the review's own read; an
    // earlier review's snapshot fills rows a drop has since emptied.
    let mut contexts = state.store.legacy_contexts().await?;
    for p in &previous.proposals {
        if !p.contexts.is_empty() {
            contexts
                .entry(p.id.clone())
                .or_insert_with(|| p.contexts.clone());
        }
    }
    let sessions: Vec<String> = rows
        .iter()
        .filter_map(|r| r.source_session.clone())
        .collect();
    let session_skill = session_skills(&state.data_dir, &sessions);
    let home = scope::home();
    let installed = installed_skills(&home);
    let rules = load_rules(&state.data_dir).await?;
    let inputs = Inputs {
        home: &home,
        contexts: &contexts,
        session_skill: &session_skill,
        installed: &installed,
        rules: &rules,
    };
    let decided: HashMap<String, Proposal> = previous
        .proposals
        .into_iter()
        .filter(|p| p.status != "pending")
        .map(|p| (p.id.clone(), p))
        .collect();
    let proposals = rows
        .iter()
        .filter_map(|r| propose(r, &inputs))
        .map(|p| decided.get(&p.id).cloned().unwrap_or(p))
        .collect();
    Ok(MigrationState {
        created_at: Some(Utc::now()),
        backup_dir: Some(backup_dir.to_string_lossy().to_string()),
        schema_applied_at: previous.schema_applied_at,
        proposals,
    })
}

fn view(state: &SharedState, m: &MigrationState) -> serde_json::Value {
    let count = |s: &str| m.proposals.iter().filter(|p| p.status == s).count();
    json!({
        "created_at": m.created_at,
        "backup_dir": m.backup_dir,
        "schema_applied_at": m.schema_applied_at,
        "schema_pending": state.store.is_legacy() || state.episodic.is_legacy(),
        "counts": {
            "total": m.proposals.len(),
            "pending": count("pending"),
            "accepted": count("accepted"),
            "skipped": count("skipped"),
            "scope_moves": m.proposals.iter().filter(|p| p.cwd_change).count(),
            "hooks": m.proposals.iter().filter(|p| p.proposed_hook.is_some()).count(),
            "model_hooks": m.proposals.iter().filter(|p| p.hook_source == "model").count(),
            "index": m.proposals.iter().filter(|p| p.proposed_indexed == Some(true)).count(),
        },
        "proposals": m.proposals,
    })
}

/// One accepted row, optionally with the person's edits to the proposal.
#[derive(Debug, Default, Deserialize)]
pub struct AcceptItem {
    pub id: String,
    /// Edited scope: a path, or `""` for none.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Edited hook: text, or `""` for none.
    #[serde(default)]
    pub hook: Option<String>,
    #[serde(default)]
    pub indexed: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AcceptRequest {
    #[serde(default)]
    pub items: Vec<AcceptItem>,
    /// Accept every pending proposal as proposed.
    #[serde(default)]
    pub all: bool,
}

async fn accept(
    State(state): State<SharedState>,
    Json(req): Json<AcceptRequest>,
) -> Result<Response, ApiError> {
    let mut m = load(&state.data_dir).await;
    if m.created_at.is_none() {
        return Err(ApiError::bad_request("no review yet — run review first"));
    }
    let mut items: Vec<AcceptItem> = req.items;
    if req.all {
        let named: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        items.extend(
            m.proposals
                .iter()
                .filter(|p| p.status == "pending" && !named.contains(&p.id))
                .map(|p| AcceptItem {
                    id: p.id.clone(),
                    ..Default::default()
                }),
        );
    }
    let mut applied = Vec::new();
    let mut missing = Vec::new();
    for item in items {
        let Some(p) = m.proposals.iter_mut().find(|p| p.id == item.id) else {
            missing.push(item.id);
            continue;
        };
        let patch = patch_for(p, &item);
        if state.store.update_quiet(&p.id, &patch).await?.is_none() {
            missing.push(p.id.clone());
            continue;
        }
        p.status = "accepted".into();
        p.decided_at = Some(Utc::now());
        applied.push(p.id.clone());
    }
    save(&state.data_dir, &m).await?;
    Ok(ok(json!({"accepted": applied, "missing": missing})))
}

/// The store patch for an accepted proposal, the person's edits winning.
fn patch_for(p: &Proposal, item: &AcceptItem) -> MemoryPatch {
    let home = scope::home();
    let blank = |s: &str| s.trim().is_empty();
    let cwd = match &item.cwd {
        Some(c) if blank(c) => Some(None),
        Some(c) => Some(scope::expand(c, &home).map(|p| p.to_string_lossy().to_string())),
        None if p.cwd_change => Some(p.proposed_cwd.clone()),
        None => None,
    };
    let hook = match &item.hook {
        Some(h) if blank(h) => Some(None),
        Some(h) => Some(Some(h.trim().to_string())),
        None => p.proposed_hook.clone().map(Some),
    };
    let indexed = item.indexed.or(p.proposed_indexed);
    let core = p.tier == "core";
    MemoryPatch {
        cwd: if core { Some(None) } else { cwd },
        hook,
        indexed: if core { Some(false) } else { indexed },
        ..Default::default()
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct IdsRequest {
    #[serde(default)]
    pub ids: Vec<String>,
}

async fn skip(
    State(state): State<SharedState>,
    Json(req): Json<IdsRequest>,
) -> Result<Response, ApiError> {
    let mut m = load(&state.data_dir).await;
    let mut skipped = Vec::new();
    for p in m.proposals.iter_mut().filter(|p| req.ids.contains(&p.id)) {
        if p.status == "pending" {
            p.status = "skipped".into();
            p.decided_at = Some(Utc::now());
            skipped.push(p.id.clone());
        }
    }
    save(&state.data_dir, &m).await?;
    Ok(ok(json!({"skipped": skipped})))
}

#[derive(Debug, Default, Deserialize)]
pub struct HooksRequest {
    /// Row id → a model-written hook. Lands on the proposal, never the row.
    #[serde(default)]
    pub hooks: HashMap<String, String>,
}

async fn set_hooks(
    State(state): State<SharedState>,
    Json(req): Json<HooksRequest>,
) -> Result<Response, ApiError> {
    let mut m = load(&state.data_dir).await;
    let mut set = 0usize;
    for p in m.proposals.iter_mut().filter(|p| p.status == "pending") {
        let Some(h) = req.hooks.get(&p.id) else {
            continue;
        };
        let line = h.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() || p.proposed_hook.is_none() {
            continue;
        }
        p.proposed_hook = Some(line.chars().take(80).collect());
        p.hook_source = "model".into();
        set += 1;
    }
    save(&state.data_dir, &m).await?;
    Ok(ok(json!({"set": set})))
}

#[derive(Debug, Default, Deserialize)]
pub struct ApplySchemaRequest {
    /// Must be `true`: the drop is the one step a backup cannot be skipped
    /// for, and the person says so.
    #[serde(default)]
    pub confirm: bool,
}

async fn apply_schema(
    State(state): State<SharedState>,
    Json(req): Json<ApplySchemaRequest>,
) -> Result<Response, ApiError> {
    if !req.confirm {
        return Err(ApiError::bad_request(
            "apply_schema drops the contexts and tags columns — pass confirm:true",
        ));
    }
    let mut m = load(&state.data_dir).await;
    let backed_up = m
        .backup_dir
        .as_deref()
        .is_some_and(|d| Path::new(d).exists());
    if !backed_up {
        return Err(ApiError::bad_request(
            "no backup on disk — run review first (it backs the store up)",
        ));
    }
    state.store.drop_legacy_columns().await?;
    state.episodic.drop_legacy_columns().await?;
    crate::memory::schema_version::stamp_layout(&state.data_dir, false)
        .map_err(ApiError::internal)?;
    m.schema_applied_at = Some(Utc::now());
    save(&state.data_dir, &m).await?;
    Ok(ok(json!({
        "schema_version": crate::memory::schema_version::STORE_SCHEMA_VERSION,
        "applied_at": m.schema_applied_at,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryType, Origin};

    fn row(content: &str, t: MemoryType, from: Origin, cwd: Option<&str>) -> Memory {
        let mut m = Memory::new(content, t, from);
        m.tier = Tier::Semantic;
        m.cwd = cwd.map(str::to_string);
        m
    }

    fn rules() -> Rules {
        serde_json::from_value(json!({"rules": [
            {"name": "lingjing", "under": "/h/w/skills/lingjing", "to": "/h/w/skills/lingjing"},
            {"name": "lingjing at skills root", "at": "/h/w/skills", "contexts_any": ["lingjing"], "to": "/h/w/skills/lingjing"},
            {"name": "sanji rules", "contexts_any": ["sanji"], "types": ["preference"], "to": "/h/w/rust/sanji"},
            {"name": "dev rules", "unscoped": true, "types": ["preference"], "keywords": ["commit"], "to": "~/w"}
        ]}))
        .unwrap()
    }

    #[test]
    fn proposals_follow_the_rules() {
        let home = PathBuf::from("/h");
        let r = rules();
        let mut contexts = HashMap::new();
        let mut sessions = HashMap::new();
        let installed = vec!["cfo".to_string()];

        let mut cfo = row(
            "Budget in CAD",
            MemoryType::Fact,
            Origin::User,
            Some("/h/w/linggen"),
        );
        cfo.source_session = Some("s-cfo".into());
        contexts.insert(cfo.id.clone(), vec!["linggen".into(), "cfo".into()]);
        sessions.insert("s-cfo".to_string(), "cfo".to_string());

        let mut dev_cfo = row(
            "CFO review fix shipped",
            MemoryType::Built,
            Origin::Derived,
            None,
        );
        dev_cfo.source_session = Some("s-dev".into());
        contexts.insert(dev_cfo.id.clone(), vec!["cfo".into()]);

        let chapter = row(
            "Write 回 in 评书 voice",
            MemoryType::Decision,
            Origin::Agent,
            Some("/h/w/skills/lingjing/story/x"),
        );
        let root_lj = row(
            "dream maps",
            MemoryType::Fact,
            Origin::Agent,
            Some("/h/w/skills"),
        );
        contexts.insert(root_lj.id.clone(), vec!["lingjing".into()]);
        let commit = row(
            "Always commit straight to main",
            MemoryType::Preference,
            Origin::User,
            None,
        );
        let mut core = row(
            "Atlantic time",
            MemoryType::Fact,
            Origin::User,
            Some("/h/w/linggen"),
        );
        core.tier = Tier::Core;
        let plain = row("likes tea", MemoryType::Fact, Origin::User, None);

        let inp = Inputs {
            home: &home,
            contexts: &contexts,
            session_skill: &sessions,
            installed: &installed,
            rules: &r,
        };
        let p = propose(&cfo, &inp).unwrap();
        assert_eq!(p.proposed_cwd.as_deref(), Some("/h/.linggen/skills/cfo"));
        assert!(
            propose(&dev_cfo, &inp).is_none(),
            "a coding session's tag is not the app's"
        );
        let p = propose(&chapter, &inp).unwrap();
        assert_eq!(p.proposed_cwd.as_deref(), Some("/h/w/skills/lingjing"));
        assert!(p.proposed_hook.is_some(), "a decision gets a drafted hook");
        assert_eq!(
            propose(&root_lj, &inp).unwrap().proposed_cwd.as_deref(),
            Some("/h/w/skills/lingjing")
        );
        let p = propose(&commit, &inp).unwrap();
        assert_eq!(p.proposed_cwd.as_deref(), Some("/h/w"));
        assert_eq!(p.proposed_indexed, Some(true));
        let p = propose(&core, &inp).unwrap();
        assert!(p.cwd_change && p.proposed_cwd.is_none());
        assert!(propose(&plain, &inp).is_none());
    }

    #[test]
    fn a_draft_hook_is_the_opening_clause() {
        assert_eq!(
            draft_hook("Always commit straight to main. Never branch; peers share the checkout."),
            "Always commit straight to main"
        );
        assert_eq!(
            draft_hook("Hanli, 2026-09-15: keep testing on the live 9527. He declined a port."),
            "keep testing on the live 9527"
        );
        let long = draft_hook(&"word ".repeat(40));
        assert!(long.chars().count() <= 80);
    }

    #[test]
    fn the_persons_edits_win_on_accept() {
        let p = Proposal {
            id: "a".into(),
            content: "x".into(),
            r#type: "preference".into(),
            from: "user".into(),
            tier: "semantic".into(),
            contexts: vec![],
            current_cwd: None,
            cwd_change: true,
            proposed_cwd: Some("/h/w".into()),
            proposed_hook: Some("draft".into()),
            hook_source: "draft".into(),
            proposed_indexed: Some(true),
            reasons: vec![],
            status: "pending".into(),
            decided_at: None,
        };
        let as_proposed = patch_for(
            &p,
            &AcceptItem {
                id: "a".into(),
                ..Default::default()
            },
        );
        assert_eq!(as_proposed.cwd, Some(Some("/h/w".into())));
        assert_eq!(as_proposed.hook, Some(Some("draft".into())));
        assert_eq!(as_proposed.indexed, Some(true));
        let edited = patch_for(
            &p,
            &AcceptItem {
                id: "a".into(),
                cwd: Some("".into()),
                hook: Some("mine".into()),
                indexed: Some(false),
            },
        );
        assert_eq!(edited.cwd, Some(None));
        assert_eq!(edited.hook, Some(Some("mine".into())));
        assert_eq!(edited.indexed, Some(false));
    }
}
