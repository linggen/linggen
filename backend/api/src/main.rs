use axum::{
    routing::{get, post},
    Router,
};
use embeddings::{EmbeddingModel, TextChunker};
use std::net::SocketAddr;
use std::sync::Arc;
use storage::VectorStore;
use tracing::info;

mod handlers;
use handlers::{index_document, search, AppState};

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt::init();

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

    let app_state = Arc::new(AppState {
        embedding_model,
        chunker,
        vector_store,
    });

    // Configure CORS
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(
            "http://localhost:5173"
                .parse::<axum::http::HeaderValue>()
                .unwrap(),
        )
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST]);

    // Build our application with routes
    let app = Router::new()
        .route("/", get(handler))
        .route("/api/index", post(index_document))
        .route("/api/search", get(search))
        .with_state(app_state)
        .layer(cors);

    // Run it
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handler() -> &'static str {
    "Hello from RememberMe Backend!"
}
