use anyhow::Context;
use axum::{extract::DefaultBodyLimit, routing::delete, routing::get, routing::post, Router};
use dashmap::DashMap;
use dirs::data_dir;
use embeddings::{EmbeddingModel, TextChunker};
use std::net::SocketAddr;
use std::time::Duration;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use storage::{MetadataStore, VectorStore};
use tokio::sync::RwLock;
use tower_http::services::{ServeDir, ServeFile};
use tracing::{info, warn};

use crate::analytics;
use crate::handlers::{
    add_resource, apply_pack, cancel_job, chat_stream, classify_intent, clear_all_data,
    create_folder, create_pack, delete_folder, delete_pack, delete_uploaded_file, enhance_prompt,
    get_app_status, get_graph, get_graph_status, get_graph_with_status, get_pack, index_source,
    list_folders, list_jobs, list_packs, list_resources, list_uploaded_files,
    mcp::{mcp_health_handler, mcp_message_handler, mcp_sse_handler, McpAppState, McpState},
    rebuild_graph, remove_resource, rename_folder, rename_pack, rename_resource, retry_init,
    save_pack, update_resource_patterns, upload_file, upload_file_stream, AppState,
};
use crate::job_manager::JobManager;

//.linggen/memory/2025-12-16T19:20:50.051071+00:00__hello-memory-test__mem_48748d35-9c3f-4526-b85e-c84dc5f9ef92.md
#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    // On Unix, `kill(pid, 0)` checks for existence without sending a signal.
    // If the PID doesn't exist, errno = ESRCH.
    unsafe {
        let res = libc::kill(pid as i32, 0);
        if res == 0 {
            return true;
        }
        // If we don't have permission, the process still exists.
        std::io::Error::last_os_error()
            .raw_os_error()
            .is_some_and(|e| e == libc::EPERM)
    }
}

#[cfg(unix)]
fn get_parent_pid() -> u32 {
    unsafe { libc::getppid() as u32 }
}

#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> bool {
    true
}

