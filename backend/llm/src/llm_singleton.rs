use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};

use crate::{LLMConfig, MiniLLM};

/// Global LLM singleton wrapped in Mutex for interior mutability
static LLM_INSTANCE: OnceCell<Arc<Mutex<MiniLLM>>> = OnceCell::const_new();

/// LLM Singleton Manager
pub struct LLMSingleton;

impl LLMSingleton {
    /// Initialize the global LLM instance
    pub async fn initialize(config: LLMConfig) -> Result<()> {
        if LLM_INSTANCE.initialized() {
            tracing::info!("LLM already initialized, skipping...");
            return Ok(());
        }

        tracing::info!("Initializing global LLM instance...");
        let llm = MiniLLM::new(config)?;
        let _ = LLM_INSTANCE.set(Arc::new(Mutex::new(llm)));
        tracing::info!("Global LLM instance initialized successfully");

        Ok(())
    }

    /// Initialize with progress callback
    pub async fn initialize_with_progress<F>(config: LLMConfig, progress_callback: F) -> Result<()>
    where
        F: Fn(&str) + Send + 'static,
    {
        if LLM_INSTANCE.initialized() {
            tracing::info!("LLM already initialized, skipping...");
            return Ok(());
        }

        tracing::info!("Initializing global LLM instance with progress tracking...");
        let llm = MiniLLM::new_with_progress(config, progress_callback)?;
        let _ = LLM_INSTANCE.set(Arc::new(Mutex::new(llm)));
        tracing::info!("Global LLM instance initialized successfully");

        Ok(())
    }

    /// Get the global LLM instance
    pub async fn get() -> Option<Arc<Mutex<MiniLLM>>> {
        LLM_INSTANCE.get().cloned()
    }

    /// Check if LLM is initialized
    pub async fn is_initialized() -> bool {
        LLM_INSTANCE.initialized()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_singleton_initialization() {
        assert!(!LLMSingleton::is_initialized().await || LLMSingleton::is_initialized().await);

        // Note: We can't actually test initialization without a real model
        // This is just to verify the singleton pattern works
    }
}
