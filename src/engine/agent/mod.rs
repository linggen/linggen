use crate::config::Config;
use crate::engine::agent::locks::LockManager;
use crate::engine::agent::registry::AgentRegistry;
use crate::engine::agent::record::{AgentSpec, AgentSpecFile};
use crate::engine::{AgentEngine, AgentOutcome, AgentRole, EngineConfig, InterfaceMode, Plan};
use crate::extensions::agents::AgentLoader;
use crate::extensions::skills::SkillLoader;
use crate::provider::models::ModelManager;
use crate::state_fs::{SessionStore, StateFile, StateFs};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::Instant;
use tracing::{info, warn};

pub mod locks;
pub mod registry;
pub mod runs;
pub mod record;

pub use runs::{AgentRunRecord, AgentRunStatus, RunStore};

pub struct ProjectContext {
    pub agents: Mutex<HashMap<String, Arc<Mutex<AgentEngine>>>>,
    pub state_fs: StateFs,
}

pub struct AgentManager {
    config: RwLock<Config>,
    config_dir: Option<PathBuf>,
    pub projects: Mutex<HashMap<String, Arc<ProjectContext>>>,
    pub locks: Mutex<LockManager>,
    pub models: RwLock<Arc<ModelManager>>,
    pub missions: Arc<crate::extensions::missions::MissionLoader>,
    pub skills: Arc<SkillLoader>,
    /// Disk loader for agent specs. Implements `AgentRegistry`; the
    /// engine consults it whenever an agent needs to be resolved by id.
    pub agents: Arc<AgentLoader>,
    /// Global flat session store at `~/.linggen/sessions/`.
    pub global_sessions: SessionStore,
    /// Per-session agent engines. Each session gets its own engine — no lock contention.
    pub session_engines: Mutex<HashMap<String, Arc<Mutex<AgentEngine>>>>,
    working_places: Mutex<HashMap<String, HashMap<String, WorkingPlaceEntry>>>,
    cancelled_runs: Mutex<HashSet<String>>,
    /// Per-tool-block cancellation flags (block_id → AtomicBool).
    tool_cancel_flags: std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>,
    events: mpsc::UnboundedSender<(AgentEvent, Option<String>)>,
    /// Pending plans awaiting user approval, keyed by "{project_root}|{session_id}|{agent_id}".
    pending_plans: Mutex<HashMap<String, Plan>>,
    /// In-memory run store — replaces file-based RunStore. Shared across all operations.
    pub run_store: Arc<crate::engine::agent::RunStore>,
    /// Last activity time per agent, keyed by "{project_root}|{agent_id}".
    last_activity: Mutex<HashMap<String, Instant>>,
    /// Interface mode passed into every EngineConfig.
    interface_mode: InterfaceMode,
    /// Monotonic per-agent seq counter for short, memorable run ids
    /// (e.g. `ling01`, `ling02`). Resets on process restart.
    run_id_counters: std::sync::Mutex<HashMap<String, u64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    TaskUpdate {
        agent_id: String,
        task: String,
    },
    Outcome {
        agent_id: String,
        outcome: AgentOutcome,
    },
    Message {
        from: String,
        to: String,
        content: String,
        /// Unique run_id of the emitting agent — set for subagents so the
        /// UI can route the message into the SubagentPane instead of
        /// leaking it into the parent chat. None for top-level messages.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        /// agent_id of the parent when this comes from a subagent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
    },
    AgentStatus {
        agent_id: String,
        status: String,
        detail: Option<String>,
        parent_id: Option<String>,
        /// Unique run_id of the emitting agent (distinguishes parallel
        /// subagents that share the same `agent_id`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        /// Unique run_id of the parent agent when this is a subagent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    SubagentSpawned {
        parent_id: String,
        subagent_id: String,
        task: String,
        /// Unique run_id of the spawned subagent — the stable key for UI
        /// tracking when multiple subagents share the same `subagent_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    SubagentResult {
        parent_id: String,
        subagent_id: String,
        outcome: AgentOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    ContextUsage {
        agent_id: String,
        stage: String,
        message_count: usize,
        char_count: usize,
        estimated_tokens: usize,
        #[serde(default)]
        token_limit: Option<usize>,
        #[serde(default)]
        actual_prompt_tokens: Option<usize>,
        #[serde(default)]
        actual_completion_tokens: Option<usize>,
        compressed: bool,
        summary_count: usize,
    },
    TextSegment {
        agent_id: String,
        text: String,
        parent_id: Option<String>,
    },
    PlanUpdate {
        agent_id: String,
        plan: Plan,
    },
    ModelFallback {
        agent_id: String,
        preferred_model: String,
        actual_model: String,
        reason: String,
    },
    ToolProgress {
        agent_id: String,
        tool: String,
        line: String,
        stream: String, // "stdout" | "stderr"
    },
    /// A new content block started within the current assistant turn.
    ContentBlockStart {
        agent_id: String,
        block_id: String,
        block_type: String, // "text" | "tool_use" | "tool_result" | "thinking"
        tool: Option<String>,
        args: Option<String>,
        parent_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    /// Update an existing content block (status change, result summary).
    ContentBlockUpdate {
        agent_id: String,
        block_id: String,
        status: Option<String>, // "running" | "done" | "failed"
        summary: Option<String>,
        is_error: Option<bool>,
        parent_id: Option<String>,
        /// Optional extra payload (e.g. diff data for Edit/Write tools).
        extra: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    /// Signal that the assistant turn is complete.
    TurnComplete {
        agent_id: String,
        duration_ms: Option<u64>,
        context_tokens: Option<usize>,
        parent_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    StateUpdated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingPlaceEntry {
    pub repo_path: String,
    pub file_path: String,
    pub agent_id: String,
    pub run_id: Option<String>,
    pub last_modified: u64,
}

impl AgentManager {
    fn normalize_agent_id(agent_id: &str) -> String {
        agent_id.trim().to_lowercase()
    }

    fn canonical_project_root(project_root: &PathBuf) -> PathBuf {
        crate::util::resolve_path(project_root)
    }

    fn model_override_for_agent(config: &Config, agent_id: &str) -> Option<String> {
        config
            .agents
            .iter()
            .find(|a| a.id.eq_ignore_ascii_case(agent_id))
            .and_then(|a| a.model.clone())
    }

    fn normalize_model_choice(raw: Option<String>) -> Option<String> {
        raw.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("inherit") {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }

    /// Resolve the model ID for an agent, using the following priority chain:
    /// 1. Config agent override (if exists in configured models)
    /// 2. Frontmatter model (if exists in configured models)
    /// 3. First model in routing.default_models
    /// 4. Routing policy
    /// 5. First configured model
    fn resolve_model_id(
        config: &Config,
        models: &ModelManager,
        agent_id: &str,
        frontmatter_model: Option<String>,
    ) -> Result<String> {
        let model_ids: std::collections::HashSet<&str> =
            config.models.iter().map(|m| m.id.as_str()).collect();

        // 1. Config agent override
        if let Some(choice) = Self::normalize_model_choice(Self::model_override_for_agent(config, agent_id)) {
            if models.has_model(&choice) {
                return Ok(choice);
            }
            warn!("Agent override model '{}' not found in configured models; falling through", choice);
        }

        // 2. Frontmatter model
        if let Some(choice) = Self::normalize_model_choice(frontmatter_model) {
            if models.has_model(&choice) {
                return Ok(choice);
            }
            warn!("Agent frontmatter model '{}' not found in configured models; falling through", choice);
        }

        // 3. First model in routing.default_models
        for dm in &config.routing.default_models {
            if model_ids.contains(dm.as_str()) {
                return Ok(dm.clone());
            }
        }

        // 4. Routing policy
        if let Some(id) = crate::provider::routing::resolve_model(
            &config.routing,
            None,
            &crate::provider::routing::ComplexitySignal {
                estimated_tokens: None,
                tool_depth: None,
                _skill_model_hint: None,
            },
            &config.models,
        ) {
            return Ok(id);
        }

        // 5. First configured model
        config
            .models
            .first()
            .map(|m| m.id.clone())
            .ok_or_else(|| anyhow::anyhow!("No models configured"))
    }

    fn make_run_id(&self, agent_id: &str) -> String {
        let mut counters = self.run_id_counters.lock().unwrap();
        let seq = counters.entry(agent_id.to_string()).or_insert(0);
        *seq += 1;
        format!("{}{:02}", agent_id, *seq)
    }

    /// Build a fully-initialized `AgentEngine` for the given project + agent_id.
    ///
    /// Centralizes the construction sequence shared by `get_or_create_agent`,
    /// `get_or_create_session_agent`, and `spawn_delegation_engine`:
    ///
    /// 1. Resolve the agent spec and effective model id.
    /// 2. Build the engine with `EngineConfig::from_app_config(...)`.
    /// 3. Apply routing/spec setters and load skill metadata.
    ///
    /// Caching is the caller's responsibility — this helper never inserts into
    /// the project or session engine maps.
    ///
    /// `apply_delegation_depth=false` skips `set_delegation_depth(0, ...)` —
    /// used by `spawn_delegation_engine` whose caller chooses the depth.
    async fn build_engine_for_agent(
        self: &Arc<Self>,
        project_root: &PathBuf,
        normalized_id: &str,
        apply_delegation_depth: bool,
    ) -> Result<AgentEngine> {
        let agent_spec = self
            .agents
            .find(project_root, normalized_id)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Agent '{}' not found in {}/agents",
                    normalized_id,
                    project_root.display()
                )
            })?;

        let config = self.config.read().await.clone();
        let models = self.models.read().await.clone();
        let model_id = Self::resolve_model_id(
            &config,
            &models,
            normalized_id,
            agent_spec.spec.model.clone(),
        )?;

        let mut engine = AgentEngine::new(
            EngineConfig::from_app_config(&config, project_root.clone(), self.interface_mode),
            models,
            model_id,
            AgentRole::Lead,
        )?;

        engine.default_models = config.routing.default_models.clone();
        engine.auto_fallback = config.routing.auto_fallback;
        engine.set_spec(
            normalized_id.to_string(),
            agent_spec.spec,
            agent_spec.system_prompt,
        );
        engine.set_manager_context(self.clone());
        if apply_delegation_depth {
            engine.set_delegation_depth(0, config.agent.max_delegation_depth);
        }
        engine.load_skill_tools(self.skills.as_ref()).await;
        engine.load_available_skills_metadata(self.skills.as_ref()).await;
        engine
            .load_available_agents_metadata(self.agents.as_ref(), project_root)
            .await;
        Ok(engine)
    }

    pub fn new(
        config: Config,
        config_dir: Option<PathBuf>,
        skills: Arc<SkillLoader>,
        agents: Arc<AgentLoader>,
        interface_mode: InterfaceMode,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<(AgentEvent, Option<String>)>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let models = Arc::new(ModelManager::new(config.models.clone()));
        (
            Arc::new(Self {
                config: RwLock::new(config),
                config_dir,
                projects: Mutex::new(HashMap::new()),
                locks: Mutex::new(LockManager::new()),
                models: RwLock::new(models),
                missions: Arc::new(crate::extensions::missions::MissionLoader::new()),
                skills,
                agents,
                working_places: Mutex::new(HashMap::new()),
                cancelled_runs: Mutex::new(HashSet::new()),
                tool_cancel_flags: std::sync::Mutex::new(HashMap::new()),
                events: tx,
                pending_plans: Mutex::new(HashMap::new()),
                run_store: Arc::new(crate::engine::agent::RunStore::new()),
                last_activity: Mutex::new(HashMap::new()),
                interface_mode,
                global_sessions: SessionStore::with_sessions_dir(crate::paths::global_sessions_dir()),
                session_engines: Mutex::new(HashMap::new()),
                run_id_counters: std::sync::Mutex::new(HashMap::new()),
            }),
            rx,
        )
    }

    pub fn register_tool_cancel_flag(&self, block_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.tool_cancel_flags
            .lock()
            .unwrap()
            .insert(block_id.to_string(), Arc::clone(&flag));
        flag
    }

    pub fn trigger_tool_cancel(&self, block_id: &str) -> bool {
        if let Some(flag) = self.tool_cancel_flags.lock().unwrap().get(block_id) {
            flag.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    pub fn clear_tool_cancel_flag(&self, block_id: &str) {
        self.tool_cancel_flags.lock().unwrap().remove(block_id);
    }

    pub async fn get_or_create_project(&self, root: PathBuf) -> Result<Arc<ProjectContext>> {
        let root = root
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("Invalid project path: {}", e))?;
        let mut projects = self.projects.lock().await;
        let key = root.to_string_lossy().to_string();
        if let Some(ctx) = projects.get(&key) {
            return Ok(ctx.clone());
        }

        let state_fs = StateFs::new(root.clone());
        let ctx = Arc::new(ProjectContext {
            agents: Mutex::new(HashMap::new()),
            state_fs,
        });

        projects.insert(key, ctx.clone());
        Ok(ctx)
    }

    pub async fn get_or_create_agent(
        self: &Arc<Self>,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> Result<Arc<Mutex<AgentEngine>>> {
        let project_root = Self::canonical_project_root(project_root);
        let ctx = self.get_or_create_project(project_root.clone()).await?;
        let mut agents = ctx.agents.lock().await;
        let normalized_id = Self::normalize_agent_id(agent_id);

        if let Some(agent) = agents.get(&normalized_id) {
            return Ok(agent.clone());
        }

        let engine = self
            .build_engine_for_agent(&project_root, &normalized_id, true)
            .await?;
        let agent = Arc::new(Mutex::new(engine));
        agents.insert(normalized_id, agent.clone());
        Ok(agent)
    }

    /// Get or create an agent engine for a specific session.
    /// Each session gets its own engine instance — no lock contention between sessions.
    pub async fn get_or_create_session_agent(
        self: &Arc<Self>,
        session_id: &str,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> Result<Arc<Mutex<AgentEngine>>> {
        let normalized_id = Self::normalize_agent_id(agent_id);

        // Check if this session already has an engine
        {
            let engines = self.session_engines.lock().await;
            if let Some(engine) = engines.get(session_id) {
                return Ok(engine.clone());
            }
        }

        // Create a new engine for this session (reuse existing creation logic)
        let project_root = Self::canonical_project_root(project_root);
        let mut engine = self
            .build_engine_for_agent(&project_root, &normalized_id, true)
            .await?;

        // Apply any per-session compact config persisted in session.yaml so a
        // skill's previously-set threshold + focus survive engine restart
        // without needing to be re-pushed on every iframe mount.
        if let Ok(Some(meta)) = self.global_sessions.get_session_meta(session_id) {
            engine.compact_threshold = meta.compact_threshold;
            engine.compact_focus = meta.compact_focus;
        }

        let agent = Arc::new(Mutex::new(engine));
        // Re-check under lock to handle concurrent creation race.
        // If another task created the engine while we were building ours, use theirs.
        let mut engines = self.session_engines.lock().await;
        if let Some(existing) = engines.get(session_id) {
            return Ok(existing.clone());
        }
        engines.insert(session_id.to_string(), agent.clone());
        Ok(agent)
    }

    /// Remove a session's engine when the session is deleted.
    pub async fn remove_session_engine(&self, session_id: &str) {
        self.session_engines.lock().await.remove(session_id);
    }

    /// Apply a runtime path-mode grant to the live engine for a session.
    ///
    /// Returns true if a live engine was found and mutated, false otherwise
    /// (caller still owns the disk write — this only propagates the change
    /// into the running engine so subsequent permission checks see it
    /// without a reload). Mirrors the in-memory mutation the consent flow
    /// performs at `tool_exec.rs:387` so out-of-band PATCHes from skill
    /// iframes are observed by the same agent without a session restart.
    pub async fn apply_runtime_grant(
        &self,
        session_id: &str,
        path: &str,
        mode: crate::engine::permission::PermissionMode,
    ) -> bool {
        let engine_arc = {
            let engines = self.session_engines.lock().await;
            engines.get(session_id).cloned()
        };
        let Some(engine_arc) = engine_arc else { return false };
        let mut engine = engine_arc.lock().await;
        engine.session_permissions.set_path_mode(path, mode);
        true
    }

    /// Create a fresh, uncached `AgentEngine` for a single delegation call.
    ///
    /// Unlike `get_or_create_agent`, the returned engine is **not** inserted into the project's
    /// agent cache.  It is intended for one-shot delegation tasks that run concurrently — each
    /// spawned delegation gets its own engine instance, avoiding the lock contention / deadlock
    /// that would occur if two delegations tried to share a single `Arc<Mutex<AgentEngine>>`.
    pub async fn spawn_delegation_engine(
        self: &Arc<Self>,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> Result<AgentEngine> {
        let project_root = Self::canonical_project_root(project_root);
        let normalized_id = Self::normalize_agent_id(agent_id);
        // Skip set_delegation_depth — the caller (run_delegation) sets the
        // depth + max_depth itself based on the parent engine's depth.
        self.build_engine_for_agent(&project_root, &normalized_id, false)
            .await
    }

    pub async fn is_path_allowed(
        &self,
        project_root: &PathBuf,
        agent_id: &str,
        path: &str,
    ) -> bool {
        // Important: do NOT lock a live agent engine here.
        // `write_file` is called while the engine mutex is already held by run_agent_loop,
        // and re-locking that same engine causes a deadlock.
        // All paths are currently allowed (no per-agent path restrictions).
        let _ = (project_root, agent_id, path);
        true
    }

    pub async fn list_agent_specs(&self, project_root: &PathBuf) -> Result<Vec<AgentSpecFile>> {
        self.agents.list(project_root).await
    }

    pub async fn list_agents(&self, project_root: &PathBuf) -> Result<Vec<AgentSpec>> {
        let mut out = Vec::new();
        for entry in self.list_agent_specs(project_root).await? {
            out.push(entry.spec);
        }
        Ok(out)
    }

    pub async fn agent_exists(
        &self,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> bool {
        matches!(self.agents.find(project_root, agent_id).await, Ok(Some(_)))
    }

    pub async fn invalidate_agent_cache(
        &self,
        project_root: &PathBuf,
        agent_id: Option<&str>,
    ) -> Result<()> {
        let project_root = Self::canonical_project_root(project_root);
        let key = project_root.to_string_lossy().to_string();
        let ctx = {
            let projects = self.projects.lock().await;
            projects.get(&key).cloned()
        };
        let Some(ctx) = ctx else {
            return Ok(());
        };

        let mut agents = ctx.agents.lock().await;
        if let Some(agent_id) = agent_id {
            let normalized = Self::normalize_agent_id(agent_id);
            agents.remove(&normalized);
            info!("Invalidated cached agent '{}' for {}", normalized, key);
        } else {
            agents.clear();
            info!("Invalidated all cached agents for {}", key);
        }
        Ok(())
    }

    pub async fn upsert_working_place(
        &self,
        repo_path: &str,
        agent_id: &str,
        file_path: &str,
        run_id: Option<String>,
    ) {
        let now = crate::util::now_ts_secs();
        let entry = WorkingPlaceEntry {
            repo_path: repo_path.to_string(),
            file_path: file_path.to_string(),
            agent_id: agent_id.to_string(),
            run_id,
            last_modified: now,
        };
        let mut places = self.working_places.lock().await;
        let repo = places.entry(repo_path.to_string()).or_default();
        // Key by run_id when available to avoid collision when the same agent runs
        // in multiple sessions simultaneously.
        let key = entry.run_id.as_deref().unwrap_or(agent_id).to_string();
        repo.insert(key, entry);
    }

    pub async fn clear_working_place_for_agent(&self, repo_path: &str, agent_id: &str) {
        let mut places = self.working_places.lock().await;
        if let Some(repo_map) = places.get_mut(repo_path) {
            // Remove entries matching this agent_id (the key might be agent_id or run_id).
            repo_map.retain(|_, entry| entry.agent_id != agent_id);
            if repo_map.is_empty() {
                places.remove(repo_path);
            }
        }
    }

    pub async fn clear_working_place_for_run(&self, run_id: &str) {
        let mut places = self.working_places.lock().await;
        let repos: Vec<String> = places.keys().cloned().collect();
        for repo in repos {
            if let Some(repo_map) = places.get_mut(&repo) {
                repo_map.retain(|_, entry| entry.run_id.as_deref() != Some(run_id));
                if repo_map.is_empty() {
                    places.remove(&repo);
                }
            }
        }
    }

    pub async fn list_working_places_for_repo(&self, repo_path: &str) -> Vec<WorkingPlaceEntry> {
        let places = self.working_places.lock().await;
        places
            .get(repo_path)
            .map(|repo| repo.values().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn begin_agent_run(
        &self,
        project_root: &PathBuf,
        session_id: Option<&str>,
        agent_id: &str,
        parent_run_id: Option<String>,
        detail: Option<String>,
    ) -> Result<String> {
        use crate::engine::agent::{AgentRunRecord, AgentRunStatus};

        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let run_id = self.make_run_id(agent_id);
        let started_at = crate::util::now_ts_secs();
        let repo_path = project_root.to_string_lossy().to_string();

        let record = AgentRunRecord {
            run_id: run_id.clone(),
            repo_path: repo_path.clone(),
            session_id: session_id.unwrap_or("default").to_string(),
            agent_id: agent_id.to_string(),
            agent_kind: None,
            parent_run_id,
            status: AgentRunStatus::Running,
            detail,
            started_at,
            ended_at: None,
        };
        self.run_store.add_run(&record);
        self.clear_working_place_for_agent(&repo_path, agent_id)
            .await;
        self.cancelled_runs.lock().await.remove(&run_id);
        tracing::info!(
            run_id = %run_id,
            agent_id = %agent_id,
            session_id = %record.session_id,
            parent_run_id = ?record.parent_run_id,
            "run/begin"
        );
        Ok(run_id)
    }

    pub async fn finish_agent_run(
        &self,
        run_id: &str,
        status: crate::engine::agent::AgentRunStatus,
        detail: Option<String>,
    ) -> Result<()> {
        let ended_at = Some(crate::util::now_ts_secs());
        self.run_store.update_run(run_id, status, detail.clone(), ended_at);
        self.clear_working_place_for_run(run_id).await;
        let _ = self.events.send((AgentEvent::StateUpdated, None));
        self.cancelled_runs.lock().await.remove(run_id);
        // Record activity for the agent that just finished
        let run_snapshot = self.run_store.get_run(run_id);
        if let Some(ref run) = run_snapshot {
            self.update_agent_activity(&run.repo_path, &run.agent_id).await;
        }
        self.run_store.remove_run(run_id);
        tracing::info!(
            run_id = %run_id,
            agent_id = run_snapshot.as_ref().map(|r| r.agent_id.as_str()).unwrap_or("?"),
            session_id = run_snapshot.as_ref().map(|r| r.session_id.as_str()).unwrap_or("?"),
            status = ?status,
            detail = ?detail,
            "run/finish"
        );
        Ok(())
    }

    pub async fn list_agent_runs(
        &self,
        _project_root: &PathBuf,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::engine::agent::AgentRunRecord>> {
        Ok(self.run_store.list_runs(session_id))
    }

    pub async fn get_agent_run(
        &self,
        run_id: &str,
        _project_root: Option<&str>,
    ) -> Result<Option<crate::engine::agent::AgentRunRecord>> {
        Ok(self.run_store.get_run(run_id))
    }

    pub async fn is_run_cancelled(&self, run_id: &str) -> bool {
        self.cancelled_runs.lock().await.contains(run_id)
    }

    pub async fn cancel_run_tree(
        &self,
        run_id: &str,
    ) -> Result<Vec<crate::engine::agent::AgentRunRecord>> {
        use crate::engine::agent::AgentRunStatus;

        let mut stack = vec![run_id.to_string()];
        let mut seen = HashSet::new();
        let mut runs = Vec::new();

        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(run) = self.run_store.get_run(&id) else {
                continue;
            };
            for child in self.run_store.list_children(&id) {
                stack.push(child.run_id.clone());
            }
            runs.push(run);
        }

        let to_cancel: Vec<crate::engine::agent::AgentRunRecord> = runs
            .into_iter()
            .filter(|run| run.status == AgentRunStatus::Running)
            .collect();

        let now = crate::util::now_ts_secs();
        {
            let mut cancelled = self.cancelled_runs.lock().await;
            for run in &to_cancel {
                cancelled.insert(run.run_id.clone());
            }
        }
        for run in &to_cancel {
            self.run_store.update_run(
                &run.run_id,
                AgentRunStatus::Cancelled,
                Some("cancelled by user".to_string()),
                Some(now),
            );
            self.clear_working_place_for_run(&run.run_id).await;
        }
        if !to_cancel.is_empty() {
            let _ = self.events.send((AgentEvent::StateUpdated, None));
        }

        Ok(to_cancel)
    }

    pub async fn get_config_snapshot(&self) -> Config {
        self.config.read().await.clone()
    }

    pub async fn apply_config(&self, new_config: Config) -> Result<()> {
        new_config.validate()?;
        // Write to disk first — if this fails, in-memory state remains unchanged.
        new_config.save_runtime(self.config_dir.as_deref())?;
        let new_models = Arc::new(ModelManager::new(new_config.models.clone()));
        *self.models.write().await = new_models;
        *self.config.write().await = new_config.clone();

        // Apply log level change at runtime.
        if let Some(ref level) = new_config.logging.level {
            crate::logging::set_log_level(level);
        }

        // Invalidate all cached agents so they pick up new config on next use.
        // Two caches to clear:
        //   1. Project-level cached agents (ctx.agents) — rarely used on the chat path.
        //   2. Per-session engines (session_engines) — THIS is where live chat engines
        //      live. Without clearing this, existing sessions keep using whatever
        //      model/routing they were built with and config changes have no effect
        //      on running chats.
        let keys: Vec<String> = {
            let projects = self.projects.lock().await;
            projects.keys().cloned().collect()
        };
        for key in keys {
            let root = PathBuf::from(&key);
            let _ = self.invalidate_agent_cache(&root, None).await;
        }
        {
            let mut engines = self.session_engines.lock().await;
            engines.clear();
        }

        let _ = self.events.send((AgentEvent::StateUpdated, None));
        Ok(())
    }

    /// Persist a chat message to the global flat session store.
    pub async fn add_chat_message(
        &self,
        _ws_root: &std::path::Path,
        session_id: &str,
        msg: &crate::state_fs::sessions::ChatMsg,
    ) {
        if let Err(e) = self.global_sessions.add_chat_message(session_id, msg) {
            tracing::warn!("Failed to persist chat message: {}", e);
        }
    }

    /// Update the last plan message in the session store (instead of appending a duplicate).
    pub async fn update_last_plan_message(
        &self,
        session_id: &str,
        msg: &crate::state_fs::sessions::ChatMsg,
    ) -> bool {
        match self.global_sessions.update_last_plan_message(session_id, msg) {
            Ok(updated) => updated,
            Err(e) => {
                tracing::warn!("Failed to update plan message: {}", e);
                false
            }
        }
    }

    pub async fn send_event(&self, event: AgentEvent, session_id: Option<String>) {
        let _ = self.events.send((event, session_id));
    }

    pub async fn set_pending_plan(&self, project_root: &str, agent_id: &str, session_id: Option<&str>, plan: Plan) {
        let sid = session_id.unwrap_or("default");
        let key = format!("{}|{}|{}", project_root, sid, agent_id);
        self.pending_plans.lock().await.insert(key, plan);
    }

    pub async fn take_pending_plan(&self, project_root: &str, agent_id: &str, session_id: Option<&str>) -> Option<Plan> {
        let sid = session_id.unwrap_or("default");
        let key = format!("{}|{}|{}", project_root, sid, agent_id);
        self.pending_plans.lock().await.remove(&key)
    }

    /// Edit the plan_text and summary of a pending plan in-place.
    /// Returns the updated plan, or `None` if the in-memory map has no entry
    /// (e.g. after daemon restart — caller should fall back to session-history
    /// recovery).
    pub async fn edit_pending_plan(
        &self,
        project_root: &str,
        agent_id: &str,
        session_id: Option<&str>,
        text: &str,
    ) -> Option<Plan> {
        let sid = session_id.unwrap_or("default");
        let key = format!("{}|{}|{}", project_root, sid, agent_id);
        let mut plans = self.pending_plans.lock().await;
        let plan = plans.get_mut(&key)?;
        plan.plan_text = text.to_string();
        plan.summary = crate::engine::AgentEngine::extract_plan_summary(text);
        Some(plan.clone())
    }

    /// Record that an agent performed activity (finished run, received message, etc.)
    pub async fn update_agent_activity(&self, project_root: &str, agent_id: &str) {
        let key = format!("{}|{}", project_root, agent_id);
        self.last_activity.lock().await.insert(key, Instant::now());
    }

    pub async fn sync_world_state(&self, project_root: &PathBuf) -> Result<()> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let ctx = self.get_or_create_project(project_root.clone()).await?;
        let tasks = ctx.state_fs.list_tasks()?;
        // All agents are patch-capable; pick the first.
        let patch_agent_id = self
            .list_agent_specs(&project_root)
            .await?
            .into_iter()
            .next()
            .map(|entry| entry.agent_id);

        let active_patch_task = tasks.iter().find(|(meta, _)| match meta {
            StateFile::CoderTask {
                status,
                assigned_to,
                ..
            } => {
                status == "active"
                    && patch_agent_id
                        .as_ref()
                        .map(|agent_id| assigned_to == agent_id)
                        .unwrap_or(false)
            }
            _ => false,
        });

        if let Some((StateFile::CoderTask { id, .. }, body)) = active_patch_task {
            let Some(patch_agent_id) = patch_agent_id else {
                return Ok(());
            };
            let agents = ctx.agents.lock().await;
            if let Some(worker) = agents.get(&patch_agent_id) {
                let mut engine = worker.lock().await;
                let current_task = engine.get_task();
                if current_task.as_deref() != Some(body) {
                    tracing::info!("Syncing active task {} to {} agent", id, patch_agent_id);
                    engine.set_task(body.clone());
                    let _ = self.events.send((AgentEvent::TaskUpdate {
                        agent_id: patch_agent_id,
                        task: body.clone(),
                    }, None));
                }
            }
        }

        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::AgentManager;

    #[test]
    fn normalize_model_choice_treats_inherit_as_none() {
        assert_eq!(AgentManager::normalize_model_choice(None), None);
        assert_eq!(
            AgentManager::normalize_model_choice(Some("inherit".to_string())),
            None
        );
        assert_eq!(
            AgentManager::normalize_model_choice(Some("  InHeRiT ".to_string())),
            None
        );
        assert_eq!(
            AgentManager::normalize_model_choice(Some(" local_ollama ".to_string())),
            Some("local_ollama".to_string())
        );
    }
}
