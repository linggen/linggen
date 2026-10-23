use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

/// Flat-file session and chat message store.
///
/// Directory layout:
/// ```text
/// <project>/.linggen/sessions/
///   <session_id>/
///     session.yaml      # SessionMeta
///     messages.jsonl     # one ChatMsg per line, append-only
/// ```
pub struct SessionStore {
    sessions_dir: PathBuf,
    append_lock: Mutex<()>,
    /// How many times each session's history was rewritten in place (a
    /// compaction, a clear) through this store. A member's cached thread
    /// from before a rewrite is stale (`chat::thread`).
    rewrites: Mutex<std::collections::HashMap<String, u64>>,
}

fn default_creator() -> String {
    "user".to_string()
}

/// A session has one execution owner. Native sessions may seat several agents;
/// an SDK session keeps its own transcript and permission policy.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum SessionRuntime {
    #[default]
    Linggen,
    ClaudeAgentSdk,
}

impl SessionRuntime {
    pub fn is_linggen(&self) -> bool {
        *self == Self::Linggen
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub enum ClaudePermissionMode {
    #[default]
    Default,
    AcceptEdits,
    Plan,
}

impl ClaudePermissionMode {
    pub fn is_default(&self) -> bool {
        *self == Self::Default
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SessionMeta {
    pub id: String,
    #[serde(default, skip_serializing_if = "SessionRuntime::is_linggen")]
    pub runtime: SessionRuntime,
    #[serde(default, skip_serializing_if = "ClaudePermissionMode::is_default")]
    pub claude_permission_mode: ClaudePermissionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub title: String,
    pub created_at: u64,
    /// Last-activity timestamp (mtime of the session's messages.jsonl),
    /// derived at list time — never persisted to session.yaml.
    #[serde(skip)]
    pub updated_at: u64,
    /// When set, this skill is bound to the session and activated on every message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    /// Who created this session: "user", "skill", "mission", "agent"
    #[serde(default = "default_creator")]
    pub creator: String,
    /// The agents at this session's table, the lead (the one whose tools
    /// an ordinary session uses) first. Each keeps its own model override.
    /// See `doc/shared-session-spec.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<SessionMember>,
    /// What a session recorded before `agents` existed — read once, moved
    /// into `agents` ([`SessionMeta::migrate_members`]), never written.
    #[serde(flatten, skip_serializing)]
    #[cfg_attr(test, ts(skip))]
    pub legacy: LegacyMember,
    /// The folder the agent works in. Never empty: set when the session is
    /// made, moved by a `cd`; a session.yaml written without one gets it when
    /// read ([`SessionMeta::settle_cwd`]) and keeps it from its next write.
    #[serde(default)]
    pub cwd: String,
    /// Detected git root path when agent is inside a project. None in home mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Display name — last segment of git root (e.g. "linggen"). None in home mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    /// Originating mission ID (when creator is "mission").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    /// User ID of the session creator (owner or consumer's linggen.dev user_id).
    /// Used to isolate sessions by user in proxy rooms.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "consumer_user_id"
    )]
    pub user_id: Option<String>,
    /// Per-session override of the auto-compaction trigger fraction of
    /// `context_window_tokens`. None = use engine default (0.95). Set via
    /// POST /api/chat/compact_config; persisted so it survives engine restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_threshold: Option<f32>,
    /// Per-session hint passed to the summarization model on every auto-compact
    /// pass. None = no hint. Set via the same endpoint as `compact_threshold`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_focus: Option<String>,
    /// True once the session title has been "locked" — either auto-derived
    /// from user input crossing the threshold OR explicitly set by the
    /// user via the rename UI. When true, the auto-rename routine stays
    /// silent.
    ///
    /// Defaults to **true** for legacy sessions (missing field in the YAML
    /// on disk → deserialized as true), so the rollout doesn't retro-
    /// rename anything created before this feature shipped. Only fresh
    /// user-creator sessions explicitly set `title_locked: false` at
    /// creation to opt into the auto-rename pool. Skill / mission
    /// sessions also create with `true` because their titles are
    /// canonical (skill name / mission name) and shouldn't be rewritten.
    #[serde(default = "default_true")]
    pub title_locked: bool,
    /// Tools no member of this session is offered or may call — a tool's
    /// name or a whole MCP server (`mcp__memory`). Set when the session is
    /// made (`POST /api/sessions`): a scratch session that must not write
    /// the real memory store withholds the memory server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub withheld_tools: Vec<String>,
    /// The values this session is bound to — the bound skill's declared
    /// session parameters (`session-params`), set once when the session is
    /// made (`POST /api/sessions` `params`) and checked against the
    /// declaration then. Empty: the skill as it runs unbound. See
    /// `doc/skill-spec.md` § Session parameters.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub params: std::collections::BTreeMap<String, String>,
}

fn default_true() -> bool {
    true
}

/// One agent at a session's table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SessionMember {
    pub id: String,
    /// This member's model in this session. None: the agent's own chain
    /// (agent config, routing default; the companion's `pet.model`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl SessionMember {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            model: None,
        }
    }
}

