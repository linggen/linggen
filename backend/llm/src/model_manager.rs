use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Model registry tracking installed models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRegistry {
    pub models: HashMap<String, ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub version: String,
    pub downloaded_at: String,
    pub size_bytes: u64,
    pub files: HashMap<String, FileInfo>,
    pub status: ModelStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModelStatus {
    Ready,
    Downloading,
    Corrupted,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub sha256: Option<String>,
    pub size: u64,
}

/// Model manager for downloading and tracking models
pub struct ModelManager {
    models_dir: PathBuf,
    registry_path: PathBuf,
}

impl ModelManager {
    pub fn new() -> Result<Self> {
        let home_dir =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;

        let models_dir = home_dir.join(".rememberme/models");
        let registry_path = models_dir.join("registry.json");

        // Create models directory if it doesn't exist
        fs::create_dir_all(&models_dir)?;

        Ok(Self {
            models_dir,
            registry_path,
        })
    }

    /// Load registry from disk
    pub fn load_registry(&self) -> Result<ModelRegistry> {
        if !self.registry_path.exists() {
            return Ok(ModelRegistry {
                models: HashMap::new(),
            });
        }

        let content = fs::read_to_string(&self.registry_path)?;
        let registry: ModelRegistry = serde_json::from_str(&content)?;
        Ok(registry)
    }

    /// Save registry to disk
    pub fn save_registry(&self, registry: &ModelRegistry) -> Result<()> {
        let content = serde_json::to_string_pretty(registry)?;
        fs::write(&self.registry_path, content)?;
        Ok(())
    }

    /// Get model status
    pub fn get_model_status(&self, model_id: &str) -> Result<ModelStatus> {
        let registry = self.load_registry()?;

        if let Some(model_info) = registry.models.get(model_id) {
            // Verify files still exist
            let model_dir = self.models_dir.join(model_id);
            if !model_dir.exists() {
                return Ok(ModelStatus::NotFound);
            }

            // Check if all files exist
            for (filename, _) in &model_info.files {
                if !model_dir.join(filename).exists() {
                    return Ok(ModelStatus::Corrupted);
                }
            }

            Ok(model_info.status.clone())
        } else {
            Ok(ModelStatus::NotFound)
        }
    }

    /// Get model directory path
    pub fn get_model_dir(&self, model_id: &str) -> PathBuf {
        self.models_dir.join(model_id)
    }

    /// Check if model exists and is ready
    pub fn is_model_ready(&self, model_id: &str) -> bool {
        matches!(self.get_model_status(model_id), Ok(ModelStatus::Ready))
    }

    /// Register a model in the registry
    pub fn register_model(
        &self,
        model_id: &str,
        name: &str,
        version: &str,
        files: HashMap<String, FileInfo>,
    ) -> Result<()> {
        let mut registry = self.load_registry()?;

        // Calculate total size
        let size_bytes: u64 = files.values().map(|f| f.size).sum();

        let model_info = ModelInfo {
            name: name.to_string(),
            version: version.to_string(),
            downloaded_at: chrono::Utc::now().to_rfc3339(),
            size_bytes,
            files,
            status: ModelStatus::Ready,
        };

        registry.models.insert(model_id.to_string(), model_info);
        self.save_registry(&registry)?;

        Ok(())
    }

    /// Update model status
    pub fn update_model_status(&self, model_id: &str, status: ModelStatus) -> Result<()> {
        let mut registry = self.load_registry()?;

        if let Some(model_info) = registry.models.get_mut(model_id) {
            model_info.status = status;
            self.save_registry(&registry)?;
        }

        Ok(())
    }

    /// Delete a model
    pub fn delete_model(&self, model_id: &str) -> Result<()> {
        let model_dir = self.get_model_dir(model_id);
        if model_dir.exists() {
            fs::remove_dir_all(&model_dir)?;
        }

        let mut registry = self.load_registry()?;
        registry.models.remove(model_id);
        self.save_registry(&registry)?;

        Ok(())
    }

    /// List all models
    pub fn list_models(&self) -> Result<Vec<(String, ModelInfo)>> {
        let registry = self.load_registry()?;
        Ok(registry.models.into_iter().collect())
    }

    /// Verify model integrity (check file sizes)
    pub fn verify_model(&self, model_id: &str) -> Result<bool> {
        let registry = self.load_registry()?;

        if let Some(model_info) = registry.models.get(model_id) {
            let model_dir = self.get_model_dir(model_id);

            for (filename, file_info) in &model_info.files {
                let file_path = model_dir.join(filename);

                if !file_path.exists() {
                    return Ok(false);
                }

                let metadata = fs::metadata(&file_path)?;
                if metadata.len() != file_info.size {
                    return Ok(false);
                }
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl Default for ModelManager {
    fn default() -> Self {
        Self::new().expect("Failed to create ModelManager")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_manager_creation() {
        let manager = ModelManager::new();
        assert!(manager.is_ok());
    }

    #[test]
    fn test_registry_operations() {
        let manager = ModelManager::new().unwrap();
        let registry = manager.load_registry();
        assert!(registry.is_ok());
    }
}
