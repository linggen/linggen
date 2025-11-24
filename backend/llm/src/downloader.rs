use anyhow::{Context, Result};
use hf_hub::api::sync::Api;
use std::path::PathBuf;

/// Model downloader from Hugging Face
pub struct ModelDownloader {
    api: Api,
    #[allow(dead_code)]
    cache_dir: PathBuf,
}

impl ModelDownloader {
    pub fn new() -> Result<Self> {
        let api = Api::new()?;
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find cache directory"))?
            .join("rememberme")
            .join("models");

        std::fs::create_dir_all(&cache_dir)?;

        Ok(Self { api, cache_dir })
    }

    /// Download Qwen2.5-1.5B-Instruct model files
    pub fn download_qwen_model(&self) -> Result<QwenModelFiles> {
        self.download_qwen_model_with_progress(|_| {})
    }

    /// Download Qwen2.5-1.5B-Instruct model files with progress callback
    pub fn download_qwen_model_with_progress<F>(&self, mut progress_fn: F) -> Result<QwenModelFiles>
    where
        F: FnMut(&str),
    {
        println!("Downloading Qwen2.5-1.5B-Instruct model...");
        progress_fn("Starting download...");

        let repo = self.api.model("Qwen/Qwen2.5-1.5B-Instruct".to_string());

        // Download model file (safetensors)
        println!("  Downloading model weights...");
        progress_fn("Downloading model weights (1/3)...");
        let model_path = repo
            .get("model.safetensors")
            .context("Failed to download model.safetensors")?;

        // Download tokenizer
        println!("  Downloading tokenizer...");
        progress_fn("Downloading tokenizer (2/3)...");
        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("Failed to download tokenizer.json")?;

        // Download config
        println!("  Downloading config...");
        progress_fn("Downloading config (3/3)...");
        let config_path = repo
            .get("config.json")
            .context("Failed to download config.json")?;

        println!("✓ Model downloaded successfully");
        progress_fn("Model downloaded successfully!");

        Ok(QwenModelFiles {
            model_path,
            tokenizer_path,
            config_path,
        })
    }
}

impl Default for ModelDownloader {
    fn default() -> Self {
        Self::new().expect("Failed to initialize model downloader")
    }
}

pub struct QwenModelFiles {
    pub model_path: PathBuf,
    pub tokenizer_path: PathBuf,
    pub config_path: PathBuf,
}
