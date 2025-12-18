//! Graph API handlers for the Architect feature

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use linggen_architect::{CacheStatus, ProjectGraph};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::index::AppState;

/// Response for graph status endpoint
#[derive(Serialize)]
pub struct GraphStatusResponse {
    pub status: String,
    pub node_count: Option<usize>,
    pub edge_count: Option<usize>,
    pub built_at: Option<String>,
}

/// Response for getting a graph
#[derive(Serialize)]
pub struct GraphResponse {
    pub project_id: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub built_at: Option<String>,
}

/// Combined response with status and graph data (optimized for single request)
#[derive(Serialize)]
pub struct GraphWithStatusResponse {
    pub status: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub built_at: Option<String>,
    pub project_id: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Node in the graph response (simplified for frontend)
#[derive(Serialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub language: String,
    pub folder: String,
}

/// Edge in the graph response (simplified for frontend)
#[derive(Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub kind: String,
}

/// Query parameters for graph endpoint
#[derive(Deserialize, Default)]
pub struct GraphQuery {
    /// Filter by folder prefix
    pub folder: Option<String>,
    /// Center on a specific node and return only its neighborhood
    pub focus: Option<String>,
    /// Number of hops for neighborhood (default 1)
    pub hops: Option<usize>,
}

/// Query parameters for focused graph endpoint (alias for focus/hops).
#[derive(Deserialize, Default)]
pub struct GraphFocusQuery {
    /// Focus node id (relative file path from project root)
    pub file_path: String,
    /// Number of hops (default 1)
    pub hops: Option<usize>,
}

impl From<&ProjectGraph> for GraphResponse {
    fn from(graph: &ProjectGraph) -> Self {
        GraphResponse {
            project_id: graph.project_id.clone(),
            nodes: graph
                .nodes
                .iter()
                .map(|n| GraphNode {
                    id: n.id.clone(),
                    label: n.label.clone(),
                    language: n.language.to_string(),
                    folder: n.folder.clone(),
                })
                .collect(),
            edges: graph
                .edges
                .iter()
                .map(|e| GraphEdge {
                    source: e.source.clone(),
                    target: e.target.clone(),
                    kind: e.kind.to_string(),
                })
                .collect(),
            built_at: graph.built_at.clone(),
        }
    }
}

/// Get the status of a project's graph
///
/// GET /api/sources/:source_id/graph/status
pub async fn get_graph_status(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> Result<Json<GraphStatusResponse>, (StatusCode, String)> {
    // Get source config to find the path
    let source = state
        .metadata_store
        .get_source(&source_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("Source not found: {}", source_id),
        ))?;

    let status = state.graph_cache.status(&source.path);

    let (status_str, node_count, edge_count, built_at) = match status {
        CacheStatus::Fresh | CacheStatus::Stale => {
            // Try to load metadata
            match state.graph_cache.load_metadata(&source.path) {
                Ok(Some(meta)) => (
                    if matches!(status, CacheStatus::Fresh) {
                        "ready"
                    } else {
                        "stale"
                    },
                    Some(meta.node_count),
                    Some(meta.edge_count),
                    Some(meta.created_at),
                ),
                Ok(None) => ("missing", None, None, None),
                Err(_) => ("error", None, None, None),
            }
        }
        CacheStatus::Missing => ("missing", None, None, None),
        CacheStatus::Building => ("building", None, None, None),
        CacheStatus::Error(e) => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e));
        }
    };

    Ok(Json(GraphStatusResponse {
        status: status_str.to_string(),
        node_count,
        edge_count,
        built_at,
    }))
}

/// Get the file dependency graph for a source
///
/// GET /api/sources/:source_id/graph
pub async fn get_graph(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    Query(query): Query<GraphQuery>,
) -> Result<Json<GraphResponse>, (StatusCode, String)> {
    // Get source config to find the path
    let source = state
        .metadata_store
        .get_source(&source_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("Source not found: {}", source_id),
        ))?;

    // Load graph from cache
    let graph = state
        .graph_cache
        .load(&source.path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Graph not found. Please wait for indexing to complete.".to_string(),
        ))?;

    // Apply filters
    let filtered_graph = if let Some(focus_node) = &query.focus {
        // Return neighborhood around the focus node
        let hops = query.hops.unwrap_or(1);
        graph.get_neighborhood(focus_node, hops)
    } else if let Some(folder) = &query.folder {
        // Filter by folder
        graph.filter_by_folder(folder)
    } else {
        // Return full graph
        graph
    };

    Ok(Json(GraphResponse::from(&filtered_graph)))
}

/// Focused neighborhood graph (convenience endpoint for extensions)
///
/// GET /api/sources/:source_id/graph/focus?file_path=src/lib.rs&hops=2
pub async fn get_graph_focus(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    Query(query): Query<GraphFocusQuery>,
) -> Result<Json<GraphResponse>, (StatusCode, String)> {
    let focus = query.file_path;
    let hops = query.hops.unwrap_or(1);
    get_graph(
        State(state),
        Path(source_id),
        Query(GraphQuery {
            focus: Some(focus),
            hops: Some(hops),
            folder: None,
        }),
    )
    .await
}