/// Seed the library directory with default folders and packs
fn seed_library(library_path: &Path) -> anyhow::Result<()> {
    if !library_path.exists() {
        std::fs::create_dir_all(library_path)?;
    }

    // ----------------------------------------------------------------------------
    // Install/upgrade-time trigger
    // ----------------------------------------------------------------------------
    // We treat library seeding as an install/upgrade-time migration:
    // - On first app launch after install, marker is missing → seed.
    // - On first app launch after upgrade, marker differs → seed.
    // Seeding never overwrites existing user files; it only copies missing templates.
    let current_version = env!("CARGO_PKG_VERSION");
    let marker_path = library_path.join(".linggen_library_seed_version");
    let previous_version = std::fs::read_to_string(&marker_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let is_first_run_or_upgrade = previous_version.as_deref() != Some(current_version);

    info!(
        "Seeding Library with default folders and packs at {:?}",
        library_path
    );
    if is_first_run_or_upgrade {
        info!(
            "Library seed trigger: {} → {}",
            previous_version.as_deref().unwrap_or("<none>"),
            current_version
        );
    } else {
        info!(
            "Library seed trigger: unchanged ({}), skipping template sync",
            current_version
        );
    }

    // ----------------------------------------------------------------------------
    // Template sync (filesystem-based; no hardcoded list)
    // ----------------------------------------------------------------------------
    fn find_library_templates_dir() -> Option<PathBuf> {
        // 0) Explicit override (useful for debugging / custom packaging).
        if let Ok(dir) = std::env::var("LINGGEN_LIBRARY_TEMPLATES_DIR") {
            let p = PathBuf::from(dir);
            if p.is_dir() {
                return Some(p);
            }
        }

        let mut candidates: Vec<PathBuf> = Vec::new();

        // 0.5) Tauri provides the resources directory at runtime in some environments.
        // Prefer it if available (cross-platform).
        if let Ok(dir) = std::env::var("TAURI_RESOURCE_DIR") {
            let base = PathBuf::from(dir);
            candidates.push(base.join("library_templates"));
            candidates.push(base.join("resources").join("library_templates"));
        }

        // 1) Resolve relative to the server executable (best for release apps / Finder launches).
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                // Common layouts for "installed binary" distributions:
                // - <prefix>/bin/linggen-server
                // - <prefix>/share/linggen/library_templates
                // - <prefix>/bin/library_templates (next to binary)
                candidates.push(exe_dir.join("library_templates"));
                candidates.push(exe_dir.join("../library_templates"));
                candidates.push(exe_dir.join("../share/linggen/library_templates"));

                // Windows-friendly layouts (or generic "resources" folder next to binary)
                candidates.push(exe_dir.join("resources").join("library_templates"));
                candidates.push(exe_dir.join("../resources").join("library_templates"));

                candidates.push(exe_dir.join("../Resources/library_templates"));
                // Tauri bundles `bundle.resources` under `Contents/Resources/resources/...` by default.
                candidates.push(exe_dir.join("../Resources/resources/library_templates"));
                candidates.push(exe_dir.join("../../Resources/library_templates"));
                candidates.push(exe_dir.join("../../Resources/resources/library_templates"));
            }
        }

        // 2) Resolve relative to this crate (useful for local dev builds).
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("library_templates"));

        // 3) Optional conventional locations (if an installer drops templates here).
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".linggen").join("library_templates"));
        }

        // 4) OS-specific application data locations (macOS Application Support, Linux XDG data dir, Windows AppData).
        // `dirs::data_dir()` typically resolves to:
        // - macOS: ~/Library/Application Support
        // - Linux: ~/.local/share (or XDG_DATA_HOME)
        // - Windows: %APPDATA%
        if let Some(data_dir) = dirs::data_dir() {
            candidates.push(data_dir.join("Linggen").join("library_templates"));
            candidates.push(data_dir.join("linggen").join("library_templates"));
        }

        // 5) Windows system-wide data dir (optional installer target)
        if let Ok(program_data) = std::env::var("PROGRAMDATA") {
            candidates.push(
                PathBuf::from(program_data)
                    .join("Linggen")
                    .join("library_templates"),
            );
        }

        candidates.into_iter().find(|p| p.is_dir())
    }

    fn sync_missing_templates(src_root: &Path, dst_root: &Path) -> anyhow::Result<usize> {
        fn walk(
            cur: &Path,
            src_root: &Path,
            dst_root: &Path,
            copied: &mut usize,
        ) -> anyhow::Result<()> {
            for entry in std::fs::read_dir(cur)? {
                let entry = entry?;
                let path = entry.path();

                // Skip dotfiles/directories.
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') {
                        continue;
                    }
                }

                if path.is_dir() {
                    walk(&path, src_root, dst_root, copied)?;
                    continue;
                }

                // Only copy markdown templates.
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }

                let rel = path.strip_prefix(src_root).context("strip_prefix failed")?;
                let dst_path = dst_root.join(rel);

                // Never overwrite existing user files.
                if dst_path.exists() {
                    continue;
                }

                if let Some(parent) = dst_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                std::fs::copy(&path, &dst_path)?;
                *copied += 1;
            }
            Ok(())
        }

        let mut copied = 0usize;
        walk(src_root, src_root, dst_root, &mut copied)?;
        Ok(copied)
    }

    if is_first_run_or_upgrade {
        match find_library_templates_dir() {
            Some(src_root) => {
                let copied = sync_missing_templates(&src_root, library_path)?;
                info!(
                    "Library templates synced from {:?} -> {:?} ({} new files)",
                    src_root, library_path, copied
                );
            }
            None => {
                warn!(
                    "Library templates dir not found; skipping template sync into {:?}",
                    library_path
                );
            }
        }
    }

    // Update marker on first run/upgrade (best effort; don't fail startup).
    if is_first_run_or_upgrade {
        if let Err(e) = std::fs::write(&marker_path, format!("{}\n", current_version)) {
            warn!(
                "Failed to write library seed marker at {:?}: {}",
                marker_path, e
            );
        }
    }

    Ok(())
}

