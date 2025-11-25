use axum::{routing::get, routing::post, Router};
use dashmap::DashMap;
use embeddings::{EmbeddingModel, TextChunker};
use std::net::SocketAddr;
use std::{path::PathBuf, sync::Arc};
use storage::{MetadataStore, VectorStore};
use tower_http::services::ServeDir;
use tracing::info;

mod handlers;
use handlers::{
    add_resource, cancel_job, classify_intent, enhance_prompt, get_app_status, get_preferences,
    index_source, list_jobs, list_resources, remove_resource, retry_init, update_preferences,
    AppState,
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

    info!("RememberMe Backend API starting...");

    // Initialize embedding model and vector store
    info!("Loading embedding model...");
    let embedding_model = Arc::new(EmbeddingModel::new().expect("Failed to load embedding model"));

    let chunker = Arc::new(TextChunker::new());

    info!("Connecting to LanceDB...");
    let vector_store = Arc::new(
        VectorStore::new("./data/lancedb")
            .await
            .expect("Failed to initialize vector store"),
    );

    info!("Initializing metadata store...");
    let metadata_store = Arc::new(
        MetadataStore::new("./data/metadata.redb").expect("Failed to initialize metadata store"),
    );

    // Initialize LLM model using ModelManager
    info!("Checking LLM model initialization...");

    // Check model status using ModelManager
    let model_manager = rememberme_llm::ModelManager::new().expect("Failed to create ModelManager");
    let model_status = model_manager
        .get_model_status("qwen3-4b")
        .unwrap_or(rememberme_llm::ModelStatus::NotFound);

    match model_status {
        rememberme_llm::ModelStatus::Ready => {
            info!("LLM model already initialized and ready, loading into singleton...");
            // Mark as initialized in redb
            let _ = metadata_store.set_setting("model_initialized", "true");

            // Initialize the singleton with the existing model
            // We use default config which will use the downloader logic in lib.rs to find the files
            let config = rememberme_llm::LLMConfig::default();

            // Initialize singleton in background
            tokio::spawn(async move {
                match rememberme_llm::LLMSingleton::initialize(config).await {
                    Ok(_) => info!("LLM singleton loaded successfully"),
                    Err(e) => info!("Failed to load LLM singleton: {}", e),
                }
            });
        }
        rememberme_llm::ModelStatus::NotFound | rememberme_llm::ModelStatus::Corrupted => {
            if model_status == rememberme_llm::ModelStatus::Corrupted {
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
                    if let Err(e) = metadata_store_for_progress.set_setting("init_progress", msg) {
                        info!("Failed to save progress: {}", e);
                    }
                };

                // Use default config to trigger download
                let config = rememberme_llm::LLMConfig::default();

                // Initialize LLM singleton
                let config_clone = config.clone();
                match rememberme_llm::LLMSingleton::initialize_with_progress(
                    config_clone,
                    progress_callback,
                )
                .await
                {
                    Ok(_) => {
                        info!("LLM singleton initialized successfully");

                        // Register model in ModelManager
                        if let Ok(model_manager) = rememberme_llm::ModelManager::new() {
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
                        if let Err(e) = metadata_store_clone.set_setting("init_error", &error_msg) {
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
        rememberme_llm::ModelStatus::Downloading => {
            info!("LLM model is currently downloading...");
        }
    }

    // Clean up stale jobs that were running when server stopped
    info!("Checking for interrupted jobs...");
    if let Ok(jobs) = metadata_store.get_jobs(None) {
        let running_jobs: Vec<_> = jobs
            .iter()
            .filter(|j| matches!(j.status, rememberme_core::JobStatus::Running))
            .collect();
        if !running_jobs.is_empty() {
            info!(
                "Found {} running jobs that were interrupted by server restart",
                running_jobs.len()
            );
            for job in running_jobs {
                let mut failed_job = job.clone();
                failed_job.status = rememberme_core::JobStatus::Failed;
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

    // Configure CORS
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(
            "http://localhost:5173"
                .parse::<axum::http::HeaderValue>()
                .unwrap(),
        )
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    // Build our application with routes
    let api_routes = Router::new()
        .route("/api/status", get(get_app_status))
        .route("/api/retry_init", post(retry_init))
        .route("/api/index_source", post(index_source))
        .route("/api/classify", post(classify_intent))
        .route("/api/enhance", post(enhance_prompt))
        .route("/api/jobs", get(list_jobs))
        .route("/api/jobs/cancel", post(cancel_job))
        .route("/api/resources", post(add_resource))
        .route("/api/resources", get(list_resources))
        .route("/api/resources/remove", post(remove_resource))
        .route(
            "/api/preferences",
            get(handlers::preferences::get_preferences)
                .put(handlers::preferences::update_preferences),
        )
        .route(
            "/api/sources/:source_id/profile",
            get(handlers::profile::get_profile).put(handlers::profile::update_profile),
        )
        .route(
            "/api/sources/:source_id/profile/generate",
            post(handlers::profile::generate_profile),
        )
        .with_state(app_state)
        .layer(cors);

    // Serve static frontend files (if they exist)
    //
    // Supported layouts:
    // - Development / in-repo build:   ../frontend/dist
    // - Packaged build-release bundle: ./frontend
    let frontend_dir: Option<PathBuf> = ["./frontend", "../frontend/dist"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists());

    let app = if let Some(frontend_dir) = frontend_dir {
        info!("Serving frontend from: {:?}", frontend_dir);
        api_routes.fallback_service(ServeDir::new(frontend_dir))
    } else {
        info!("Frontend assets not found, serving API only");
        api_routes.route("/", get(handler))
    };

    // Run it
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handler() -> &'static str {
    "Hello from RememberMe Backend!"
}