/// Get graph with status in a single request (optimized endpoint)
///
/// GET /api/sources/:source_id/graph/with_status?focus=lib.rs&hops=2
///
/// This endpoint combines status and graph data to reduce round trips.
/// Supports same query parameters as /graph endpoint:
/// - focus: Return only nodes connected to this node
/// - hops: Number of hops from focus node (default: 1)
/// - folder: Filter by folder prefix
pub async fn get_graph_with_status(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    Query(query): Query<GraphQuery>,
) -> Result<Json<GraphWithStatusResponse>, (StatusCode, String)> {
    // Get source config to find the path
    let source = state
        .metadata_store
        .get_source(&source_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("Source not found: {}", source_id),
        ))?;

    // Check cache status
    let cache_status = state.graph_cache.status(&source.path);

    match cache_status {
        CacheStatus::Missing => {
            return Err((
                StatusCode::NOT_FOUND,
                "Graph not built yet. Please wait for indexing.".to_string(),
            ))
        }
        CacheStatus::Building => {
            return Err((
                StatusCode::ACCEPTED,
                "Graph is currently building.".to_string(),
            ))
        }
        CacheStatus::Error(e) => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e));
        }
        CacheStatus::Fresh | CacheStatus::Stale => {
            // Load graph from cache
            let graph = state
                .graph_cache
                .load(&source.path)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                .ok_or((
                    StatusCode::NOT_FOUND,
                    "Graph not found in cache.".to_string(),
                ))?;

            // Apply filters
            let filtered_graph = if let Some(focus_node) = &query.focus {
                // Return neighborhood around the focus node
                let hops = query.hops.unwrap_or(1);
                graph.get_neighborhood(focus_node, hops)
            } else if let Some(folder) = &query.folder {
                // Filter by folder
                graph.filter_by_folder(folder)
            } else {
                // Return full graph
                graph
            };

            // Get metadata for built_at timestamp
            let built_at = state
                .graph_cache
                .load_metadata(&source.path)
                .ok()
                .flatten()
                .map(|meta| meta.created_at);

            let response = GraphWithStatusResponse {
                status: if matches!(cache_status, CacheStatus::Stale) {
                    "stale".to_string()
                } else {
                    "ready".to_string()
                },
                node_count: filtered_graph.nodes.len(),
                edge_count: filtered_graph.edges.len(),
                built_at,
                project_id: filtered_graph.project_id.clone(),
                nodes: filtered_graph
                    .nodes
                    .iter()
                    .map(|n| GraphNode {
                        id: n.id.clone(),
                        label: n.label.clone(),
                        language: n.language.to_string(),
                        folder: n.folder.clone(),
                    })
                    .collect(),
                edges: filtered_graph
                    .edges
                    .iter()
                    .map(|e| GraphEdge {
                        source: e.source.clone(),
                        target: e.target.clone(),
                        kind: e.kind.to_string(),
                    })
                    .collect(),
            };

            Ok(Json(response))
        }
    }
}

/// Rebuild the graph for a source (force refresh)
///
/// POST /api/sources/:source_id/graph/rebuild
pub async fn rebuild_graph(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> Result<Json<GraphStatusResponse>, (StatusCode, String)> {
    // Get source config
    let source = state
        .metadata_store
        .get_source(&source_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("Source not found: {}", source_id),
        ))?;

    let source_path = source.path.clone();
    let graph_cache = state.graph_cache.clone();

    // Spawn background task to rebuild
    tokio::spawn(async move {
        tracing::info!(
            "🏗️  Rebuilding file dependency graph for {}...",
            source_path
        );

        let source_path_clone = source_path.clone();
        let graph_result = tokio::task::spawn_blocking(move || {
            let project_path = std::path::Path::new(&source_path_clone);
            linggen_architect::build_project_graph(project_path)
        })
        .await;

        match graph_result {
            Ok(Ok(mut graph)) => {
                graph.set_built_at(chrono::Utc::now().to_rfc3339());
                if let Err(e) = graph_cache.save(&graph) {
                    tracing::error!("Failed to save rebuilt graph: {}", e);
                } else {
                    tracing::info!(
                        "✅ Graph rebuilt ({} nodes, {} edges)",
                        graph.node_count(),
                        graph.edge_count()
                    );
                }
            }
            Ok(Err(e)) => {
                tracing::error!("Failed to rebuild graph: {}", e);
            }
            Err(e) => {
                tracing::error!("Graph rebuild task panicked: {}", e);
            }
        }
    });

    Ok(Json(GraphStatusResponse {
        status: "building".to_string(),
        node_count: None,
        edge_count: None,
        built_at: None,
    }))
}
