use anyhow::Context;
use axum::{extract::DefaultBodyLimit, routing::get, routing::post, Router};
use dashmap::DashMap;
use dirs::data_dir;
use embeddings::{EmbeddingModel, TextChunker};
use std::net::SocketAddr;
use std::time::Duration;
use std::{path::PathBuf, sync::Arc};
use storage::{MetadataStore, VectorStore};
use tokio::sync::RwLock;
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;

use crate::analytics;
use crate::handlers::{
    add_resource, cancel_job, chat_stream, classify_intent, clear_all_data, delete_uploaded_file,
    enhance_prompt, get_app_status, get_graph, get_graph_status, get_graph_with_status,
    index_source, list_jobs, list_resources, list_uploaded_files,
    mcp::{mcp_health_handler, mcp_message_handler, mcp_sse_handler, McpAppState, McpState},
    rebuild_graph, remove_resource, rename_resource, retry_init, update_resource_patterns,
    upload_file, upload_file_stream, AppState,
};
use crate::job_manager::JobManager;

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

#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> bool {
    true
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
            }
        });
    }

    // Initialize embedding model and vector store
    // Use a stable application data directory so the app works
    // regardless of where it's launched from (Finder, CLI, DMG-installed, etc.)
    info!("Initializing metadata store...");

    // Allow overriding the data directory to run multiple instances (e.g. dev server alongside app).
    // Example:
    //   LINGGEN_DATA_DIR="$HOME/Library/Application Support/Linggen-dev" linggen-server --port 8788
    let base_data_dir = if let Ok(dir) = std::env::var("LINGGEN_DATA_DIR") {
        PathBuf::from(dir)
    } else {
        data_dir()
            .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current dir"))
            .join("Linggen")
    };

    let metadata_path = base_data_dir.join("metadata.redb");
    let lancedb_path = base_data_dir.join("lancedb");

    info!("Using metadata store at {:?}", metadata_path);
    info!("Using LanceDB store at {:?}", lancedb_path);

    let metadata_store = match MetadataStore::new(&metadata_path) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            // redb returns "Database already open. Cannot acquire lock." when another instance is running.
            let msg = e.to_string();
            if msg.contains("Database already open") || msg.contains("Cannot acquire lock") {
                return Err(anyhow::anyhow!(
                    "Linggen metadata database is already in use.\n\
                     - Another Linggen backend is likely running (e.g. Linggen.app sidecar).\n\
                     - Stop the other backend OR run this instance with a separate data dir via LINGGEN_DATA_DIR.\n\
                     - Example: LINGGEN_DATA_DIR=\"$HOME/Library/Application Support/Linggen-dev\" linggen-server --port 8788\n\
                     \n\
                     DB path: {}",
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
    let vector_store = Arc::new(
        VectorStore::new(
            lancedb_path
                .to_str()
                .expect("Failed to convert lancedb path to string"),
        )
        .await
        .expect("Failed to initialize vector store"),
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

    let app_state = Arc::new(AppState {
        embedding_model,
        chunker,
        vector_store,
        metadata_store,
        cancellation_flags: DashMap::new(),
        job_manager,
        graph_cache,
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
        .route("/api/status", get(get_app_status))
        .route("/api/retry_init", post(retry_init))
        .route("/api/index_source", post(index_source))
        .route("/api/classify", post(classify_intent))
        .route("/api/enhance", post(enhance_prompt))
        .route("/api/chat/stream", post(chat_stream))
        .route("/api/jobs", get(list_jobs))
        .route("/api/jobs/cancel", post(cancel_job))
        .route("/api/resources", post(add_resource))
        .route("/api/resources", get(list_resources))
        .route("/api/resources/remove", post(remove_resource))
        .route("/api/resources/rename", post(rename_resource))
        .route("/api/resources/patterns", post(update_resource_patterns))
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

    // 3) Legacy relative paths (kept for compatibility).
    candidates.push(PathBuf::from("../Resources/frontend"));
    candidates.push(PathBuf::from("./frontend"));
    candidates.push(PathBuf::from("../frontend/dist"));

    candidates.into_iter().find(|p| p.exists())
}
