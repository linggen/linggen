use serde::{Deserialize, Serialize};

/// Source profile: metadata about a specific source (repository, folder, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceProfile {
    /// Profile name (e.g., "Default Profile", "Production Config")
    pub profile_name: String,
    /// Brief description of the source
    pub description: String,
}

impl SourceProfile {
    pub fn new(profile_name: String, description: String) -> Self {
        Self {
            profile_name,
            description,
        }
    }
}