/// The one agent and model a session pinned before it had members.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LegacyMember {
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
}

impl SessionMeta {
    /// Give a session with no folder its own: its project, else where a
    /// session like it starts — a skill's in the skill's folder, any other
    /// in the default workspace. True when it had none.
    pub fn settle_cwd(&mut self) -> bool {
        if !self.cwd.trim().is_empty() {
            return false;
        }
        self.cwd = self
            .project
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| fallback_cwd(self.skill.as_deref()));
        true
    }

    /// Move what the session recorded before it had members into `agents`:
    /// its pinned agent and model become a one-member list; a session that
    /// pinned neither seats the agents that answered there (`answered`, in
    /// the order they first spoke). True when anything moved.
    pub fn migrate_members(&mut self, answered: impl FnOnce() -> Vec<String>) -> bool {
        if !self.agents.is_empty() {
            return false;
        }
        let legacy = std::mem::take(&mut self.legacy);
        if legacy.agent_id.is_none() && legacy.model_id.is_none() {
            self.agents = answered().iter().map(|id| SessionMember::new(id)).collect();
            return !self.agents.is_empty();
        }
        let id = legacy
            .agent_id
            .or_else(|| answered().into_iter().next())
            .unwrap_or_else(|| crate::engine::agent::LEAD_AGENT_ID.to_string());
        self.agents = vec![SessionMember {
            id,
            model: legacy.model_id,
        }];
        true
    }

    /// The member `id`, if seated.
    pub fn member(&self, id: &str) -> Option<&SessionMember> {
        self.agents.iter().find(|m| m.id == id)
    }

    /// Keep `model` as member `id`'s model, seating it if it isn't yet.
    /// True when anything changed.
    pub fn set_member_model(&mut self, id: &str, model: Option<String>) -> bool {
        match self.agents.iter_mut().find(|m| m.id == id) {
            Some(m) if m.model == model => false,
            Some(m) => {
                m.model = model;
                true
            }
            None => {
                self.agents.push(SessionMember {
                    id: id.to_string(),
                    model,
                });
                true
            }
        }
    }
}

/// Where a session with no folder of its own works: a skill's in the
/// skill's folder, any other in the default workspace.
pub fn fallback_cwd(skill: Option<&str>) -> String {
    let dir = match skill.filter(|s| !s.trim().is_empty()) {
        Some(name) => crate::paths::global_skills_dir().join(name),
        None => crate::paths::default_workspace(),
    };
    crate::util::resolve_path(&dir)
        .to_string_lossy()
        .to_string()
}

/// A session.yaml as read: its folder settled ([`SessionMeta::settle_cwd`]).
fn parse_meta(content: &str) -> Result<SessionMeta> {
    let mut meta: SessionMeta = serde_norway::from_str(content)?;
    meta.settle_cwd();
    Ok(meta)
}

/// A session as written: never without a folder.
fn meta_yaml(meta: &SessionMeta) -> Result<String> {
    if !meta.cwd.trim().is_empty() {
        return Ok(serde_norway::to_string(meta)?);
    }
    let mut settled = meta.clone();
    settled.settle_cwd();
    Ok(serde_norway::to_string(&settled)?)
}

