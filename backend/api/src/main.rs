use axum::{routing::get, routing::post, Router};
use embeddings::{EmbeddingModel, TextChunker};
use std::net::SocketAddr;
use std::{path::PathBuf, sync::Arc};
use storage::{MetadataStore, VectorStore};
use tower_http::services::ServeDir;
use tracing::info;

mod handlers;
use handlers::{
    add_resource, index_document, index_folder, index_source, list_jobs, list_resources,
    remove_resource, search, AppState,
};

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

    let app_state = Arc::new(AppState {
        embedding_model,
        chunker,
        vector_store,
        metadata_store,
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
        .route("/api/index", post(index_document))
        .route("/api/index_folder", post(index_folder))
        .route("/api/index_source", post(index_source))
        .route("/api/jobs", get(list_jobs))
        .route("/api/search", get(search))
        .route("/api/resources", post(add_resource))
        .route("/api/resources", get(list_resources))
        .route("/api/resources/remove", post(remove_resource))
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