pub async fn start_server(port: u16, parent_pid: Option<u32>) -> anyhow::Result<()> {
    // Initialize logging with custom format
    tracing_subscriber::fmt()
        .with_target(false) // Remove module path
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(true) // Show file name
        .with_line_number(true) // Show line number
        .with_level(true)
        .compact() // Use compact format
        .init();

    info!("Linggen Backend API starting on port {}...", port);

    // If a parent PID is provided (desktop sidecar mode), exit automatically when parent exits.
    if let Some(ppid) = parent_pid {
        info!(
            "Parent PID set to {} (will auto-exit when parent exits)",
            ppid
        );
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if !pid_is_alive(ppid) {
                    eprintln!(
                        "[linggen-server] Parent PID {} exited; shutting down backend sidecar",
                        ppid
                    );
                    std::process::exit(0);
                }

                // Check for orphaned process (adopted by init)
                #[cfg(unix)]
                if get_parent_pid() == 1 {
                    eprintln!(
                        "[linggen-server] Backend sidecar was orphaned (adopted by init); shutting down"
                    );
                    std::process::exit(0);
                }
            }
        });
    }

    // Initialize embedding model and vector store
    // Use a stable application data directory so the app works
    // regardless of where it's launched from (Finder, CLI, DMG-installed, etc.)
    info!("Initializing metadata store...");

    // Allow overriding the data directory to run multiple instances (e.g. dev server alongside app).
    let base_data_dir = if let Ok(dir) = std::env::var("LINGGEN_DATA_DIR") {
        PathBuf::from(dir)
    } else {
        // Priority:
        // 1. macOS/Linux standard App Data dir (~/Library/Application Support/Linggen)
        // 2. If that fails, ONLY THEN fall back to current directory (dev mode)
        data_dir().map(|d| d.join("Linggen")).unwrap_or_else(|| {
            // Dev fallback: look for a 'data' folder in the workspace
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            if cwd.join("data").is_dir() {
                cwd.join("data")
            } else {
                cwd.join("Linggen")
            }
        })
    };

    let metadata_path = base_data_dir.join("metadata.redb");
    let lancedb_path = base_data_dir.join("lancedb");

    // Library path: prioritize ~/.linggen/library
    let library_path = dirs::home_dir()
        .map(|h| h.join(".linggen").join("library"))
        .unwrap_or_else(|| base_data_dir.join("library"));

    info!("Using metadata store at {:?}", metadata_path);
    info!("Using LanceDB store at {:?}", lancedb_path);
    info!("Using Library store at {:?}", library_path);

    // Seed Library with default packs
    if let Err(e) = seed_library(&library_path) {
        warn!("Failed to seed library: {}", e);
    }

    let mut metadata_store_result = MetadataStore::new(&metadata_path);
    let mut lock_retries = 0;
    let max_lock_retries = 20; // 10 seconds total wait
    while metadata_store_result.is_err() && lock_retries < max_lock_retries {
        let err_msg = metadata_store_result.as_ref().err().unwrap().to_string();
        if err_msg.contains("Database already open") || err_msg.contains("Cannot acquire lock") {
            warn!(
                "Metadata database is locked, retrying in 500ms... (attempt {}/{})",
                lock_retries + 1,
                max_lock_retries
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
            metadata_store_result = MetadataStore::new(&metadata_path);
            lock_retries += 1;
        } else {
            break;
        }
    }

    let metadata_store = match metadata_store_result {
        Ok(s) => Arc::new(s),
        Err(e) => {
            // redb returns "Database already open. Cannot acquire lock." when another instance is running.
            let msg = e.to_string();
            if msg.contains("Database already open") || msg.contains("Cannot acquire lock") {
                return Err(anyhow::anyhow!(
                    "Linggen metadata database is still locked after {} seconds.\n\
                     - Another Linggen backend is likely running.\n\
                     - Stop the other backend OR run this instance with a separate data dir via LINGGEN_DATA_DIR.\n\
                     \n\
                     DB path: {}",
                    max_lock_retries / 2,
                    metadata_path.display()
                ));
            }
            return Err(e).with_context(|| {
                format!(
                    "Failed to initialize metadata store at {}",
                    metadata_path.display()
                )
            });
        }
    };

    // Initialize analytics (generates installation_id if needed)
    info!("Initializing analytics...");
    let first_launch = metadata_store
        .get_setting("installation_id")
        .ok()
        .flatten()
        .is_none();
    match analytics::AnalyticsClient::initialize(&metadata_store).await {
        Ok(analytics_client) => {
            info!(
                "Analytics initialized (enabled: {})",
                analytics_client.is_enabled()
            );
            // Track app started in background
            let analytics_client_clone = analytics_client.clone();
            tokio::spawn(async move {
                analytics_client_clone.track_app_started(first_launch).await;
            });
        }
        Err(e) => {
            info!("Failed to initialize analytics (non-fatal): {}", e);
        }
    }

    // Initialize embedding model in background
    info!("Loading embedding model (async)...");
    // Reset initialization state to ensure UI shows initializing
    if let Err(e) = metadata_store.set_setting("model_initialized", "false") {
        tracing::error!("Failed to reset model_initialized: {}", e);
    }
    if let Err(e) = metadata_store.set_setting("init_progress", "Starting...") {
        tracing::error!("Failed to reset init_progress: {}", e);
    }

    let embedding_model = Arc::new(RwLock::new(None));
    let embedding_model_clone = embedding_model.clone();
    let metadata_store_clone = metadata_store.clone();

    tokio::spawn(async move {
        if let Err(e) =
            metadata_store_clone.set_setting("init_progress", "Downloading embedding model...")
        {
            tracing::error!("Failed to set init_progress: {}", e);
        }

        // Load model in blocking task
        let model_result = tokio::task::spawn_blocking(|| EmbeddingModel::new()).await;

        match model_result {
            Ok(Ok(model)) => {
                let mut lock = embedding_model_clone.write().await;
                *lock = Some(model);
                if let Err(e) = metadata_store_clone.set_setting("init_progress", "Ready") {
                    tracing::error!("Failed to set init_progress to Ready: {}", e);
                }
                if let Err(e) = metadata_store_clone.set_setting("model_initialized", "true") {
                    tracing::error!("Failed to set model_initialized to true: {}", e);
                }
                info!("Embedding model loaded successfully");
            }
            Ok(Err(e)) => {
                let error_msg = format!("Failed to load embedding model: {}", e);
                if let Err(e) = metadata_store_clone.set_setting("init_error", &error_msg) {
                    tracing::error!("Failed to set init_error: {}", e);
                }
                if let Err(e) = metadata_store_clone.set_setting("model_initialized", "error") {
                    tracing::error!("Failed to set model_initialized to error: {}", e);
                }
                tracing::error!("{}", error_msg);
            }
            Err(e) => {
                let error_msg = format!("Failed to join embedding model task: {}", e);
                tracing::error!("{}", error_msg);
            }
        }
    });

    let chunker = Arc::new(TextChunker::new());

    info!("Connecting to LanceDB...");
    let lancedb_uri = lancedb_path
        .to_str()
        .expect("Failed to convert lancedb path to string");

    let vector_store = Arc::new(
        VectorStore::new(lancedb_uri)
            .await
            .expect("Failed to initialize vector store"),
    );

    // Initialize internal index store (same DB, different table)
    let internal_index_store = Arc::new(
        storage::InternalIndexStore::new(lancedb_uri)
            .await
            .expect("Failed to initialize internal index store"),
    );

    // Check if LLM is enabled in settings
    let app_settings = metadata_store.get_app_settings().unwrap_or_default();

    if app_settings.llm_enabled {
        // Initialize LLM model using ModelManager
        info!("LLM is enabled, checking model initialization...");

        // Check model status using ModelManager
        let model_manager =
            linggen_llm::ModelManager::new().expect("Failed to create ModelManager");
        let model_status = model_manager
            .get_model_status("qwen3-4b")
            .unwrap_or(linggen_llm::ModelStatus::NotFound);

        match model_status {
            linggen_llm::ModelStatus::Ready => {
                info!("LLM model already initialized and ready, loading into singleton...");
                // Mark as initialized in redb
                let _ = metadata_store.set_setting("model_initialized", "true");

                // Initialize the singleton with the existing model
                // We use default config which will use the downloader logic in lib.rs to find the files
                let config = linggen_llm::LLMConfig::default();

                // Initialize singleton in background
                tokio::spawn(async move {
                    match linggen_llm::LLMSingleton::initialize(config).await {
                        Ok(_) => info!("LLM singleton loaded successfully"),
                        Err(e) => info!("Failed to load LLM singleton: {}", e),
                    }
                });
            }
            linggen_llm::ModelStatus::NotFound | linggen_llm::ModelStatus::Corrupted => {
                if model_status == linggen_llm::ModelStatus::Corrupted {
                    info!("LLM model corrupted, will attempt to use anyway...");
                } else {
                    info!("LLM model not found, will attempt initialization...");
                }

                let metadata_store_clone = metadata_store.clone();
                tokio::spawn(async move {
                    // Clone for the closure
                    let metadata_store_for_progress = metadata_store_clone.clone();

                    // Progress callback that saves to redb
                    let progress_callback = move |msg: &str| {
                        info!("Model init progress: {}", msg);
                        if let Err(e) =
                            metadata_store_for_progress.set_setting("init_progress", msg)
                        {
                            info!("Failed to save progress: {}", e);
                        }
                    };

                    // Use default config to trigger download
                    let config = linggen_llm::LLMConfig::default();

                    // Initialize LLM singleton
                    let config_clone = config.clone();
                    match linggen_llm::LLMSingleton::initialize_with_progress(
                        config_clone,
                        progress_callback,
                    )
                    .await
                    {
                        Ok(_) => {
                            info!("LLM singleton initialized successfully");

                            // Register model in ModelManager
                            if let Ok(model_manager) = linggen_llm::ModelManager::new() {
                                // We use HF cache now, so just register as ready without specific file tracking for now
                                let _ = model_manager.register_model(
                                    "qwen3-4b",
                                    "Qwen3-4B-Instruct-2507",
                                    "main",
                                    std::collections::HashMap::new(),
                                );
                            }

                            // Mark as initialized in redb
                            if let Err(e) =
                                metadata_store_clone.set_setting("model_initialized", "true")
                            {
                                info!("Failed to save model initialization state: {}", e);
                            }
                            if let Err(e) = metadata_store_clone.set_setting("init_progress", "") {
                                info!("Failed to clear progress: {}", e);
                            }
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to initialize LLM model: {}", e);
                            info!("{}", error_msg);
                            if let Err(e) =
                                metadata_store_clone.set_setting("init_error", &error_msg)
                            {
                                info!("Failed to save error: {}", e);
                            }
                            if let Err(e) =
                                metadata_store_clone.set_setting("model_initialized", "error")
                            {
                                info!("Failed to save model error state: {}", e);
                            }
                        }
                    }
                });
            }
            linggen_llm::ModelStatus::Downloading => {
                info!("LLM model is currently downloading...");
            }
        }
    } else {
        info!("LLM is disabled in settings, skipping model initialization");
    }

    // Clean up stale jobs that were pending/running when server stopped.
    //
    // `Pending` jobs are "waiting in queue" (JobManager permit). If the server restarts,
    // they will never run, so leaving them as Pending makes the UI look stuck forever.
    info!("Checking for interrupted jobs...");
    if let Ok(jobs) = metadata_store.get_jobs(None) {
        let interrupted_jobs: Vec<_> = jobs
            .iter()
            .filter(|j| {
                matches!(
                    j.status,
                    linggen_core::JobStatus::Pending | linggen_core::JobStatus::Running
                )
            })
            .collect();

        if !interrupted_jobs.is_empty() {
            info!(
                "Found {} interrupted jobs (Pending/Running) from previous server instance",
                interrupted_jobs.len()
            );
            for job in interrupted_jobs {
                let reason = match job.status {
                    linggen_core::JobStatus::Pending => {
                        "Server was restarted before job started (was waiting in queue)"
                    }
                    linggen_core::JobStatus::Running => {
                        "Server was restarted while job was running"
                    }
                    _ => "Server was restarted",
                };

                let mut failed_job = job.clone();
                failed_job.status = linggen_core::JobStatus::Failed;
                failed_job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                failed_job.error = Some(reason.to_string());
                if let Err(e) = metadata_store.update_job(&failed_job) {
                    info!("Failed to update interrupted job {}: {}", job.id, e);
                } else {
                    info!("Marked job {} as failed (was {:?})", job.id, job.status);
                }
            }
        }
    }

    let job_manager = Arc::new(JobManager::new(1)); // Limit to 1 concurrent job

    // Initialize graph cache for architect feature
    let graph_cache_dir = base_data_dir.join("graph_cache");
    info!("Using graph cache at {:?}", graph_cache_dir);
    let graph_cache = Arc::new(
        linggen_architect::GraphCache::new(&graph_cache_dir)
            .expect("Failed to initialize graph cache"),
    );

    // Memory store:
    // - Prefer a workspace/repo `.linggen/` when running inside a project.
    // - But when launched from Finder / as a sidecar, cwd may be `/` and is not writable.
    //   In that case fall back to the stable app data dir (Application Support/Linggen).
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (linggen_dir, memory_dir) = resolve_memory_dirs(&cwd, &base_data_dir);

    // Best-effort migration: move any nested `.linggen/memory/*.md` into the repo-root memory dir.
    for nested in find_all_linggen_dirs(&cwd) {
        let nested_memory = nested.join("memory");
        if nested_memory.exists() && nested_memory != memory_dir {
            if let Err(e) = migrate_memory_files(&nested_memory, &memory_dir) {
                warn!(
                    "Failed to migrate nested memory files from {:?} to {:?}: {}",
                    nested_memory, memory_dir, e
                );
            }
        }
    }
    info!("Using memory store at {:?}", memory_dir);
    let memory_store = Arc::new(
        crate::memory::MemoryStore::new(memory_dir.clone())
            .expect("Failed to initialize memory store"),
    );

    let (broadcast_tx, _) = tokio::sync::broadcast::channel(100);

    let app_state = Arc::new(AppState {
        embedding_model: embedding_model.clone(),
        chunker: chunker.clone(),
        vector_store: vector_store.clone(),
        internal_index_store: internal_index_store.clone(),
        metadata_store: metadata_store.clone(),
        memory_store: memory_store.clone(),
        cancellation_flags: DashMap::new(),
        job_manager: job_manager.clone(),
        graph_cache: graph_cache.clone(),
        broadcast_tx: broadcast_tx.clone(),
        library_path: library_path.clone(),
    });

    // Start background watchers for all configured projects
    let internal_index_store_clone = internal_index_store.clone();
    let embedding_model_clone = embedding_model.clone();
    let chunker_clone = chunker.clone();
    let broadcast_tx_clone = broadcast_tx.clone();
    let metadata_store_clone = metadata_store.clone();
    let linggen_dir_clone = linggen_dir.clone();

    tokio::spawn(async move {
        // 1. Watch the global .linggen directory
        if let Err(e) = crate::internal_indexer::start_internal_watcher(
            internal_index_store_clone.clone(),
            embedding_model_clone.clone(),
            chunker_clone.clone(),
            broadcast_tx_clone.clone(),
            "global".to_string(),
            linggen_dir_clone,
        )
        .await
        {
            warn!("Failed to start internal file watcher for global: {}", e);
        }

        // 2. Watch all existing local sources
        if let Ok(sources) = metadata_store_clone.get_sources() {
            for source in sources {
                if source.source_type == linggen_core::SourceType::Local {
                    let project_path = std::path::PathBuf::from(&source.path);
                    let linggen_path = project_path.join(".linggen");

                    if linggen_path.exists() {
                        info!(
                            "Starting internal watcher for source '{}' at {:?}",
                            source.name, linggen_path
                        );
                        if let Err(e) = crate::internal_indexer::start_internal_watcher(
                            internal_index_store_clone.clone(),
                            embedding_model_clone.clone(),
                            chunker_clone.clone(),
                            broadcast_tx_clone.clone(),
                            source.id.clone(),
                            linggen_path,
                        )
                        .await
                        {
                            warn!("Failed to start watcher for {}: {}", source.name, e);
                        }
                    }
                }
            }
        }
    });

    // Create MCP state (wraps app_state for MCP handlers)
    let mcp_state = Arc::new(McpState::new());
    let mcp_app_state = Arc::new(McpAppState {
        app: app_state.clone(),
        mcp: mcp_state,
    });

    // Configure CORS - allow any origin for MCP clients
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        // Allow common HTTP methods used by the frontend and MCP
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any);

    // Build our application with routes
    // Upload routes need higher body limit for large files (100MB)
    let upload_routes = Router::new()
        .route("/api/upload", post(upload_file))
        .route("/api/upload/stream", post(upload_file_stream))
        .route("/api/upload/files", post(list_uploaded_files))
        .route("/api/upload/delete", post(delete_uploaded_file))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024)) // 100MB for file uploads
        .with_state(app_state.clone());

    let api_routes = Router::new()
        .route("/api/events", get(crate::handlers::events_handler))
        .route("/api/status", get(get_app_status))
        .route("/api/retry_init", post(retry_init))
        .route("/api/index_source", post(index_source))
        .route("/api/classify", post(classify_intent))
        .route("/api/enhance", post(enhance_prompt))
        // Vector search/query (useful for VS Code extensions)
        .route("/api/query", post(crate::handlers::search::search))
        .route("/api/search", post(crate::handlers::search::search))
        // Memories
        .route(
            "/api/memory/search",
            post(crate::handlers::memory::list_memories),
        )
        .route(
            "/api/memory/search_semantic",
            post(crate::handlers::memory_search_semantic),
        )
        .route(
            "/api/memory/read",
            get(crate::handlers::memory::read_memory),
        )
        .route(
            "/api/memory/create",
            post(crate::handlers::memory::create_memory),
        )
        .route(
            "/api/memory/update",
            post(crate::handlers::memory::update_memory),
        )
        .route(
            "/api/memory/delete",
            post(crate::handlers::memory::delete_memory),
        )
        .route("/api/chat/stream", post(chat_stream))
        .route("/api/jobs", get(list_jobs))
        .route("/api/jobs/cancel", post(cancel_job))
        .route("/api/resources", post(add_resource))
        .route("/api/resources", get(list_resources))
        .route("/api/resources/remove", post(remove_resource))
        .route("/api/resources/rename", post(rename_resource))
        .route("/api/resources/patterns", post(update_resource_patterns))
        .route("/api/library/packs", get(list_packs).post(create_pack))
        .route("/api/library/packs/rename", post(rename_pack))
        .route(
            "/api/library/packs/:pack_id",
            get(get_pack).put(save_pack).delete(delete_pack),
        )
        .route("/api/library/packs/:pack_id/apply", post(apply_pack))
        .route(
            "/api/library/folders",
            get(list_folders).post(create_folder),
        )
        .route("/api/library/folders/rename", post(rename_folder))
        .route("/api/library/folders/:folder_name", delete(delete_folder))
        .route(
            "/api/settings",
            get(crate::handlers::settings::get_settings)
                .put(crate::handlers::settings::update_settings),
        )
        .route("/api/clear_all_data", post(clear_all_data))
        .route(
            "/api/sources/:source_id/profile",
            get(crate::handlers::profile::get_profile)
                .put(crate::handlers::profile::update_profile),
        )
        .route(
            "/api/sources/:source_id/profile/generate",
            post(crate::handlers::profile::generate_profile),
        )
        // Graph (Architect) routes
        .route("/api/sources/:source_id/graph", get(get_graph))
        .route(
            "/api/sources/:source_id/graph/focus",
            get(crate::handlers::graph::get_graph_focus),
        )
        .route(
            "/api/sources/:source_id/graph/status",
            get(get_graph_status),
        )
        .route(
            "/api/sources/:source_id/graph/with_status",
            get(get_graph_with_status),
        )
        .route("/api/sources/:source_id/graph/rebuild", post(rebuild_graph))
        // Design Notes routes
        .route(
            "/api/sources/:source_id/notes",
            get(crate::handlers::notes::list_notes),
        )
        .route(
            "/api/sources/:source_id/notes/*note_path",
            get(crate::handlers::notes::get_note)
                .put(crate::handlers::notes::save_note)
                .delete(crate::handlers::notes::delete_note),
        )
        .route(
            "/api/sources/:source_id/notes/rename",
            post(crate::handlers::notes::rename_note),
        )
        // Source Memories (markdown files under source/.linggen/memory)
        .route(
            "/api/sources/:source_id/memory",
            get(crate::handlers::list_memory_files),
        )
        .route(
            "/api/sources/:source_id/memory/*file_path",
            get(crate::handlers::get_memory_file)
                .put(crate::handlers::save_memory_file)
                .delete(crate::handlers::delete_memory_file),
        )
        .route(
            "/api/sources/:source_id/memory/rename",
            post(crate::handlers::rename_memory_file),
        )
        // Source Prompt Templates (markdown files under source/.linggen/prompts)
        .route(
            "/api/sources/:source_id/prompts",
            get(crate::handlers::list_prompts),
        )
        .route(
            "/api/sources/:source_id/prompts/*file_path",
            get(crate::handlers::get_prompt)
                .put(crate::handlers::save_prompt)
                .delete(crate::handlers::delete_prompt),
        )
        .route(
            "/api/sources/:source_id/prompts/rename",
            post(crate::handlers::rename_prompt),
        )
        .route(
            "/api/sources/:source_id/internal/rescan",
            post(crate::handlers::rescan_internal_index),
        )
        .with_state(app_state);

    // MCP routes (for Cursor integration)
    let mcp_routes = Router::new()
        .route("/mcp/sse", get(mcp_sse_handler).post(mcp_message_handler))
        .route("/mcp/message", post(mcp_message_handler))
        .route("/mcp/health", get(mcp_health_handler))
        .with_state(mcp_app_state);

    // Combine API and MCP routes
    let combined_routes = api_routes
        .merge(upload_routes)
        .merge(mcp_routes)
        .layer(cors)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024)); // 10MB default for other routes

    // Serve static frontend files (if they exist).
    // IMPORTANT: Finder/launched apps often have a surprising cwd, so prefer resolving relative to
    // the executable location (e.g. Linggen.app/Contents/MacOS -> ../Resources/frontend).
    let frontend_dir: Option<PathBuf> = find_frontend_dir();

    let app = if let Some(frontend_dir) = frontend_dir {
        info!("Serving frontend from: {:?}", frontend_dir);
        // SPA fallback: unknown paths should return index.html (so refresh/deep links work).
        let index = frontend_dir.join("index.html");
        combined_routes
            .fallback_service(ServeDir::new(frontend_dir).not_found_service(ServeFile::new(index)))
    } else {
        info!("Frontend assets not found, serving API only");
        combined_routes.route("/", get(root_handler))
    };

    // Run it
    // Bind to 0.0.0.0 to allow both local and remote access
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("listening on {}", addr);
    info!("Remote access available at http://<your-ip>:{}", port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root_handler() -> &'static str {
    "Hello from Linggen Backend!"
}