/// The agents that answered in `rows`, in the order they first spoke.
pub fn answering_agents(rows: &[ChatMsg]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in rows {
        if r.from_id == r.agent_id && !r.is_observation && !out.contains(&r.agent_id) {
            out.push(r.agent_id.clone());
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMsg {
    pub agent_id: String,
    pub from_id: String,
    pub to_id: String,
    pub content: String,
    pub timestamp: u64,
    pub is_observation: bool,
    /// A person's message: the id the surface that sent it gave its own
    /// bubble, so that surface can match the two (the row may hold other
    /// words than were typed — an `@name` is not kept). None otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// A tool's result row: the call failed (an error, a cancel, a refusal).
    /// The history views read each saved call's status from it. Rows written
    /// before 2026-10-06 have none (`call_failed` reads their text).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
    /// A person's message the session's own agent is continuing from another
    /// device ("iphone"): stored as the user's, shown and read as a hand-off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// The hand-off in the agent's own voice ('ChatMsg::via_note').
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_note: Option<String>,
    /// The pictures a person's message carried, as file names under the
    /// session's `images/` folder ([`SessionStore::keep_images`]). Rows
    /// before 2026-10-19 kept none: their pictures were never stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// The run that wrote the row: every row a run saves (its calls, their
    /// results, its words, its final reply) carries it, and the chat binds
    /// the row to that run's live bubble by it (chat-spec.md § Runs). A
    /// person's message has none — it opens the turn before a run exists.
    /// Rows before 2026-10-22 have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

impl SessionStore {
    /// Create a store rooted at the given sessions directory.
    pub fn with_sessions_dir(sessions_dir: PathBuf) -> Self {
        Self {
            sessions_dir,
            append_lock: Mutex::new(()),
            rewrites: Mutex::new(std::collections::HashMap::new()),
        }
    }

    // ------------------------------------------------------------------
    // Session CRUD
    // ------------------------------------------------------------------

    pub fn add_session(&self, meta: &SessionMeta) -> Result<()> {
        Self::validate_id(&meta.id)?;
        let dir = self.session_dir(&meta.id);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("session.yaml"), meta_yaml(meta)?)?;
        // Create empty messages file
        let msgs_path = dir.join("messages.jsonl");
        if !msgs_path.exists() {
            fs::write(&msgs_path, "")?;
        }
        Ok(())
    }

    pub fn get_session_meta(&self, session_id: &str) -> Result<Option<SessionMeta>> {
        let yaml_path = self.session_dir(session_id).join("session.yaml");
        if !yaml_path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&yaml_path)?;
        let mut meta = parse_meta(&content)?;
        let answered = || {
            self.get_chat_history(session_id)
                .map(|rows| answering_agents(&rows))
                .unwrap_or_default()
        };
        if meta.migrate_members(answered) {
            // Once: the moved fields are never written back.
            let _ = fs::write(&yaml_path, serde_norway::to_string(&meta)?);
        }
        Ok(Some(meta))
    }

    /// The values session `session_id` is bound to, when it is a session of
    /// `skill`; `None` when it isn't (or doesn't exist). A session's params
    /// are its own skill's declarations — never handed to another skill.
    pub fn params_for(
        &self,
        session_id: &str,
        skill: &str,
    ) -> Option<std::collections::BTreeMap<String, String>> {
        match self.get_session_meta(session_id) {
            Ok(Some(meta)) if meta.skill.as_deref() == Some(skill) => Some(meta.params),
            _ => None,
        }
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
        self.list_sessions_paginated(None, None)
    }

    /// List sessions with optional pagination. Results are sorted newest-first.
    /// `limit` caps how many are returned; `offset` skips that many from the top.
    pub fn list_sessions_paginated(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<SessionMeta>> {
        if !self.sessions_dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let yaml_path = entry.path().join("session.yaml");
            if yaml_path.exists() {
                let content = fs::read_to_string(&yaml_path)?;
                match parse_meta(&content) {
                    Ok(mut meta) => {
                        // A listing reads no transcripts: only a pinned
                        // agent or model moves here (see get_session_meta).
                        meta.migrate_members(Vec::new);
                        // Last activity = when the transcript last grew. A
                        // session touched today should surface as today's,
                        // however old its creation date.
                        meta.updated_at = fs::metadata(entry.path().join("messages.jsonl"))
                            .and_then(|m| m.modified())
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(meta.created_at);
                        sessions.push(meta)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Skipping corrupt session.yaml at {}: {}",
                            yaml_path.display(),
                            e
                        );
                    }
                }
            }
        }
        sessions.sort_by_key(|m| std::cmp::Reverse(m.updated_at));
        let off = offset.unwrap_or(0);
        if off > 0 {
            sessions = sessions.into_iter().skip(off).collect();
        }
        if let Some(lim) = limit {
            sessions.truncate(lim);
        }
        Ok(sessions)
    }

    /// Return total session count without reading YAML files (just counts directories).
    pub fn count_sessions(&self) -> usize {
        if !self.sessions_dir.exists() {
            return 0;
        }
        fs::read_dir(&self.sessions_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                    .filter(|e| e.path().join("session.yaml").exists())
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        Self::validate_id(session_id)?;
        let yaml_path = self.session_dir(session_id).join("session.yaml");
        if !yaml_path.exists() {
            bail!("Session not found: {}", session_id);
        }
        let content = fs::read_to_string(&yaml_path)?;
        let mut meta = parse_meta(&content)?;
        meta.title = new_title.to_string();
        // Any explicit rename — user via the API, or the chat auto-rename
        // hook — locks the title so the placeholder-overwrite path stays
        // silent afterwards.
        meta.title_locked = true;
        let yaml = serde_norway::to_string(&meta)?;
        fs::write(yaml_path, yaml)?;
        Ok(())
    }

    pub fn update_session_meta(&self, meta: &SessionMeta) -> Result<()> {
        Self::validate_id(&meta.id)?;
        let yaml_path = self.session_dir(&meta.id).join("session.yaml");
        if !yaml_path.exists() {
            bail!("Session not found: {}", meta.id);
        }
        let yaml = meta_yaml(meta)?;
        fs::write(yaml_path, yaml)?;
        Ok(())
    }

    /// Update the per-session compact config (threshold + focus) in session.yaml.
    /// Read-modify-write so other fields (title, cwd, model_id, ...) aren't disturbed.
    /// Either field can be `None` to clear that override.
    pub fn set_compact_config(
        &self,
        session_id: &str,
        threshold: Option<f32>,
        focus: Option<String>,
    ) -> Result<()> {
        Self::validate_id(session_id)?;
        let yaml_path = self.session_dir(session_id).join("session.yaml");
        if !yaml_path.exists() {
            bail!("Session not found: {}", session_id);
        }
        let content = fs::read_to_string(&yaml_path)?;
        let mut meta = parse_meta(&content)?;
        meta.compact_threshold = threshold;
        meta.compact_focus = focus;
        let yaml = serde_norway::to_string(&meta)?;
        fs::write(yaml_path, yaml)?;
        Ok(())
    }

    pub fn remove_session(&self, session_id: &str) -> Result<()> {
        Self::validate_id(session_id)?;
        let dir = self.session_dir(session_id);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Chat messages
    // ------------------------------------------------------------------

    pub fn add_chat_message(&self, session_id: &str, msg: &ChatMsg) -> Result<()> {
        Self::validate_id(session_id)?;
        let dir = self.session_dir(session_id);
        fs::create_dir_all(&dir)?;
        let msgs_path = dir.join("messages.jsonl");
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        let _guard = self.append_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(msgs_path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    pub fn get_chat_history(&self, session_id: &str) -> Result<Vec<ChatMsg>> {
        Self::validate_id(session_id)?;
        let msgs_path = self.session_dir(session_id).join("messages.jsonl");
        if !msgs_path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(msgs_path)?;
        let reader = BufReader::new(file);
        let mut messages = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<ChatMsg>(trimmed) {
                Ok(msg) => {
                    messages.push(msg);
                }
                Err(e) => {
                    tracing::warn!("Skipping corrupt JSONL line: {}", e);
                }
            }
        }
        Ok(messages)
    }

    /// Replace the last plan message in the session's messages.jsonl.
    /// A plan message has content containing `"type":"plan"`.
    pub fn update_last_plan_message(&self, session_id: &str, updated: &ChatMsg) -> Result<bool> {
        Self::validate_id(session_id)?;
        let msgs_path = self.session_dir(session_id).join("messages.jsonl");
        if !msgs_path.exists() {
            return Ok(false);
        }
        let file = fs::File::open(&msgs_path)?;
        let reader = BufReader::new(file);
        let mut lines: Vec<String> = Vec::new();
        let mut last_plan_idx: Option<usize> = None;
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.contains("\"type\":\"plan\"") && trimmed.contains("\"plan\":{") {
                last_plan_idx = Some(lines.len());
            }
            lines.push(line);
        }
        let Some(idx) = last_plan_idx else {
            return Ok(false);
        };
        lines[idx] = serde_json::to_string(updated)?;
        let tmp_path = msgs_path.with_extension("jsonl.tmp");
        {
            let mut tmp = fs::File::create(&tmp_path)?;
            for l in &lines {
                writeln!(tmp, "{}", l)?;
            }
        }
        fs::rename(&tmp_path, &msgs_path)?;
        Ok(true)
    }

    pub fn clear_chat_history(&self, session_id: &str) -> Result<usize> {
        Self::validate_id(session_id)?;
        let msgs_path = self.session_dir(session_id).join("messages.jsonl");
        if !msgs_path.exists() {
            return Ok(0);
        }
        // Count existing lines first
        let file = fs::File::open(&msgs_path)?;
        let count = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty())
            .count();
        // Truncate
        fs::write(&msgs_path, "")?;
        self.note_rewrite(session_id);
        Ok(count)
    }

    /// Rewrite a session's history from what it holds now, with appends held
    /// off meanwhile: a row written while `edit` runs can't be lost.
    pub fn edit_chat_history(
        &self,
        session_id: &str,
        edit: impl FnOnce(Vec<ChatMsg>) -> Vec<ChatMsg>,
    ) -> Result<()> {
        let _guard = self.append_lock.lock().unwrap_or_else(|e| e.into_inner());
        let rows = self.get_chat_history(session_id)?;
        self.rewrite_chat_history(session_id, &edit(rows))
    }

    /// Replace the entire chat history for a session with the given messages.
    pub fn rewrite_chat_history(&self, session_id: &str, msgs: &[ChatMsg]) -> Result<()> {
        Self::validate_id(session_id)?;
        let dir = self.session_dir(session_id);
        fs::create_dir_all(&dir)?;
        let msgs_path = dir.join("messages.jsonl");
        let mut buf = String::new();
        for msg in msgs {
            let line = serde_json::to_string(msg)?;
            buf.push_str(&line);
            buf.push('\n');
        }
        fs::write(msgs_path, buf)?;
        self.note_rewrite(session_id);
        Ok(())
    }

    fn note_rewrite(&self, session_id: &str) {
        let mut all = self.rewrites.lock().unwrap_or_else(|e| e.into_inner());
        *all.entry(session_id.to_string()).or_default() += 1;
    }

    /// How many times the session's history was rewritten in place through
    /// this store.
    pub fn rewrite_count(&self, session_id: &str) -> u64 {
        let all = self.rewrites.lock().unwrap_or_else(|e| e.into_inner());
        all.get(session_id).copied().unwrap_or(0)
    }

    /// The number of rows the session holds, and the rows from index `skip`
    /// on — the earlier ones are counted, not parsed.
    pub fn rows_since(&self, session_id: &str, skip: usize) -> Result<(usize, Vec<ChatMsg>)> {
        Self::validate_id(session_id)?;
        let msgs_path = self.session_dir(session_id).join("messages.jsonl");
        if !msgs_path.exists() {
            return Ok((0, Vec::new()));
        }
        let reader = BufReader::new(fs::File::open(msgs_path)?);
        let mut total = 0;
        let mut rows = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if total >= skip {
                match serde_json::from_str::<ChatMsg>(trimmed) {
                    Ok(msg) => rows.push(msg),
                    Err(e) => tracing::warn!("Skipping corrupt JSONL line: {}", e),
                }
            }
            total += 1;
        }
        Ok((total, rows))
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(session_id)
    }

    fn validate_id(id: &str) -> Result<()> {
        if id.is_empty() {
            bail!("Session ID must not be empty");
        }
        if id.contains("..") || id.contains('/') || id.contains('\\') {
            bail!("Session ID contains invalid characters: {}", id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (SessionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().join(".linggen/sessions"));
        (store, dir)
    }

    /// Write a session.yaml as an older engine did: no `cwd:` at all.
    fn legacy_session(store: &SessionStore, id: &str, extra: &str) {
        let dir = store.session_dir(id);
        fs::create_dir_all(&dir).unwrap();
        let yaml = format!("id: {id}\ntitle: Old\ncreated_at: 1\n{extra}");
        fs::write(dir.join("session.yaml"), yaml).unwrap();
        fs::write(dir.join("messages.jsonl"), "").unwrap();
    }

    fn yaml_of(store: &SessionStore, id: &str) -> String {
        fs::read_to_string(store.session_dir(id).join("session.yaml")).unwrap()
    }

    /// A session read without a folder gets one — a plain chat the default
    /// workspace, a skill's its skill folder, one with a project that project
    /// — on every read path, and keeps it from its next write.
    #[test]
    fn a_session_without_a_folder_settles_on_one() {
        let (store, _dir) = temp_store();
        legacy_session(&store, "sess-1-plain", "creator: user\n");
        legacy_session(&store, "sess-2-skill", "creator: skill\nskill: dice\n");
        legacy_session(&store, "sess-3-proj", "creator: user\nproject: /tmp\n");

        let plain = store.get_session_meta("sess-1-plain").unwrap().unwrap();
        assert_eq!(plain.cwd, fallback_cwd(None));
        assert!(plain.cwd.ends_with("workspace"), "{}", plain.cwd);
        let skill = store.get_session_meta("sess-2-skill").unwrap().unwrap();
        assert_eq!(skill.cwd, fallback_cwd(Some("dice")));
        assert!(skill.cwd.ends_with("skills/dice"), "{}", skill.cwd);
        let proj = store.get_session_meta("sess-3-proj").unwrap().unwrap();
        assert_eq!(proj.cwd, "/tmp");
        let listed = store.list_sessions().unwrap();
        assert!(
            listed.iter().all(|m| !m.cwd.is_empty()),
            "a listing gave none"
        );

        assert!(
            !yaml_of(&store, "sess-1-plain").contains("cwd:"),
            "a read wrote"
        );
        store.rename_session("sess-1-plain", "Renamed").unwrap();
        assert!(yaml_of(&store, "sess-1-plain").contains(&format!("cwd: {}", plain.cwd)));
        store.update_session_meta(&skill).unwrap();
        assert!(yaml_of(&store, "sess-2-skill").contains("skills/dice"));
    }

    /// The one write path never writes a session without a folder.
    #[test]
    fn a_session_is_never_written_without_a_folder() {
        let (store, _dir) = temp_store();
        let meta = SessionMeta {
            id: "sess-1-blank".into(),
            ..Default::default()
        };
        store.add_session(&meta).unwrap();
        assert!(yaml_of(&store, "sess-1-blank").contains(&format!("cwd: {}", fallback_cwd(None))));
    }

    #[test]
    fn test_session_crud() {
        let (store, _dir) = temp_store();
        let meta = SessionMeta {
            runtime: Default::default(),
            claude_permission_mode: Default::default(),
            parent_session_id: None,
            id: "sess-1000-abcd1234".into(),
            title: "Test Session".into(),
            created_at: 1000,
            updated_at: 0,
            skill: None,
            creator: "user".into(),
            cwd: "/tmp".into(),
            project: None,
            project_name: None,
            mission_id: None,
            agents: Vec::new(),
            legacy: Default::default(),
            user_id: None,
            compact_threshold: None,
            compact_focus: None,
            title_locked: false,
            withheld_tools: Vec::new(),
            params: Default::default(),
        };
        store.add_session(&meta).unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Test Session");
        assert_eq!(sessions[0].id, "sess-1000-abcd1234");

        store
            .rename_session("sess-1000-abcd1234", "Renamed")
            .unwrap();
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions[0].title, "Renamed");
        // rename_session locks the title so the auto-rename hook stays
        // silent on subsequent turns.
        let reloaded = store
            .get_session_meta("sess-1000-abcd1234")
            .unwrap()
            .unwrap();
        assert!(reloaded.title_locked);

        store.remove_session("sess-1000-abcd1234").unwrap();
        assert_eq!(store.list_sessions().unwrap().len(), 0);
    }

    /// A session's bound values survive the store: written with the meta,
    /// read back on restore, every later meta update keeping them. A session
    /// bound to nothing writes no `params` key at all.
    #[test]
    fn a_sessions_bound_params_survive_restore() {
        let (store, dir) = temp_store();
        let mut meta = SessionMeta {
            id: "sess-1-bound".into(),
            title: "t".into(),
            skill: Some("game".into()),
            params: [("save".to_string(), "lx-gu3".to_string())].into(),
            ..Default::default()
        };
        store.add_session(&meta).unwrap();
        let reopened = SessionStore::with_sessions_dir(dir.path().join(".linggen/sessions"));
        let back = reopened.get_session_meta("sess-1-bound").unwrap().unwrap();
        assert_eq!(back.params.get("save").map(String::as_str), Some("lx-gu3"));
        reopened.rename_session("sess-1-bound", "renamed").unwrap();
        let back = reopened.get_session_meta("sess-1-bound").unwrap().unwrap();
        assert_eq!(back.params.len(), 1, "an update keeps them");
        let tools_see = reopened.params_for("sess-1-bound", "game").unwrap();
        assert_eq!(tools_see.get("save").map(String::as_str), Some("lx-gu3"));
        assert!(
            reopened.params_for("sess-1-bound", "other").is_none(),
            "only its own skill's"
        );
        assert!(reopened.params_for("sess-unknown", "game").is_none());

        meta.id = "sess-2-plain".into();
        meta.params.clear();
        store.add_session(&meta).unwrap();
        let yaml = std::fs::read_to_string(store.session_dir("sess-2-plain").join("session.yaml"))
            .unwrap();
        assert!(!yaml.contains("params"), "{yaml}");
    }

    #[test]
    fn test_list_sessions_sorted_desc() {
        let (store, _dir) = temp_store();
        for (id, ts) in [("s1", 100u64), ("s2", 300), ("s3", 200)] {
            store
                .add_session(&SessionMeta {
                    runtime: Default::default(),
                    claude_permission_mode: Default::default(),
                    parent_session_id: None,
                    id: id.into(),
                    title: id.into(),
                    created_at: ts,
                    updated_at: 0,
                    skill: None,
                    creator: "user".into(),
                    cwd: "/tmp".into(),
                    project: None,
                    project_name: None,
                    mission_id: None,
                    agents: Vec::new(),
                    legacy: Default::default(),
                    user_id: None,
                    compact_threshold: None,
                    compact_focus: None,
                    title_locked: false,
                    withheld_tools: Vec::new(),
                    params: Default::default(),
                })
                .unwrap();
        }
        let sessions = store.list_sessions().unwrap();
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["s2", "s3", "s1"]);
    }

    #[test]
    fn test_chat_messages_roundtrip() {
        let (store, _dir) = temp_store();
        let meta = SessionMeta {
            runtime: Default::default(),
            claude_permission_mode: Default::default(),
            parent_session_id: None,
            id: "s1".into(),
            title: "t".into(),
            created_at: 1000,
            updated_at: 0,
            skill: None,
            creator: "user".into(),
            cwd: "/tmp".into(),
            project: None,
            project_name: None,
            mission_id: None,
            agents: Vec::new(),
            legacy: Default::default(),
            user_id: None,
            compact_threshold: None,
            compact_focus: None,
            title_locked: false,
            withheld_tools: Vec::new(),
            params: Default::default(),
        };
        store.add_session(&meta).unwrap();

        let msg1 = ChatMsg {
            run_id: None,
            via: None,
            via_note: None,
            agent_id: "ling".into(),
            from_id: "user".into(),
            to_id: "ling".into(),
            content: "Hello".into(),
            timestamp: 1000,
            is_observation: false,
            client_id: None,
            failed: false,
            images: Vec::new(),
        };
        let msg2 = ChatMsg {
            run_id: None,
            via: None,
            via_note: None,
            agent_id: "ling".into(),
            from_id: "ling".into(),
            to_id: "user".into(),
            content: "Hi there".into(),
            timestamp: 1001,
            is_observation: false,
            client_id: None,
            failed: false,
            images: Vec::new(),
        };
        store.add_chat_message("s1", &msg1).unwrap();
        store.add_chat_message("s1", &msg2).unwrap();

        let history = store.get_chat_history("s1").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "Hello");
        assert_eq!(history[1].content, "Hi there");
    }

    /// A person's row keeps the id its surface sent it with; a row without
    /// one writes no key, and an older row (no key) reads as None.
    #[test]
    fn a_rows_client_id_round_trips_and_is_optional() {
        let row = |client_id: Option<&str>| ChatMsg {
            run_id: None,
            via: None,
            via_note: None,
            agent_id: "yinyue".into(),
            from_id: "user".into(),
            to_id: "yinyue".into(),
            content: "wish me luck".into(),
            timestamp: 1000,
            is_observation: false,
            client_id: client_id.map(str::to_string),
            failed: false,
            images: Vec::new(),
        };
        let kept = serde_json::to_string(&row(Some("c-1-ab"))).unwrap();
        let back: ChatMsg = serde_json::from_str(&kept).unwrap();
        assert_eq!(back.client_id.as_deref(), Some("c-1-ab"));
        let plain = serde_json::to_string(&row(None)).unwrap();
        assert!(!plain.contains("client_id"));
        let old = r#"{"agent_id":"ling","from_id":"user","to_id":"ling","content":"hi","timestamp":1,"is_observation":false}"#;
        assert_eq!(
            serde_json::from_str::<ChatMsg>(old).unwrap().client_id,
            None
        );
    }

    #[test]
    fn test_chat_loads_all_agents_in_session() {
        let (store, _dir) = temp_store();
        store
            .add_session(&SessionMeta {
                runtime: Default::default(),
                claude_permission_mode: Default::default(),
                parent_session_id: None,
                id: "s1".into(),
                title: "t".into(),
                created_at: 1000,
                updated_at: 0,
                skill: None,
                creator: "user".into(),
                cwd: "/tmp".into(),
                project: None,
                project_name: None,
                mission_id: None,
                agents: Vec::new(),
                legacy: Default::default(),
                user_id: None,
                compact_threshold: None,
                compact_focus: None,
                title_locked: false,
                withheld_tools: Vec::new(),
                params: Default::default(),
            })
            .unwrap();

        store
            .add_chat_message(
                "s1",
                &ChatMsg {
                    run_id: None,
                    via: None,
                    via_note: None,
                    agent_id: "ling".into(),
                    from_id: "user".into(),
                    to_id: "ling".into(),
                    content: "for ling".into(),
                    timestamp: 1000,
                    is_observation: false,
                    client_id: None,
                    failed: false,
                    images: Vec::new(),
                },
            )
            .unwrap();
        store
            .add_chat_message(
                "s1",
                &ChatMsg {
                    run_id: None,
                    via: None,
                    via_note: None,
                    agent_id: "coder".into(),
                    from_id: "user".into(),
                    to_id: "coder".into(),
                    content: "for coder".into(),
                    timestamp: 1001,
                    is_observation: false,
                    client_id: None,
                    failed: false,
                    images: Vec::new(),
                },
            )
            .unwrap();

        // Session loads all messages regardless of agent_id
        let all_msgs = store.get_chat_history("s1").unwrap();
        assert_eq!(all_msgs.len(), 2);
        assert_eq!(all_msgs[0].content, "for ling");
        assert_eq!(all_msgs[1].content, "for coder");
    }

    #[test]
    fn test_clear_chat_history() {
        let (store, _dir) = temp_store();
        store
            .add_session(&SessionMeta {
                runtime: Default::default(),
                claude_permission_mode: Default::default(),
                parent_session_id: None,
                id: "s1".into(),
                title: "t".into(),
                created_at: 1000,
                updated_at: 0,
                skill: None,
                creator: "user".into(),
                cwd: "/tmp".into(),
                project: None,
                project_name: None,
                mission_id: None,
                agents: Vec::new(),
                legacy: Default::default(),
                user_id: None,
                compact_threshold: None,
                compact_focus: None,
                title_locked: false,
                withheld_tools: Vec::new(),
                params: Default::default(),
            })
            .unwrap();
        store
            .add_chat_message(
                "s1",
                &ChatMsg {
                    run_id: None,
                    via: None,
                    via_note: None,
                    agent_id: "ling".into(),
                    from_id: "user".into(),
                    to_id: "ling".into(),
                    content: "hello".into(),
                    timestamp: 1000,
                    is_observation: false,
                    client_id: None,
                    failed: false,
                    images: Vec::new(),
                },
            )
            .unwrap();

        let cleared = store.clear_chat_history("s1").unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(store.get_chat_history("s1").unwrap().len(), 0);
    }

    #[test]
    fn test_remove_session_deletes_messages() {
        let (store, _dir) = temp_store();
        store
            .add_session(&SessionMeta {
                runtime: Default::default(),
                claude_permission_mode: Default::default(),
                parent_session_id: None,
                id: "s1".into(),
                title: "t".into(),
                created_at: 1000,
                updated_at: 0,
                skill: None,
                creator: "user".into(),
                cwd: "/tmp".into(),
                project: None,
                project_name: None,
                mission_id: None,
                agents: Vec::new(),
                legacy: Default::default(),
                user_id: None,
                compact_threshold: None,
                compact_focus: None,
                title_locked: false,
                withheld_tools: Vec::new(),
                params: Default::default(),
            })
            .unwrap();
        store
            .add_chat_message(
                "s1",
                &ChatMsg {
                    run_id: None,
                    via: None,
                    via_note: None,
                    agent_id: "ling".into(),
                    from_id: "user".into(),
                    to_id: "ling".into(),
                    content: "hello".into(),
                    timestamp: 1000,
                    is_observation: false,
                    client_id: None,
                    failed: false,
                    images: Vec::new(),
                },
            )
            .unwrap();

        store.remove_session("s1").unwrap();
        // Messages file is gone with the directory
        assert!(store.get_chat_history("s1").unwrap().is_empty());
    }

    #[test]
    fn test_invalid_session_id_rejected() {
        let (store, _dir) = temp_store();
        assert!(store
            .add_session(&SessionMeta {
                runtime: Default::default(),
                claude_permission_mode: Default::default(),
                parent_session_id: None,
                id: "../escape".into(),
                title: "t".into(),
                created_at: 1000,
                updated_at: 0,
                skill: None,
                creator: "user".into(),
                cwd: "/tmp".into(),
                project: None,
                project_name: None,
                mission_id: None,
                agents: Vec::new(),
                legacy: Default::default(),
                user_id: None,
                compact_threshold: None,
                compact_focus: None,
                title_locked: false,
                withheld_tools: Vec::new(),
                params: Default::default(),
            })
            .is_err());
        assert!(store
            .add_session(&SessionMeta {
                runtime: Default::default(),
                claude_permission_mode: Default::default(),
                parent_session_id: None,
                id: "a/b".into(),
                title: "t".into(),
                created_at: 1000,
                updated_at: 0,
                skill: None,
                creator: "user".into(),
                cwd: "/tmp".into(),
                project: None,
                project_name: None,
                mission_id: None,
                agents: Vec::new(),
                legacy: Default::default(),
                user_id: None,
                compact_threshold: None,
                compact_focus: None,
                title_locked: false,
                withheld_tools: Vec::new(),
                params: Default::default(),
            })
            .is_err());
        assert!(store
            .add_session(&SessionMeta {
                runtime: Default::default(),
                claude_permission_mode: Default::default(),
                parent_session_id: None,
                id: "".into(),
                title: "t".into(),
                created_at: 1000,
                updated_at: 0,
                skill: None,
                creator: "user".into(),
                cwd: "/tmp".into(),
                project: None,
                project_name: None,
                mission_id: None,
                agents: Vec::new(),
                legacy: Default::default(),
                user_id: None,
                compact_threshold: None,
                compact_focus: None,
                title_locked: false,
                withheld_tools: Vec::new(),
                params: Default::default(),
            })
            .is_err());
    }

    #[test]
    fn test_add_message_creates_session_dir() {
        let (store, _dir) = temp_store();
        // add_chat_message should work even if add_session wasn't called
        store
            .add_chat_message(
                "auto-created",
                &ChatMsg {
                    run_id: None,
                    via: None,
                    via_note: None,
                    agent_id: "ling".into(),
                    from_id: "user".into(),
                    to_id: "ling".into(),
                    content: "hello".into(),
                    timestamp: 1000,
                    is_observation: false,
                    client_id: None,
                    failed: false,
                    images: Vec::new(),
                },
            )
            .unwrap();
        let history = store.get_chat_history("auto-created").unwrap();
        assert_eq!(history.len(), 1);
    }
}
