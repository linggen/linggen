use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use rememberme_enhancement::ProfileManager;
use rememberme_llm::LLMSingleton;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use storage::SourceProfile;
use tracing::{error, info};

use crate::AppState;

/// Get the profile for a specific source
pub async fn get_profile(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> Result<Json<SourceProfile>, (StatusCode, String)> {
    let profile = state.metadata_store.get_source_profile(&source_id).map_err(|e| {
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
    /// List of file paths to analyze (optional, defaults to scanning the source directory)
    pub files: Option<Vec<String>>,
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
    let llm = LLMSingleton::get().await;
    if llm.is_none() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM is not available".to_string(),
        ));
    }

    let profile_manager = ProfileManager::new(llm);

    // 1. Auto-detect project type and get relevant files
    let base_path = PathBuf::from(&source.path);
    let files_to_scan = if let Some(files) = req.files {
        files.into_iter().map(|f| base_path.join(f)).collect()
    } else {
        discover_project_files(&base_path)
    };

    // 2. Read file contents
    let mut file_contents = Vec::new();
    for path in files_to_scan {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                file_contents.push((path, content));
            }
        }
    }

    if file_contents.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("No valid source files found in {}", source.path),
        ));
    }

    // 3. Generate profile
    let profile = profile_manager
        .generate_initial_profile(file_contents)
        .await
        .map_err(|e| {
            error!("Failed to generate profile: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to generate profile: {}", e),
            )
        })?;

    // 4. Save to metadata store
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

/// Intelligently discover relevant files for profile generation
/// Auto-detects project type and returns appropriate files
fn discover_project_files(base_path: &PathBuf) -> Vec<PathBuf> {
    let mut files = Vec::new();
    
    // Always look for documentation files
    for doc_pattern in &["README.md", "README.txt", "README", "CONTRIBUTING.md", "docs/README.md"] {
        let path = base_path.join(doc_pattern);
        if path.exists() {
            files.push(path);
        }
    }
    
    // Detect project type and add type-specific files
    let mut found_type = false;
    
    // Rust project detection
    if base_path.join("Cargo.toml").exists() {
        found_type = true;
        files.push(base_path.join("Cargo.toml"));
        // Check for main entry points
        if base_path.join("src/lib.rs").exists() {
            files.push(base_path.join("src/lib.rs"));
        }
        if base_path.join("src/main.rs").exists() {
            files.push(base_path.join("src/main.rs"));
        }
    }
    
    // JavaScript/TypeScript project detection
    if base_path.join("package.json").exists() {
        found_type = true;
        files.push(base_path.join("package.json"));
        // Check for config files
        for config in &["tsconfig.json", "vite.config.ts", "next.config.js", "webpack.config.js"] {
            let path = base_path.join(config);
            if path.exists() {
                files.push(path);
            }
        }
        // Check for entry points
        for entry in &["src/index.ts", "src/index.js", "src/main.ts", "src/App.tsx"] {
            let path = base_path.join(entry);
            if path.exists() {
                files.push(path);
            }
        }
    }
    
    // Python project detection
    if base_path.join("setup.py").exists() || base_path.join("pyproject.toml").exists() {
        found_type = true;
        if base_path.join("setup.py").exists() {
            files.push(base_path.join("setup.py"));
        }
        if base_path.join("pyproject.toml").exists() {
            files.push(base_path.join("pyproject.toml"));
        }
        if base_path.join("requirements.txt").exists() {
            files.push(base_path.join("requirements.txt"));
        }
        // Check for main module
        if base_path.join("__init__.py").exists() {
            files.push(base_path.join("__init__.py"));
        }
    }
    
    // Go project detection
    if base_path.join("go.mod").exists() {
        found_type = true;
        files.push(base_path.join("go.mod"));
        if base_path.join("main.go").exists() {
            files.push(base_path.join("main.go"));
        }
    }
    
    // Java/Maven project detection
    if base_path.join("pom.xml").exists() {
        found_type = true;
        files.push(base_path.join("pom.xml"));
    }
    
    // If no specific type detected, try to find any config files
    if !found_type {
        for pattern in &[".project", "*.toml", "*.yaml", "*.yml", "config.json"] {
            if let Ok(entries) = std::fs::read_dir(base_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if pattern.contains('*') {
                            let ext = pattern.trim_start_matches("*.");
                            if name.ends_with(ext) {
                                files.push(path);
                                break;
                            }
                        } else if name == *pattern {
                            files.push(path);
                        }
                    }
                }
            }
        }
    }
    
    // Limit to first 10 files to avoid overwhelming the LLM
    files.truncate(10);
    files
}
