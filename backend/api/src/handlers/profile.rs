use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use rememberme_enhancement::ProfileManager;
use rememberme_llm::LLMSingleton;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use storage::SourceProfile;
use tracing::{error, info, warn};

use crate::AppState;

/// Get the profile for a specific source
pub async fn get_profile(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> Result<Json<SourceProfile>, (StatusCode, String)> {
    let profile = state
        .metadata_store
        .get_source_profile(&source_id)
        .map_err(|e| {
            error!("Failed to get source profile for {}: {}", source_id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get profile: {}", e),
            )
        })?;

    Ok(Json(profile))
}

/// Update the profile for a specific source
pub async fn update_profile(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    Json(profile): Json<SourceProfile>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .metadata_store
        .update_source_profile(&source_id, &profile)
        .map_err(|e| {
            error!("Failed to update source profile for {}: {}", source_id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update profile: {}", e),
            )
        })?;

    Ok(StatusCode::OK)
}

/// Request to generate profile from source files
#[derive(Deserialize)]
pub struct GenerateProfileRequest {
    /// List of file patterns to analyze (optional, defaults to LLM-detected patterns)
    pub files: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
struct ProjectTypeResponse {
    project_type: String,
    key_patterns: Vec<String>,
}

/// Generate profile for a specific source using LLM
pub async fn generate_profile(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    Json(req): Json<GenerateProfileRequest>,
) -> Result<Json<SourceProfile>, (StatusCode, String)> {
    info!("Generating profile for source {}...", source_id);

    // Get the source configuration
    let source = state.metadata_store.get_source(&source_id).map_err(|e| {
        error!("Failed to get source {}: {}", source_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get source: {}", e),
        )
    })?;

    let source = source.ok_or_else(|| {
        error!("Source {} not found", source_id);
        (
            StatusCode::NOT_FOUND,
            format!("Source {} not found", source_id),
        )
    })?;

    // Get LLM instance
    let llm = rememberme_llm::LLMSingleton::get().await;
    if llm.is_none() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM is not available".to_string(),
        ));
    }

    let profile_manager = ProfileManager::new(llm.clone());
    let base_path = PathBuf::from(&source.path);

    // Step 1: Get file patterns - from user, auto-detected, or fallback
    let patterns = if let Some(files) = req.files {
        files
    } else {
        // Auto-detect key file patterns from project root
        match ask_llm_for_file_patterns(&base_path, llm).await {
            Ok(patterns) => {
                info!(
                    "Auto-detected patterns for source {}: {:?}",
                    source_id, patterns
                );
                patterns
            }
            Err(e) => {
                warn!("Pattern detection failed: {}. Using fallback patterns", e);
                get_fallback_patterns()
            }
        }
    };

    // Step 2: Query vector DB for chunks matching patterns
    let mut all_chunks = Vec::new();
    for pattern in &patterns {
        match state
            .vector_store
            .get_chunks_by_file_pattern(&source_id, pattern)
            .await
        {
            Ok(chunks) => {
                info!(
                    "Found {} chunks matching pattern: {}",
                    chunks.len(),
                    pattern
                );
                all_chunks.extend(chunks);
            }
            Err(e) => {
                warn!("Failed to query pattern {}: {}", pattern, e);
            }
        }
    }

    // If no chunks found with specific patterns, try getting all chunks
    if all_chunks.is_empty() {
        info!("No chunks found with specific patterns, trying to fetch all chunks limit 50");
        // Fallback: get some chunks to at least generate something
        match state
            .vector_store
            .search(vec![0.0; 384], None, 50) // Zero vector to match anything, or just list
            .await
        {
            Ok(chunks) => {
                info!(
                    "Found {} fallback chunks, first 3 document_ids: {:?}",
                    chunks.len(),
                    chunks
                        .iter()
                        .take(3)
                        .map(|c| c.document_id.clone())
                        .collect::<Vec<_>>()
                );
                all_chunks.extend(chunks);
            }
            Err(e) => {
                warn!("Failed to fetch fallback chunks: {}", e);
            }
        }
    }

    if all_chunks.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "No indexed content found for source {}. Please index the source first.",
                source.name
            ),
        ));
    }

    info!(
        "Generating profile from {} indexed chunks",
        all_chunks.len()
    );

    // Step 3: Generate profile directly from chunks
    let profile = profile_manager
        .generate_initial_profile(all_chunks)
        .await
        .map_err(|e| {
            error!("Failed to generate profile: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to generate profile: {}", e),
            )
        })?;

    // Step 5: Save to metadata store
    state
        .metadata_store
        .update_source_profile(&source_id, &profile)
        .map_err(|e| {
            error!("Failed to save generated profile: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save profile: {}", e),
            )
        })?;

    Ok(Json(profile))
}

/// Analyze root directory and suggest key file patterns (no LLM dependency)
async fn ask_llm_for_file_patterns(
    base_path: &PathBuf,
    _llm: Option<Arc<tokio::sync::Mutex<rememberme_llm::MiniLLM>>>,
) -> Result<Vec<String>, String> {
    // List root directory files
    let root_files = std::fs::read_dir(base_path)
        .map_err(|e| format!("Failed to read directory: {}", e))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().map(|s| s.to_string()))
        .take(50) // Limit to first 50 files
        .collect::<Vec<_>>();

    // Start with common documentation patterns
    use std::collections::HashSet;
    let mut patterns: HashSet<String> = HashSet::new();
    patterns.insert("*.md".to_string());
    patterns.insert("README*".to_string());

    // Simple heuristics based on root files
    let has_cargo = root_files.iter().any(|f| f == "Cargo.toml");
    let has_package_json = root_files.iter().any(|f| f == "package.json");
    let has_pyproject = root_files.iter().any(|f| f == "pyproject.toml");
    let has_requirements = root_files.iter().any(|f| f == "requirements.txt");

    if has_cargo {
        patterns.insert("Cargo.toml".to_string());
        patterns.insert("*.toml".to_string());
    }

    if has_package_json {
        patterns.insert("package.json".to_string());
        patterns.insert("*.config.*".to_string());
        patterns.insert("*.yaml".to_string());
        patterns.insert("*.yml".to_string());
    }

    if has_pyproject {
        patterns.insert("pyproject.toml".to_string());
    }

    if has_requirements {
        patterns.insert("requirements.txt".to_string());
    }

    // If heuristics didn't add much, fall back to the generic patterns
    if patterns.len() <= 2 {
        return Ok(get_fallback_patterns());
    }

    Ok(patterns.into_iter().collect())
}

/// Fallback patterns when LLM fails
fn get_fallback_patterns() -> Vec<String> {
    vec![
        "*.md".to_string(),
        "README*".to_string(),
        "*.toml".to_string(),
        "*.json".to_string(),
        "*.yaml".to_string(),
        "*.yml".to_string(),
        "*.txt".to_string(),
    ]
}