fn is_writable_dir(path: &Path) -> bool {
    // We consider a directory "writable" if we can create it (if missing) and create a temp file.
    if std::fs::create_dir_all(path).is_err() {
        return false;
    }
    let probe = path.join(".linggen_write_probe");
    match std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn resolve_memory_dirs(cwd: &Path, base_data_dir: &Path) -> (PathBuf, PathBuf) {
    // 1. Check for an explicit workspace/repo '.linggen' directory first.
    let candidate_linggen_dir = find_workspace_linggen_dir(cwd);
    let candidate_linggen_dir =
        std::fs::canonicalize(&candidate_linggen_dir).unwrap_or(candidate_linggen_dir);

    let home = dirs::home_dir().and_then(|h| std::fs::canonicalize(h).ok());

    // Only use the workspace candidate if:
    // - It has a parent (not root)
    // - It's writable
    // - It is NOT exactly $HOME/.linggen (we want to use Application Support for global data)
    let is_home_linggen = home
        .map(|h| candidate_linggen_dir == h.join(".linggen"))
        .unwrap_or(false);

    if candidate_linggen_dir.parent().is_some()
        && is_writable_dir(&candidate_linggen_dir)
        && !is_home_linggen
    {
        let memory_dir = candidate_linggen_dir.join("memory");
        return (candidate_linggen_dir, memory_dir);
    }

    // 2. Fallback: Use the stable app data directory (~/Library/Application Support/Linggen/.linggen)
    let fallback_linggen_dir = base_data_dir.join(".linggen");
    let fallback_memory_dir = fallback_linggen_dir.join("memory");

    (fallback_linggen_dir, fallback_memory_dir)
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start;
    loop {
        if cur.join(".git").is_dir() {
            return Some(cur.to_path_buf());
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return None,
        }
    }
}

/// Find the workspace `.linggen/` directory to use for memory.
///
/// Priority:
/// 1) Git repo root (if any): `<git_root>/.linggen` (created if missing)
/// 2) Nearest `.linggen` under `start` ancestry, but **do not** pick `$HOME/.linggen` as the project store.
fn find_workspace_linggen_dir(start: &Path) -> PathBuf {
    if let Some(git_root) = find_git_root(start) {
        return git_root.join(".linggen");
    }

    // Fall back to nearest `.linggen` but stop before reaching HOME.
    let home = dirs::home_dir();
    let mut cur = start;
    loop {
        let candidate = cur.join(".linggen");
        if candidate.is_dir() {
            return candidate;
        }
        match cur.parent() {
            Some(parent) => {
                if let Some(ref h) = home {
                    if parent == h.as_path() {
                        break;
                    }
                }
                cur = parent;
            }
            None => break,
        }
    }

    // Default: `.linggen` in current working directory
    start.join(".linggen")
}

fn find_all_linggen_dirs(start: &Path) -> Vec<PathBuf> {
    let mut cur = start;
    let mut dirs: Vec<PathBuf> = Vec::new();
    // Stop at git root if present (to avoid pulling in $HOME/.linggen)
    let stop_at = find_git_root(start);
    loop {
        let candidate = cur.join(".linggen");
        if candidate.is_dir() {
            dirs.push(candidate);
        }
        if let Some(ref stop) = stop_at {
            if cur == stop.as_path() {
                break;
            }
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => break,
        }
    }
    dirs
}

fn migrate_memory_files(from_dir: &Path, to_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to_dir)?;
    for entry in std::fs::read_dir(from_dir)? {
        let entry = entry?;
        let src = entry.path();
        if src.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let file_name = match src.file_name() {
            Some(f) => f,
            None => continue,
        };
        let mut dst = to_dir.join(file_name);
        if dst.exists() {
            // Avoid overwriting: prefix with "migrated_"
            dst = to_dir.join(format!("migrated_{}", file_name.to_string_lossy()));
        }
        std::fs::rename(&src, &dst)?;
    }
    // Try to clean up empty legacy dirs (best effort).
    let _ = std::fs::remove_dir(from_dir);
    if let Some(parent) = from_dir.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

fn find_frontend_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 0) Explicit override (useful for debugging / custom packaging).
    if let Ok(dir) = std::env::var("LINGGEN_FRONTEND_DIR") {
        candidates.push(PathBuf::from(dir));
    }

    // 0.5) Tauri provides the resources directory at runtime in some environments.
    // Prefer it if available.
    if let Ok(dir) = std::env::var("TAURI_RESOURCE_DIR") {
        let base = PathBuf::from(dir);
        candidates.push(base.join("frontend"));
        candidates.push(base.join("resources/frontend"));
    }

    // 1) Resolve relative to the server executable (best for release apps / Finder launches).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("../Resources/frontend"));
            // Tauri bundles `bundle.resources` under `Contents/Resources/resources/...` by default.
            candidates.push(exe_dir.join("../Resources/resources/frontend"));
            candidates.push(exe_dir.join("../../Resources/frontend")); // extra safety for odd layouts
            candidates.push(exe_dir.join("../../Resources/resources/frontend"));
        }
    }

    // 2) Resolve relative to this crate (useful for local dev builds).
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../frontend/dist"));
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../frontend/dist"));

    // 3) Conventional Linux locations (for tarball/installer use).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            // If binary is in /usr/local/bin, look for /usr/local/share/linggen/frontend
            candidates.push(exe_dir.join("../share/linggen/frontend"));
        }
    }
    candidates.push(PathBuf::from("/usr/local/share/linggen/frontend"));
    candidates.push(PathBuf::from("./frontend"));

    // 4) Legacy relative paths (kept for compatibility).
    candidates.push(PathBuf::from("../Resources/frontend"));
    candidates.push(PathBuf::from("./frontend"));
    candidates.push(PathBuf::from("../frontend/dist"));

    candidates.into_iter().find(|p| p.exists())
}
