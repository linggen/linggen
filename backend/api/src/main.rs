use axum::{extract::DefaultBodyLimit, routing::get, routing::post, Router};
use dashmap::DashMap;
use dirs::data_dir;
use embeddings::{EmbeddingModel, TextChunker};
use std::net::SocketAddr;
use std::{path::PathBuf, sync::Arc};
use storage::{MetadataStore, VectorStore};
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tracing::info;

mod handlers;
use handlers::{
    add_resource, cancel_job, chat_stream, classify_intent, clear_all_data, delete_uploaded_file,
    enhance_prompt, get_app_status, index_source, list_jobs, list_resources, list_uploaded_files,
    mcp::{mcp_health_handler, mcp_message_handler, mcp_sse_handler, McpAppState, McpState},
    remove_resource, rename_resource, retry_init, update_resource_patterns, upload_file, AppState,
};
mod job_manager;
use job_manager::JobManager;

#[tokio::main]
async fn main() {
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

    info!("Linggen Backend API starting...");

    // Initialize embedding model and vector store
    // Use a stable application data directory so the app works
    // regardless of where it's launched from (Finder, CLI, DMG-installed, etc.)
    info!("Initializing metadata store...");

    let base_data_dir = data_dir()
        .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current dir"))
        .join("Linggen");

    let metadata_path = base_data_dir.join("metadata.redb");
    let lancedb_path = base_data_dir.join("lancedb");

    info!("Using metadata store at {:?}", metadata_path);
    info!("Using LanceDB store at {:?}", lancedb_path);

    let metadata_store =
        Arc::new(MetadataStore::new(&metadata_path).expect("Failed to initialize metadata store"));

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

    // Clean up stale jobs that were running when server stopped
    info!("Checking for interrupted jobs...");
    if let Ok(jobs) = metadata_store.get_jobs(None) {
        let running_jobs: Vec<_> = jobs
            .iter()
            .filter(|j| matches!(j.status, linggen_core::JobStatus::Running))
            .collect();
        if !running_jobs.is_empty() {
            info!(
                "Found {} running jobs that were interrupted by server restart",
                running_jobs.len()
            );
            for job in running_jobs {
                let mut failed_job = job.clone();
                failed_job.status = linggen_core::JobStatus::Failed;
                failed_job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                failed_job.error = Some("Server was restarted".to_string());
                if let Err(e) = metadata_store.update_job(&failed_job) {
                    info!("Failed to update interrupted job {}: {}", job.id, e);
                } else {
                    info!("Marked job {} as failed (was interrupted)", job.id);
                }
            }
        }
    }

    let job_manager = Arc::new(JobManager::new(1)); // Limit to 1 concurrent job

    let app_state = Arc::new(AppState {
        embedding_model,
        chunker,
        vector_store,
        metadata_store,
        cancellation_flags: DashMap::new(),
        job_manager,
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
            axum::http::Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any);

    // Build our application with routes
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
        .route("/api/upload", post(upload_file))
        .route("/api/upload/files", post(list_uploaded_files))
        .route("/api/upload/delete", post(delete_uploaded_file))
        .route(
            "/api/settings",
            get(handlers::settings::get_settings).put(handlers::settings::update_settings),
        )
        .route("/api/clear_all_data", post(clear_all_data))
        // .route(
        //     "/api/preferences",
        //     get(handlers::preferences::get_preferences)
        //         .put(handlers::preferences::update_preferences),
        // )
        .route(
            "/api/sources/:source_id/profile",
            get(handlers::profile::get_profile).put(handlers::profile::update_profile),
        )
        .route(
            "/api/sources/:source_id/profile/generate",
            post(handlers::profile::generate_profile),
        )
        .with_state(app_state);

    // MCP routes (for Cursor integration)
    // Note: /mcp/sse accepts both GET (SSE stream) and POST (messages) per MCP streamable HTTP spec
    let mcp_routes = Router::new()
        .route("/mcp/sse", get(mcp_sse_handler).post(mcp_message_handler))
        .route("/mcp/message", post(mcp_message_handler))
        .route("/mcp/health", get(mcp_health_handler))
        .with_state(mcp_app_state);

    // Combine API and MCP routes
    // Increase body limit to 100MB for large file uploads (default is 2MB)
    let combined_routes = api_routes
        .merge(mcp_routes)
        .layer(cors)
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024));

    // Serve static frontend files (if they exist)
    //
    // Supported layouts:
    // - macOS .app bundle:             ../Resources/frontend (relative to MacOS/ dir where binary runs)
    // - Development / in-repo build:   ../frontend/dist
    // - Packaged build-release bundle: ./frontend
    let frontend_dir: Option<PathBuf> = ["../Resources/frontend", "./frontend", "../frontend/dist"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists());

    let app = if let Some(frontend_dir) = frontend_dir {
        info!("Serving frontend from: {:?}", frontend_dir);
        combined_routes.fallback_service(ServeDir::new(frontend_dir))
    } else {
        info!("Frontend assets not found, serving API only");
        combined_routes.route("/", get(handler))
    };

    // Run it
    // Bind to 0.0.0.0 to allow both local and remote access
    // Remote users can access the UI at http://<linggen-ip>:8787
    let addr = SocketAddr::from(([0, 0, 0, 0], 8787));
    info!("listening on {}", addr);
    info!("Remote access available at http://<your-ip>:8787");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handler() -> &'static str {
    "Hello from Linggen Backend!"
}
