use serde::{Deserialize, Serialize};

/// Source profile: metadata about a specific source (repository, folder, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceProfile {
    /// Source name
    pub name: String,
    /// Brief description of the source
    pub description: String,
    /// List of technologies used (e.g., "Rust", "React", "Tauri")
    pub tech_stack: Vec<String>,
    /// Architectural notes (e.g., "Microservices", "Clean Architecture")
    pub architecture_notes: Vec<String>,
    /// Coding conventions (e.g., "Use snake_case", "Prefer composition")
    pub key_conventions: Vec<String>,
}

impl SourceProfile {
    pub fn new(name: String, description: String) -> Self {
        Self {
            name,
            description,
            ..Default::default()
        }
    }
}
