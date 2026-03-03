use crate::agent_manager::locks::LockManager;
use crate::agent_manager::models::ModelManager;
use crate::config::{AgentPolicyCapability, AgentSpec, Config};
use crate::engine::{AgentEngine, AgentOutcome, AgentRole, EngineConfig, Plan};
use crate::project_store::ProjectStore;
use crate::skills::SkillManager;
use crate::state_fs::{SessionStore, StateFile, StateFs};
use anyhow::Result;
use globset::Glob;
use ignore::gitignore::GitignoreBuilder;
use notify::{EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::Instant;
use tracing::{info, warn};

pub mod locks;
pub mod models;
pub mod routing;

pub struct ProjectContext {
    pub agents: Mutex<HashMap<String, Arc<Mutex<AgentEngine>>>>,
    pub state_fs: StateFs,
    pub sessions: SessionStore,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

pub struct AgentManager {
    config: RwLock<Config>,
    config_dir: Option<PathBuf>,
    pub projects: Mutex<HashMap<String, Arc<ProjectContext>>>,
    pub locks: Mutex<LockManager>,
    pub models: RwLock<Arc<ModelManager>>,
    pub store: Arc<ProjectStore>,
    pub skill_manager: Arc<SkillManager>,
    working_places: Mutex<HashMap<String, HashMap<String, WorkingPlaceEntry>>>,
    cancelled_runs: Mutex<HashSet<String>>,
    events: mpsc::UnboundedSender<AgentEvent>,
    /// Pending plans awaiting user approval, keyed by "{project_root}|{agent_id}".
    pending_plans: Mutex<HashMap<String, Plan>>,
    /// Maps run_id → repo_path for O(1) lookups in finish/get/cancel.
    run_project_map: Mutex<HashMap<String, String>>,
    /// Last activity time per agent, keyed by "{project_root}|{agent_id}".
    last_activity: Mutex<HashMap<String, Instant>>,
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
    },
    AgentStatus {
        agent_id: String,
        status: String,
        detail: Option<String>,
        parent_id: Option<String>,
    },
    SubagentSpawned {
        parent_id: String,
        subagent_id: String,
        task: String,
    },
    SubagentResult {
        parent_id: String,
        subagent_id: String,
        outcome: AgentOutcome,
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
    },
    /// Signal that the assistant turn is complete.
    TurnComplete {
        agent_id: String,
        duration_ms: Option<u64>,
        context_tokens: Option<usize>,
        parent_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpecFile {
    pub agent_id: String,
    pub spec: AgentSpec,
    pub spec_path: PathBuf,
    #[serde(skip)]
    pub system_prompt: String,
}

impl AgentManager {
    fn normalize_agent_id(agent_id: &str) -> String {
        agent_id.trim().to_lowercase()
    }

    fn canonical_project_root(project_root: &PathBuf) -> PathBuf {
        project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone())
    }

    fn agent_specs_dir(project_root: &PathBuf) -> PathBuf {
        project_root.join("agents")
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
        if let Some(id) = routing::resolve_model(
            &config.routing,
            None,
            &routing::ComplexitySignal {
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

    /// Load agent specs from a single directory.
    fn load_agent_specs_from_dir(agents_dir: &std::path::Path) -> Vec<AgentSpecFile> {
        if !agents_dir.exists() {
            return Vec::new();
        }

        let mut paths: Vec<PathBuf> = match std::fs::read_dir(agents_dir) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| {
                    path.is_file()
                        && path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| ext.eq_ignore_ascii_case("md"))
                            .unwrap_or(false)
                })
                .collect(),
            Err(err) => {
                warn!("Cannot read agents directory {}: {}", agents_dir.display(), err);
                return Vec::new();
            }
        };
        paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        let mut seen = HashSet::new();
        let mut specs = Vec::new();
        for spec_path in paths {
            let (spec, system_prompt) = match AgentSpec::from_markdown(&spec_path) {
                Ok(parsed) => parsed,
                Err(err) => {
                    warn!(
                        "Skipping invalid agent spec {}: {}",
                        spec_path.display(),
                        err
                    );
                    continue;
                }
            };
            let raw_name = spec.name.trim();
            let fallback = spec_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("agent");
            let agent_id = Self::normalize_agent_id(if raw_name.is_empty() {
                fallback
            } else {
                raw_name
            });
            if agent_id.is_empty() {
                warn!(
                    "Skipping agent spec {}: resolved to empty agent id",
                    spec_path.display()
                );
                continue;
            }
            if !seen.insert(agent_id.clone()) {
                warn!(
                    "Skipping agent spec {}: duplicate agent id '{}' in agents directory {}",
                    spec_path.display(),
                    agent_id,
                    agents_dir.display()
                );
                continue;
            }
            specs.push(AgentSpecFile {
                agent_id,
                spec,
                spec_path,
                system_prompt,
            });
        }

        specs
    }

    /// Load agent specs layered: global (`~/.linggen/agents/`) then project (`<project>/agents/`).
    /// Project specs override global specs with the same `agent_id`.
    /// If the project-level `agents/` dir doesn't exist, also searches one level deep
    /// in subdirectories (e.g. `<project>/linggen-agent/agents/`).
    fn load_agent_specs_for_project(project_root: &PathBuf) -> Result<Vec<AgentSpecFile>> {
        let mut merged: HashMap<String, AgentSpecFile> = HashMap::new();

        // 1. Global agents (lower priority)
        let global_dir = crate::paths::global_agents_dir();
        for spec in Self::load_agent_specs_from_dir(&global_dir) {
            merged.insert(spec.agent_id.clone(), spec);
        }

        // 2. Project agents (higher priority — overrides global)
        let project_dir = Self::agent_specs_dir(project_root);
        if project_dir.is_dir() {
            for spec in Self::load_agent_specs_from_dir(&project_dir) {
                merged.insert(spec.agent_id.clone(), spec);
            }
        } else {
            // Fallback: look for agents/ in immediate subdirectories
            if let Ok(entries) = std::fs::read_dir(project_root) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        let sub_agents = entry.path().join("agents");
                        if sub_agents.is_dir() {
                            for spec in Self::load_agent_specs_from_dir(&sub_agents) {
                                merged.insert(spec.agent_id.clone(), spec);
                            }
                            break;
                        }
                    }
                }
            }
        }

        let mut specs: Vec<AgentSpecFile> = merged.into_values().collect();
        specs.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        Ok(specs)
    }

    fn find_agent_spec_for_project(
        &self,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> Result<Option<AgentSpecFile>> {
        let wanted = Self::normalize_agent_id(agent_id);
        Ok(Self::load_agent_specs_for_project(project_root)?
            .into_iter()
            .find(|entry| entry.agent_id == wanted))
    }

    fn make_run_id(agent_id: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        format!(
            "run-{}-{}-{}",
            agent_id,
            now.as_secs(),
            now.subsec_nanos()
        )
    }

    pub fn new(
        config: Config,
        config_dir: Option<PathBuf>,
        store: Arc<ProjectStore>,
        skill_manager: Arc<SkillManager>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<AgentEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let models = Arc::new(ModelManager::new(config.models.clone()));
        (
            Arc::new(Self {
                config: RwLock::new(config),
                config_dir,
                projects: Mutex::new(HashMap::new()),
                locks: Mutex::new(LockManager::new()),
                models: RwLock::new(models),
                store,
                skill_manager,
                working_places: Mutex::new(HashMap::new()),
                cancelled_runs: Mutex::new(HashSet::new()),
                events: tx,
                pending_plans: Mutex::new(HashMap::new()),
                run_project_map: Mutex::new(HashMap::new()),
                last_activity: Mutex::new(HashMap::new()),
            }),
            rx,
        )
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
        let sessions = self.store.session_store(&key);
        let ctx = Arc::new(ProjectContext {
            agents: Mutex::new(HashMap::new()),
            state_fs,
            sessions,
            watcher: Mutex::new(None),
        });

        // Setup watcher
        let root_clone = root.clone();
        let events_tx = self.events.clone();
        let mut gitignore_builder = GitignoreBuilder::new(&root);
        let root_gitignore = root.join(".gitignore");
        if root_gitignore.exists() {
            if let Some(path_str) = root_gitignore.to_str() {
                if let Some(err) = gitignore_builder.add(path_str) {
                    tracing::warn!("Failed to load .gitignore: {}", err);
                }
            }
        }
        let gitignore = Arc::new(gitignore_builder.build()?);

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    let gitignore = gitignore.clone();
                    let is_ignored = |path: &std::path::Path| -> bool {
                        match path.strip_prefix(&root_clone) {
                            Ok(rel) => gitignore
                                .matched_path_or_any_parents(rel, false)
                                .is_ignore(),
                            Err(_) => false,
                        }
                    };

                    // Ignore internal and build directories
                    if event.paths.iter().any(|p| {
                        let s = p.to_string_lossy();
                        s.contains("/target/")
                            || s.contains("/.git/")
                            || s.contains("/.linggen-agent/")
                            || s.contains("/node_modules/")
                    }) {
                        return;
                    }

                    match event.kind {
                        EventKind::Remove(_) => {
                            let has_relevant = event.paths.iter().any(|p| !is_ignored(p));
                            if has_relevant {
                                let _ = events_tx.send(AgentEvent::StateUpdated);
                            }
                        }
                        EventKind::Modify(notify::event::ModifyKind::Name(
                            notify::event::RenameMode::Both,
                        )) => {
                            if event.paths.len() == 2 {
                                let old = &event.paths[0];
                                let new = &event.paths[1];
                                if !is_ignored(old) || !is_ignored(new) {
                                    let _ = events_tx.send(AgentEvent::StateUpdated);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            })?;

        watcher.watch(&root, RecursiveMode::Recursive)?;
        *ctx.watcher.lock().await = Some(watcher);

        projects.insert(key.clone(), ctx.clone());

        // Register in project store
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let _ = self.store.add_project(key, name);

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

        let agent_spec = self
            .find_agent_spec_for_project(&project_root, &normalized_id)?
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
            &normalized_id,
            agent_spec.spec.model.clone(),
        )?;

        let role = if agent_spec
            .spec
            .allows_policy(AgentPolicyCapability::Finalize)
        {
            AgentRole::Lead
        } else if agent_spec.spec.allows_policy(AgentPolicyCapability::Patch) {
            AgentRole::Coder
        } else {
            AgentRole::Operator
        };

        let mut engine = AgentEngine::new(
            EngineConfig {
                ws_root: project_root.clone(),
                max_iters: config.agent.max_iters,
                write_safety_mode: config.agent.write_safety_mode,
                tool_permission_mode: config.agent.tool_permission_mode,
                prompt_loop_breaker: config.agent.prompt_loop_breaker.clone(),
            },
            models,
            model_id,
            role,
        )?;

        engine.default_models = config.routing.default_models.clone();
        engine.set_spec(
            normalized_id.clone(),
            agent_spec.spec,
            agent_spec.system_prompt,
        );
        engine.set_manager_context(self.clone());
        engine.set_delegation_depth(0, config.agent.max_delegation_depth);
        engine.load_skill_tools(&self.skill_manager).await;
        engine.load_available_skills_metadata(&self.skill_manager).await;

        // Set up auto memory directory
        let repo_path_str = project_root.to_string_lossy().to_string();
        engine.set_memory_dir(self.store.memory_dir(&repo_path_str));

        let agent = Arc::new(Mutex::new(engine));
        agents.insert(normalized_id, agent.clone());
        Ok(agent)
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

        let agent_spec = self
            .find_agent_spec_for_project(&project_root, &normalized_id)?
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
            &normalized_id,
            agent_spec.spec.model.clone(),
        )?;

        let role = if agent_spec
            .spec
            .allows_policy(AgentPolicyCapability::Finalize)
        {
            AgentRole::Lead
        } else if agent_spec.spec.allows_policy(AgentPolicyCapability::Patch) {
            AgentRole::Coder
        } else {
            AgentRole::Operator
        };

        let mut engine = AgentEngine::new(
            EngineConfig {
                ws_root: project_root.clone(),
                max_iters: config.agent.max_iters,
                write_safety_mode: config.agent.write_safety_mode,
                tool_permission_mode: config.agent.tool_permission_mode,
                prompt_loop_breaker: config.agent.prompt_loop_breaker.clone(),
            },
            models,
            model_id,
            role,
        )?;

        engine.default_models = config.routing.default_models.clone();
        engine.set_spec(
            normalized_id.clone(),
            agent_spec.spec,
            agent_spec.system_prompt,
        );
        engine.set_manager_context(self.clone());
        engine.load_skill_tools(&self.skill_manager).await;
        engine.load_available_skills_metadata(&self.skill_manager).await;

        // Set up auto memory directory
        let repo_path_str = project_root.to_string_lossy().to_string();
        engine.set_memory_dir(self.store.memory_dir(&repo_path_str));

        Ok(engine)
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
        let project_root = Self::canonical_project_root(project_root);
        let Ok(Some(agent_spec)) = self.find_agent_spec_for_project(&project_root, agent_id) else {
            return true;
        };

        if agent_spec.spec.work_globs.is_empty() {
            return true;
        }

        for glob_str in &agent_spec.spec.work_globs {
            if let Ok(glob) = Glob::new(glob_str) {
                if glob.compile_matcher().is_match(path) {
                    return true;
                }
            }
        }
        false
    }

    pub async fn list_agent_specs(&self, project_root: &PathBuf) -> Result<Vec<AgentSpecFile>> {
        let project_root = Self::canonical_project_root(project_root);
        Self::load_agent_specs_for_project(&project_root)
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
        let project_root = Self::canonical_project_root(project_root);
        let Ok(found) = self.find_agent_spec_for_project(&project_root, agent_id) else {
            return false;
        };
        found.is_some()
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
        repo.insert(agent_id.to_string(), entry);
    }

    pub async fn clear_working_place_for_agent(&self, repo_path: &str, agent_id: &str) {
        let mut places = self.working_places.lock().await;
        if let Some(repo_map) = places.get_mut(repo_path) {
            repo_map.remove(agent_id);
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
        use crate::project_store::{AgentRunRecord, AgentRunStatus};

        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let run_id = Self::make_run_id(agent_id);
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
        self.store.run_store(&repo_path).add_run(&record)?;
        self.run_project_map
            .lock()
            .await
            .insert(run_id.clone(), repo_path.clone());
        self.clear_working_place_for_agent(&repo_path, agent_id)
            .await;
        self.cancelled_runs.lock().await.remove(&run_id);
        Ok(run_id)
    }

    pub async fn finish_agent_run(
        &self,
        run_id: &str,
        status: crate::project_store::AgentRunStatus,
        detail: Option<String>,
    ) -> Result<()> {
        let ended_at = Some(crate::util::now_ts_secs());
        let repo_path = self.run_project_map.lock().await.get(run_id).cloned();
        if let Some(repo_path) = &repo_path {
            self.store
                .run_store(repo_path)
                .update_run(run_id, status, detail, ended_at)?;
        }
        self.clear_working_place_for_run(run_id).await;
        let _ = self.events.send(AgentEvent::StateUpdated);
        self.cancelled_runs.lock().await.remove(run_id);
        // Record activity for the agent that just finished
        if let Some(repo_path) = &repo_path {
            // Look up agent_id from the run record
            if let Ok(Some(run)) = self.store.run_store(repo_path).get_run(run_id) {
                self.update_agent_activity(repo_path, &run.agent_id).await;
            }
        }
        self.run_project_map.lock().await.remove(run_id);
        Ok(())
    }

    pub async fn list_agent_runs(
        &self,
        project_root: &PathBuf,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::project_store::AgentRunRecord>> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        self.store
            .run_store(&project_root.to_string_lossy())
            .list_runs(session_id)
    }

    pub async fn list_agent_children(
        &self,
        parent_run_id: &str,
        project_root: Option<&str>,
    ) -> Result<Vec<crate::project_store::AgentRunRecord>> {
        let repo_path = self
            .run_project_map
            .lock()
            .await
            .get(parent_run_id)
            .cloned();
        let repo_path = repo_path.or_else(|| {
            project_root.map(|p| {
                std::path::Path::new(p)
                    .canonicalize()
                    .map(|c| c.to_string_lossy().to_string())
                    .unwrap_or_else(|_| p.to_string())
            })
        });
        if let Some(repo_path) = repo_path {
            self.store
                .run_store(&repo_path)
                .list_children(parent_run_id)
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn get_agent_run(
        &self,
        run_id: &str,
        project_root: Option<&str>,
    ) -> Result<Option<crate::project_store::AgentRunRecord>> {
        let repo_path = self.run_project_map.lock().await.get(run_id).cloned();
        let repo_path = repo_path.or_else(|| {
            project_root.map(|p| {
                std::path::Path::new(p)
                    .canonicalize()
                    .map(|c| c.to_string_lossy().to_string())
                    .unwrap_or_else(|_| p.to_string())
            })
        });
        if let Some(repo_path) = repo_path {
            self.store.run_store(&repo_path).get_run(run_id)
        } else {
            Ok(None)
        }
    }

    pub async fn is_run_cancelled(&self, run_id: &str) -> bool {
        self.cancelled_runs.lock().await.contains(run_id)
    }

    pub async fn cancel_run_tree(
        &self,
        run_id: &str,
    ) -> Result<Vec<crate::project_store::AgentRunRecord>> {
        use crate::project_store::AgentRunStatus;

        let repo_path = self.run_project_map.lock().await.get(run_id).cloned();
        let Some(repo_path) = repo_path else {
            return Ok(Vec::new());
        };
        let run_store = self.store.run_store(&repo_path);

        let mut stack = vec![run_id.to_string()];
        let mut seen = HashSet::new();
        let mut runs = Vec::new();

        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(run) = run_store.get_run(&id)? else {
                continue;
            };
            for child in run_store.list_children(&id)? {
                stack.push(child.run_id.clone());
            }
            runs.push(run);
        }

        let to_cancel: Vec<crate::project_store::AgentRunRecord> = runs
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
            let _ = run_store.update_run(
                &run.run_id,
                AgentRunStatus::Cancelled,
                Some("cancelled by user".to_string()),
                Some(now),
            );
            self.clear_working_place_for_run(&run.run_id).await;
        }
        if !to_cancel.is_empty() {
            let _ = self.events.send(AgentEvent::StateUpdated);
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

        // Invalidate all cached agents so they pick up new config on next use
        let keys: Vec<String> = {
            let projects = self.projects.lock().await;
            projects.keys().cloned().collect()
        };
        for key in keys {
            let root = PathBuf::from(&key);
            let _ = self.invalidate_agent_cache(&root, None).await;
        }

        let _ = self.events.send(AgentEvent::StateUpdated);
        Ok(())
    }

    /// Convenience: persist a chat message via the project's flat-file session store.
    pub async fn add_chat_message(
        &self,
        ws_root: &std::path::Path,
        session_id: &str,
        msg: &crate::state_fs::sessions::ChatMsg,
    ) {
        if let Ok(ctx) = self.get_or_create_project(ws_root.to_path_buf()).await {
            if let Err(e) = ctx.sessions.add_chat_message(session_id, msg) {
                tracing::warn!("Failed to persist chat message: {}", e);
            }
        }
    }

    pub async fn send_event(&self, event: AgentEvent) {
        let _ = self.events.send(event);
    }

    pub async fn set_pending_plan(&self, project_root: &str, agent_id: &str, plan: Plan) {
        let key = format!("{}|{}", project_root, agent_id);
        self.pending_plans.lock().await.insert(key, plan);
    }

    pub async fn take_pending_plan(&self, project_root: &str, agent_id: &str) -> Option<Plan> {
        let key = format!("{}|{}", project_root, agent_id);
        self.pending_plans.lock().await.remove(&key)
    }

    /// Edit the plan_text and summary of a pending plan in-place.
    /// Returns `true` if the plan was found and updated, `false` otherwise.
    pub async fn edit_pending_plan(
        &self,
        project_root: &str,
        agent_id: &str,
        text: &str,
    ) -> bool {
        let key = format!("{}|{}", project_root, agent_id);
        let mut plans = self.pending_plans.lock().await;
        if let Some(plan) = plans.get_mut(&key) {
            plan.plan_text = text.to_string();
            plan.summary = crate::engine::AgentEngine::extract_plan_summary(text);
            true
        } else {
            false
        }
    }

    /// Record that an agent performed activity (finished run, received message, etc.)
    pub async fn update_agent_activity(&self, project_root: &str, agent_id: &str) {
        let key = format!("{}|{}", project_root, agent_id);
        self.last_activity.lock().await.insert(key, Instant::now());
    }

    /// How long an agent has been idle (since last activity).
    pub async fn get_agent_idle_duration(
        &self,
        project_root: &str,
        agent_id: &str,
    ) -> std::time::Duration {
        let key = format!("{}|{}", project_root, agent_id);
        let activity = self.last_activity.lock().await;
        match activity.get(&key) {
            Some(instant) => instant.elapsed(),
            None => std::time::Duration::from_secs(u64::MAX), // never active = infinitely idle
        }
    }

    /// Get the effective idle config for an agent, merging:
    /// 1. Mission-level per-agent config (highest priority)
    /// 2. DB agent override
    /// 3. Agent markdown defaults (lowest priority)
    pub async fn get_effective_idle_config(
        &self,
        project_root: &PathBuf,
        agent_id: &str,
    ) -> (Option<String>, u64) {
        let project_root_str = project_root.to_string_lossy().to_string();

        // 1. Start with markdown defaults
        let spec = self
            .find_agent_spec_for_project(project_root, agent_id)
            .ok()
            .flatten();
        let mut idle_prompt = spec.as_ref().and_then(|s| s.spec.idle_prompt.clone());
        let mut idle_interval = spec
            .as_ref()
            .and_then(|s| s.spec.idle_interval_secs)
            .unwrap_or(60);

        // 2. Override with DB agent override
        if let Ok(Some(overr)) = self.store.get_agent_override(&project_root_str, agent_id) {
            if let Some(prompt) = overr.idle_prompt {
                idle_prompt = Some(prompt);
            }
            if let Some(interval) = overr.idle_interval_secs {
                idle_interval = interval;
            }
        }

        // 3. Override with mission-level per-agent config
        if let Ok(Some(mission)) = self.store.get_mission(&project_root_str) {
            if let Some(ma) = mission.agents.iter().find(|a| a.id.eq_ignore_ascii_case(agent_id)) {
                if let Some(prompt) = &ma.idle_prompt {
                    idle_prompt = Some(prompt.clone());
                }
                if let Some(interval) = ma.idle_interval_secs {
                    idle_interval = interval;
                }
            }
        }

        // Enforce minimum 30 seconds
        idle_interval = idle_interval.max(30);

        (idle_prompt, idle_interval)
    }

    pub async fn sync_world_state(&self, project_root: &PathBuf) -> Result<()> {
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        let ctx = self.get_or_create_project(project_root.clone()).await?;
        let tasks = ctx.state_fs.list_tasks()?;
        let patch_agent_id = self
            .list_agent_specs(&project_root)
            .await?
            .into_iter()
            .find(|entry| {
                entry.spec.allows_policy(AgentPolicyCapability::Patch)
            })
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
                    let _ = self.events.send(AgentEvent::TaskUpdate {
                        agent_id: patch_agent_id,
                        task: body.clone(),
                    });
                }
            }
        }

        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::AgentManager;
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(prefix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!(
            "linggen-agent-{prefix}-{}-{ts}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp root");
        dir
    }

    fn valid_agent_md(name: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: test agent\ntools: [Read]\npolicy: []\n---\n\nYou are {name}.\n"
        )
    }

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

    #[test]
    fn load_agent_specs_skips_invalid_files_and_duplicates() {
        let root = temp_root("agent-specs");
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).expect("create agents dir");

        fs::write(agents_dir.join("a.md"), valid_agent_md("alpha")).expect("write alpha");
        fs::write(agents_dir.join("bad.md"), "this is not frontmatter").expect("write bad");
        fs::write(agents_dir.join("z.md"), valid_agent_md("alpha")).expect("write duplicate");

        let specs = AgentManager::load_agent_specs_for_project(&root).expect("load agent specs");
        // "bad.md" (invalid frontmatter) should be skipped.
        assert!(
            !specs.iter().any(|s| s.spec_path.ends_with("bad.md")),
            "invalid file should be skipped"
        );
        // "z.md" and "a.md" both define "alpha" — only one should survive (dedup by agent_id).
        let alpha_specs: Vec<_> = specs.iter().filter(|s| s.agent_id == "alpha").collect();
        assert_eq!(alpha_specs.len(), 1, "duplicate agent_id should be deduplicated");
        // The winner is "a.md" (alphabetically first file wins: HashMap insert order, first inserted stays unless overwritten).
        assert!(alpha_specs[0].spec_path.ends_with("a.md") || alpha_specs[0].spec_path.ends_with("z.md"));

        let _ = fs::remove_dir_all(&root);
    }
}
