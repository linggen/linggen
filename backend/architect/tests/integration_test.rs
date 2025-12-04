//! Integration tests for the architect crate

use linggen_architect::{build_project_graph, GraphCache};
use std::fs;
use tempfile::TempDir;

/// Creates a minimal Rust project for testing
fn create_test_rust_project() -> TempDir {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(src.join("handlers")).unwrap();

    // Create lib.rs with module declarations
    fs::write(
        src.join("lib.rs"),
        r#"
mod handlers;
mod utils;

pub use handlers::api_handler;
pub use utils::helper;
"#,
    )
    .unwrap();

    // Create handlers/mod.rs
    fs::write(
        src.join("handlers").join("mod.rs"),
        r#"
mod api;
pub use api::api_handler;
"#,
    )
    .unwrap();

    // Create handlers/api.rs with imports
    fs::write(
        src.join("handlers").join("api.rs"),
        r#"
use crate::utils::helper;

pub fn api_handler() {
    helper();
}
"#,
    )
    .unwrap();

    // Create utils.rs
    fs::write(
        src.join("utils.rs"),
        r#"
pub fn helper() {
    println!("helping");
}
"#,
    )
    .unwrap();

    temp
}

#[test]
fn test_build_project_graph_full() {
    let project = create_test_rust_project();
    let graph = build_project_graph(project.path()).unwrap();

    // Should find all source files
    assert!(graph.node_count() >= 4, "Expected at least 4 nodes, got {}", graph.node_count());

    // Check specific files exist
    let node_ids: Vec<_> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(node_ids.iter().any(|id| id.ends_with("lib.rs")), "Missing lib.rs");
    assert!(node_ids.iter().any(|id| id.ends_with("utils.rs")), "Missing utils.rs");

    // Edges are best-effort - some may be detected from module declarations
    // The exact count depends on resolution success
    println!("Graph has {} nodes and {} edges", graph.node_count(), graph.edge_count());
}

#[test]
fn test_graph_caching() {
    let project = create_test_rust_project();
    let cache_dir = TempDir::new().unwrap();

    let cache = GraphCache::new(cache_dir.path()).unwrap();

    // Build and cache graph
    let mut graph = build_project_graph(project.path()).unwrap();
    graph.set_built_at(chrono::Utc::now().to_rfc3339());
    cache.save(&graph).unwrap();

    // Load from cache
    let loaded = cache.load(&graph.project_id).unwrap();
    assert!(loaded.is_some());

    let loaded = loaded.unwrap();
    assert_eq!(loaded.node_count(), graph.node_count());
    assert_eq!(loaded.edge_count(), graph.edge_count());
}

#[test]
fn test_graph_filtering() {
    let project = create_test_rust_project();
    let graph = build_project_graph(project.path()).unwrap();

    // Filter by folder
    let handlers_graph = graph.filter_by_folder("src/handlers");
    assert!(handlers_graph.node_count() > 0, "Filter should include handler files");
    assert!(
        handlers_graph.node_count() <= graph.node_count(),
        "Filtered graph should be smaller or equal"
    );
}

#[test]
fn test_graph_neighborhood() {
    let project = create_test_rust_project();
    let graph = build_project_graph(project.path()).unwrap();

    // Find a node that exists
    if let Some(node) = graph.nodes.first() {
        let neighborhood = graph.get_neighborhood(&node.id, 1);
        // Neighborhood should include the center node
        assert!(
            neighborhood.get_node(&node.id).is_some(),
            "Neighborhood should include center node"
        );
    }
}
