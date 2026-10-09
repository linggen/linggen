use crate::config::Config;
use crate::engine::agent::locks::LockManager;
use crate::engine::agent::record::{AgentSpec, AgentSpecFile};
use crate::engine::agent::registry::AgentRegistry;
use crate::engine::mission::MissionRegistry;
use crate::engine::skill::SkillRegistry;
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode, Plan};
use crate::provider::models::ModelManager;
use crate::state_fs::{SessionStore, StateFile, StateFs};
use crate::util::LockExt;

/// The resident companion agent — a built-in agent like `ling`, not a skill.
/// Everything the engine does specially for her (heralds, the presenter
/// registry, relaying another agent's prompt) keys on this one id.
pub const COMPANION_AGENT_ID: &str = "yinyue";
/// The agent an ordinary session seats, and the one that answers on the Mac
/// when a message names nobody (`doc/shared-session-spec.md` § Who answers).
pub const LEAD_AGENT_ID: &str = "ling";
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::Instant;
use tracing::{info, warn};

pub mod event;
pub mod locks;
pub mod met;
pub mod presence;
pub mod record;
pub mod registry;
pub mod runs;
pub mod runs_api;

pub use event::AgentEvent;
pub use presence::{Beat, Presence, PresenceBoard};
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
    /// Mission lookup. The daemon's loader implements it; editing and
    /// scheduling go through the loader the server holds, not through here.
    pub missions: Arc<dyn MissionRegistry>,
    pub skills: Arc<dyn SkillRegistry>,
    /// Agent specs by id. The daemon's disk loader implements it; the
    /// engine consults it whenever an agent needs to be resolved by id.
    pub agents: Arc<dyn AgentRegistry>,
    /// Global flat session store at `~/.linggen/sessions/`.
    pub global_sessions: SessionStore,
    /// Live engines, one per (session, member agent).
    pub session_engines: Mutex<HashMap<(String, String), Arc<Mutex<AgentEngine>>>>,
    /// Each session's turn lock: one agent speaks at a time in a session,
    /// whoever it is. Held for the whole turn, taken before the engine's.
    session_turns: std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>,
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
    /// Live user-presence signal, fed by a throttled client beat
    /// (`POST /api/presence`) — only recency, focus, and a typing flag, never
    /// keystroke content — one reading per surface. Read by the `sense` tool.
    presence: std::sync::Mutex<PresenceBoard>,
    /// Sessions whose current turn was woken by an `agent_chat` — that turn may
    /// not emit another `agent_chat` (the one-hop loop-break; a fresh user
    /// message re-arms it). Turns serialize per session, so a session key is
    /// unambiguous.
    agent_chat_sessions: std::sync::Mutex<HashSet<String>>,
    /// The most recent top-level session per agent: `agent_id → (session_id,
    /// repo_path)`. Survives run completion (the run store is in-flight only), so
    /// `agent_chat` can deliver a message into the agent's current chat.
    latest_session_by_agent: std::sync::Mutex<HashMap<String, (String, String)>>,
    /// `config.pet.muted`, readable without the config lock — every spoken
    /// line checks it.
    pet_muted: AtomicBool,
    /// Which agents appear only once met, and which have been
    /// (`met.rs`) — the one gate every surface asks.
    pub met: met::MetGate,
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

    fn canonical_project_root(project_root: &Path) -> PathBuf {
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
    /// 4. First configured model
    fn resolve_model_id(
        config: &Config,
        models: &ModelManager,
        agent_id: &str,
        frontmatter_model: Option<String>,
    ) -> Result<String> {
        let model_ids: std::collections::HashSet<&str> =
            config.models.iter().map(|m| m.id.as_str()).collect();

        // 1. Config agent override
        if let Some(choice) =
            Self::normalize_model_choice(Self::model_override_for_agent(config, agent_id))
        {
            if models.has_model(&choice) {
                return Ok(choice);
            }
            warn!(
                "Agent override model '{}' not found in configured models; falling through",
                choice
            );
        }

        // 2. Frontmatter model
        if let Some(choice) = Self::normalize_model_choice(frontmatter_model) {
            if models.has_model(&choice) {
                return Ok(choice);
            }
            warn!(
                "Agent frontmatter model '{}' not found in configured models; falling through",
                choice
            );
        }

        // 3. First model in routing.default_models. Checked against the
        // ModelManager (not just config.models) so built-in models like
        // Linggen Cloud can be the default too. An entry whose credentials
        // are absent (signed-out cloud/ChatGPT) is passed over rather than
        // returned — a default that can only bail AUTH_REQUIRED must not
        // shadow a working model further down the chain. It is remembered,
        // though: if nothing else resolves, failing on it keeps the clear
        // sign-in message instead of "No models configured".
        let mut auth_dead_default: Option<String> = None;
        for dm in &config.routing.default_models {
            if model_ids.contains(dm.as_str()) || models.has_model(dm) {
                if models.model_auth_ok(dm) {
                    return Ok(dm.clone());
                }
                if auth_dead_default.is_none() {
                    auth_dead_default = Some(dm.clone());
                }
                warn!(
                    "Default model '{}' has no usable credentials right now; falling through",
                    dm
                );
            }
        }

        // 4. First configured model
        if let Some(m) = config.models.first() {
            return Ok(m.id.clone());
        }

        // Nothing else exists: hand back the auth-dead default so the turn
        // fails with its AUTH_REQUIRED sign-in message, not a config error.
        auth_dead_default.ok_or_else(|| anyhow::anyhow!("No models configured"))
    }

    fn make_run_id(&self, agent_id: &str) -> String {
        let mut counters = self.run_id_counters.lock_ok();
        let seq = counters.entry(agent_id.to_string()).or_insert(0);
        *seq += 1;
        format!("{}{:02}", agent_id, *seq)
    }

    /// Build a fully-initialized `AgentEngine` for the given project + agent_id.
    ///
    /// Centralizes the construction sequence shared by
    /// `get_or_create_session_agent` and `spawn_delegation_engine`:
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
        project_root: &Path,
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
            EngineConfig::from_app_config(&config, project_root.to_path_buf(), self.interface_mode),
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
        engine
            .load_available_skills_metadata(self.skills.as_ref())
            .await;
        engine
            .load_available_agents_metadata(self.agents.as_ref(), project_root)
            .await;
        // An agent not met yet is not one to hand work to.
        engine
            .available_agents_metadata
            .retain(|(name, _)| self.met.present(&Self::normalize_agent_id(name)));
        Ok(engine)
    }

    pub fn new(
        config: Config,
        config_dir: Option<PathBuf>,
        skills: Arc<dyn SkillRegistry>,
        agents: Arc<dyn AgentRegistry>,
        missions: Arc<dyn MissionRegistry>,
        interface_mode: InterfaceMode,
    ) -> (
        Arc<Self>,
        mpsc::UnboundedReceiver<(AgentEvent, Option<String>)>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let models = Arc::new(ModelManager::new(config.models.clone()));
        let pet_muted = AtomicBool::new(config.pet.muted);
        (
            Arc::new(Self {
                config: RwLock::new(config),
                config_dir,
                projects: Mutex::new(HashMap::new()),
                locks: Mutex::new(LockManager::new()),
                models: RwLock::new(models),
                missions,
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
                global_sessions: SessionStore::with_sessions_dir(
                    crate::paths::global_sessions_dir(),
                ),
                session_engines: Mutex::new(HashMap::new()),
                session_turns: std::sync::Mutex::new(HashMap::new()),
                run_id_counters: std::sync::Mutex::new(HashMap::new()),
                presence: std::sync::Mutex::new(PresenceBoard::default()),
                agent_chat_sessions: std::sync::Mutex::new(HashSet::new()),
                latest_session_by_agent: std::sync::Mutex::new(HashMap::new()),
                pet_muted,
                met: met::MetGate::new(crate::paths::met_dir()),
            }),
            rx,
        )
    }

    /// Whether the pet's voice is off on this machine.
    pub fn pet_muted(&self) -> bool {
        self.pet_muted.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Turn the pet's voice off or on. Saved with the config — without
    /// rebuilding any engine, since nothing a session runs with depends on
    /// it — and announced to every surface.
    pub async fn set_pet_muted(&self, muted: bool) -> Result<()> {
        {
            let mut cfg = self.config.write().await;
            if cfg.pet.muted != muted {
                let mut next = cfg.clone();
                next.pet.muted = muted;
                next.save_runtime(self.config_dir.as_deref())?;
                *cfg = next;
            }
        }
        self.pet_muted
            .store(muted, std::sync::atomic::Ordering::Relaxed);
        let _ = self.events.send((AgentEvent::PetVoice { muted }, None));
        Ok(())
    }

    pub fn register_tool_cancel_flag(&self, block_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.tool_cancel_flags
            .lock_ok()
            .insert(block_id.to_string(), Arc::clone(&flag));
        flag
    }

    pub fn clear_tool_cancel_flag(&self, block_id: &str) {
        self.tool_cancel_flags.lock_ok().remove(block_id);
    }

    /// Record a presence beat from a client surface. Carries no keystroke
    /// content — only recency, focus, and a typing flag. Each surface keeps its
    /// own reading (`presence.rs`), so one surface never overwrites another.
    pub fn update_presence(&self, beat: Beat) {
        let now = crate::util::now_ts_secs();
        self.presence.lock_ok().record(beat, now);
    }

    /// A person who just typed to an agent is present, wherever they typed it.
    ///
    /// The beat above is sent by browser surfaces only, so a turn from a skill
    /// page, the phone, or any client that never beats used to read as "away" —
    /// and Yinyue heralded "their reply is ready" at someone watching it arrive
    /// (2026-09-10, chatting with Ling on the DJ page). A message is the least
    /// deniable presence signal there is: they typed it a moment ago. It is a
    /// surface of its own, aging out like any other.
    pub fn mark_user_turn_presence(&self) {
        let app = self.presence_snapshot().app;
        self.update_presence(Beat {
            surface: Some("user-turn".to_string()),
            focused: true,
            typing: true,
            idle_ms: 0,
            app,
        });
    }

    /// The person's presence now — the most present live surface — for the
    /// `sense` tool and the herald watch.
    pub fn presence_snapshot(&self) -> Presence {
        self.presence.lock_ok().snapshot(crate::util::now_ts_secs())
    }

    /// Mark a session's current turn as woken by an `agent_chat` (loop-break).
    pub fn mark_agent_chat_session(&self, session_id: &str) {
        self.agent_chat_sessions
            .lock_ok()
            .insert(session_id.to_string());
    }

    /// Clear the agent_chat mark once the turn finishes.
    pub fn clear_agent_chat_session(&self, session_id: &str) {
        self.agent_chat_sessions.lock_ok().remove(session_id);
    }

    /// True while this session's current turn was woken by an `agent_chat` — the
    /// `agent_chat` tool refuses to relay onward, so the chain stops at one hop.
    pub fn is_agent_chat_session(&self, session_id: &str) -> bool {
        self.agent_chat_sessions.lock_ok().contains(session_id)
    }

    /// Record the agent's current top-level session (for `agent_chat` delivery).
    pub fn record_latest_session(&self, agent_id: &str, session_id: &str, repo_path: &str) {
        self.latest_session_by_agent.lock_ok().insert(
            agent_id.to_string(),
            (session_id.to_string(), repo_path.to_string()),
        );
    }

    /// The agent's most recent top-level `(session_id, repo_path)`, if any.
    pub fn latest_session(&self, agent_id: &str) -> Option<(String, String)> {
        self.latest_session_by_agent
            .lock_ok()
            .get(agent_id)
            .cloned()
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

    /// Get or create `agent_id`'s engine in a session — one per member.
    /// The session's turn lock (`session_turn`) keeps two from running at
    /// once; each member keeps its own cached thread, prompt and model.
    pub async fn get_or_create_session_agent(
        self: &Arc<Self>,
        session_id: &str,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> Result<Arc<Mutex<AgentEngine>>> {
        let normalized_id = Self::normalize_agent_id(agent_id);
        let key = (session_id.to_string(), normalized_id.clone());
        if let Some(engine) = self.session_engines.lock().await.get(&key) {
            return Ok(engine.clone());
        }

        // A session pinned to a workspace (a mission's, an agent run's)
        // wins over the caller's guess: the UI routes into a fresh mission
        // session before the scheduler dispatches, and the engine must
        // stand where the mission does.
        let meta = self
            .global_sessions
            .get_session_meta(session_id)
            .ok()
            .flatten();
        let pinned_root = meta
            .as_ref()
            .filter(|m| matches!(m.creator.as_str(), "mission" | "agent"))
            .and_then(|m| m.cwd.as_deref())
            .filter(|s| !s.trim().is_empty())
            .map(|cwd| crate::util::resolve_path(std::path::Path::new(cwd)));
        let project_root =
            pinned_root.unwrap_or_else(|| Self::canonical_project_root(project_root));
        let mut engine = self
            .build_engine_for_agent(&project_root, &normalized_id, true)
            .await?;

        // Apply any per-session compact config persisted in session.yaml so a
        // skill's previously-set threshold + focus survive engine restart
        // without needing to be re-pushed on every iframe mount.
        if let Some(meta) = &meta {
            engine.compact_threshold = meta.compact_threshold;
            engine.compact_focus = meta.compact_focus.clone();
        }

        let agent = Arc::new(Mutex::new(engine));
        // Re-check under lock to handle concurrent creation race.
        // If another task created the engine while we were building ours, use theirs.
        let mut engines = self.session_engines.lock().await;
        if let Some(existing) = engines.get(&key) {
            return Ok(existing.clone());
        }
        engines.insert(key, agent.clone());
        Ok(agent)
    }

    /// Every member's live engine in a session.
    pub async fn session_engines_of(&self, session_id: &str) -> Vec<Arc<Mutex<AgentEngine>>> {
        self.session_engines
            .lock()
            .await
            .iter()
            .filter(|((sid, _), _)| sid == session_id)
            .map(|(_, e)| e.clone())
            .collect()
    }

    /// Remove a session's engines when the session is deleted.
    pub async fn remove_session_engine(&self, session_id: &str) {
        self.session_engines
            .lock()
            .await
            .retain(|(sid, _), _| sid != session_id);
        self.session_turns.lock_ok().remove(session_id);
    }

    /// The session's turn lock. A turn — any member's, from any path —
    /// holds it from start to end, so two never run at once in a session.
    pub fn session_turn(&self, session_id: &str) -> Arc<Mutex<()>> {
        self.session_turns
            .lock_ok()
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    /// Fraction of its soft context limit a session's **live** thread is
    /// using — the fullest member's — or `None` when no member has a cached
    /// engine idle enough to read (mid-turn engines are locked). Yinyue's
    /// rolling-session resolver reads this to decide when to roll to a
    /// fresh segment instead of compacting a long companion thread.
    pub async fn session_context_fraction(&self, session_id: &str) -> Option<f32> {
        let mut most: Option<f32> = None;
        for engine_arc in self.session_engines_of(session_id).await {
            let Ok(engine) = engine_arc.try_lock() else {
                continue;
            };
            let limit = engine.context_soft_token_limit();
            if limit == 0 {
                continue;
            }
            let f = engine.accumulated_token_estimate as f32 / limit as f32;
            most = Some(most.map_or(f, |m| m.max(f)));
        }
        most
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
        let engines = self.session_engines_of(session_id).await;
        for engine_arc in &engines {
            let mut engine = engine_arc.lock().await;
            engine.session_permissions.set_path_mode(path, mode);
        }
        !engines.is_empty()
    }

    /// Create a fresh, uncached `AgentEngine` for a single delegation call.
    ///
    /// Unlike `get_or_create_session_agent`, the returned engine is **not** cached.  It is intended for one-shot delegation tasks that run concurrently — each
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

    /// The agents that are here: every installed spec, less the ones that
    /// appear only once met and have not been (`met.rs`). The listing every
    /// surface reads — the agent list, the `@` picker, the settings editor.
    pub async fn list_agent_specs(&self, project_root: &Path) -> Result<Vec<AgentSpecFile>> {
        let mut specs = self.agents.list(project_root).await?;
        specs.retain(|s| self.met.present(&s.agent_id));
        Ok(specs)
    }

    pub async fn list_agents(&self, project_root: &PathBuf) -> Result<Vec<AgentSpec>> {
        let mut out = Vec::new();
        for entry in self.list_agent_specs(project_root).await? {
            out.push(entry.spec);
        }
        Ok(out)
    }

    /// Every `(agent id, name)` a leading `@name` may address — ids, spec
    /// names and declared aliases of the agents a person may address.
    pub async fn mention_names(&self, project_root: &Path) -> Vec<(String, String)> {
        let Ok(specs) = self.list_agent_specs(project_root).await else {
            return Vec::new();
        };
        specs
            .iter()
            .flat_map(|s| {
                s.mention_names()
                    .into_iter()
                    .map(|n| (s.agent_id.clone(), n.to_string()))
            })
            .collect()
    }

    /// The model `agent_id` runs on by its own chain (config override,
    /// frontmatter, routing default) — what a fresh engine of it gets.
    pub async fn agent_model_id(&self, project_root: &Path, agent_id: &str) -> Option<String> {
        let normalized = Self::normalize_agent_id(agent_id);
        let spec = self.agents.find(project_root, &normalized).await.ok()??;
        let config = self.config.read().await.clone();
        let models = self.models.read().await.clone();
        Self::resolve_model_id(&config, &models, &normalized, spec.spec.model.clone()).ok()
    }

    pub async fn agent_exists(&self, project_root: &Path, agent_id: &str) -> bool {
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
        let was_muted = self
            .pet_muted
            .swap(new_config.pet.muted, std::sync::atomic::Ordering::Relaxed);
        if was_muted != new_config.pet.muted {
            let _ = self.events.send((
                AgentEvent::PetVoice {
                    muted: new_config.pet.muted,
                },
                None,
            ));
        }

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
        match self
            .global_sessions
            .update_last_plan_message(session_id, msg)
        {
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

    pub async fn set_pending_plan(
        &self,
        project_root: &str,
        agent_id: &str,
        session_id: Option<&str>,
        plan: Plan,
    ) {
        let sid = session_id.unwrap_or("default");
        let key = format!("{}|{}|{}", project_root, sid, agent_id);
        self.pending_plans.lock().await.insert(key, plan);
    }

    pub async fn take_pending_plan(
        &self,
        project_root: &str,
        agent_id: &str,
        session_id: Option<&str>,
    ) -> Option<Plan> {
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

    pub async fn sync_world_state(&self, project_root: &Path) -> Result<()> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf());
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
            // Drop the agents-map guard before waiting on the engine (held
            // for a whole turn).
            let worker = ctx.agents.lock().await.get(&patch_agent_id).cloned();
            if let Some(worker) = worker {
                let mut engine = worker.lock().await;
                let current_task = engine.get_task();
                if current_task.as_deref() != Some(body) {
                    tracing::info!("Syncing active task {} to {} agent", id, patch_agent_id);
                    engine.set_task(body.clone());
                    let _ = self.events.send((
                        AgentEvent::TaskUpdate {
                            agent_id: patch_agent_id,
                            task: body.clone(),
                        },
                        None,
                    ));
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

#[cfg(test)]
mod pet_voice_tests {
    use super::*;

    fn manager() -> Arc<AgentManager> {
        AgentManager::new(
            Config::default(),
            None,
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            InterfaceMode::Web,
        )
        .0
    }

    /// One agent speaks at a time in a session: two turns there — whoever
    /// runs them — serialize; another session's turn runs beside them.
    #[tokio::test]
    async fn two_turns_in_one_session_serialize_and_another_session_runs_beside() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let manager = manager();
        let in_s1 = Arc::new(AtomicUsize::new(0));
        let most_in_s1 = Arc::new(AtomicUsize::new(0));
        let both = Arc::new(AtomicUsize::new(0));
        let most_both = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for sid in ["s1", "s1", "s2"] {
            let turn = manager.session_turn(sid);
            let (in_s1, most_in_s1) = (in_s1.clone(), most_in_s1.clone());
            let (both, most_both) = (both.clone(), most_both.clone());
            tasks.push(tokio::spawn(async move {
                let _held = turn.lock_owned().await;
                let now = both.fetch_add(1, Ordering::SeqCst) + 1;
                most_both.fetch_max(now, Ordering::SeqCst);
                if sid == "s1" {
                    let now = in_s1.fetch_add(1, Ordering::SeqCst) + 1;
                    most_in_s1.fetch_max(now, Ordering::SeqCst);
                }
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                if sid == "s1" {
                    in_s1.fetch_sub(1, Ordering::SeqCst);
                }
                both.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(
            most_in_s1.load(Ordering::SeqCst),
            1,
            "never two at once in s1"
        );
        assert_eq!(most_both.load(Ordering::SeqCst), 2, "s2 ran beside s1");
        assert!(Arc::ptr_eq(
            &manager.session_turn("s1"),
            &manager.session_turn("s1")
        ));
    }

    #[tokio::test]
    async fn muting_is_saved_announced_and_read_without_the_config_lock() {
        let dir = tempfile::tempdir().unwrap();
        let (manager, mut rx) = AgentManager::new(
            Config::default(),
            Some(dir.path().to_path_buf()),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            InterfaceMode::Web,
        );
        assert!(!manager.pet_muted());

        manager.set_pet_muted(true).await.unwrap();
        assert!(manager.pet_muted());
        assert!(matches!(
            rx.recv().await,
            Some((AgentEvent::PetVoice { muted: true }, None))
        ));
        let saved = std::fs::read_to_string(Config::runtime_config_path(Some(dir.path()))).unwrap();
        assert!(saved.contains("muted = true"), "kept across a restart");
        assert!(manager.get_config_snapshot().await.pet.muted);

        manager.set_pet_muted(false).await.unwrap();
        assert!(!manager.pet_muted());
        assert!(matches!(
            rx.recv().await,
            Some((AgentEvent::PetVoice { muted: false }, None))
        ));
    }
}
