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

    /// Download Qwen3-4B-Instruct-2507 model files with progress callback
    pub fn download_qwen_model_with_progress<F>(&self, mut progress_fn: F) -> Result<QwenModelFiles>
    where
        F: FnMut(&str),
    {
        tracing::info!("Downloading Qwen3-4B-Instruct-2507 model...");
        progress_fn("Starting download...");

        let repo = self.api.model("Qwen/Qwen3-4B-Instruct-2507".to_string());

        // Download model files (3 safetensors files for Qwen3-4B)
        tracing::info!("  Downloading model weights (part 1/3)...");
        progress_fn("Downloading model weights 1/3...");
        let model_path_1 = repo
            .get("model-00001-of-00003.safetensors")
            .context("Failed to download model-00001-of-00003.safetensors")?;

        tracing::info!("  Downloading model weights (part 2/3)...");
        progress_fn("Downloading model weights 2/3...");
        let model_path_2 = repo
            .get("model-00002-of-00003.safetensors")
            .context("Failed to download model-00002-of-00003.safetensors")?;

        tracing::info!("  Downloading model weights (part 3/3)...");
        progress_fn("Downloading model weights 3/3...");
        let model_path_3 = repo
            .get("model-00003-of-00003.safetensors")
            .context("Failed to download model-00003-of-00003.safetensors")?;

        // Download tokenizer
        tracing::info!("  Downloading tokenizer...");
        progress_fn("Downloading tokenizer (4/5)...");
        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("Failed to download tokenizer.json")?;

        // Download config
        tracing::info!("  Downloading config...");
        progress_fn("Downloading config (5/5)...");
        let config_path = match repo.get("config.json") {
            Ok(path) => path,
            Err(e) => {
                tracing::warn!(
                    "hf_hub failed to download config.json, attempting fallback with curl: {}",
                    e
                );
                let fallback_dir = dirs::cache_dir()
                    .ok_or_else(|| anyhow::anyhow!("No cache dir"))?
                    .join("rememberme")
                    .join("models");
                std::fs::create_dir_all(&fallback_dir)?;
                let output_path = fallback_dir.join("config.json");

                let status = std::process::Command::new("curl")
                    .arg("-L")
                    .arg("-o")
                    .arg(&output_path)
                    .arg("https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507/resolve/main/config.json")
                    .status()
                    .context("Failed to execute curl")?;

                if !status.success() {
                    return Err(anyhow::anyhow!("Fallback download failed"));
                }
                output_path
            }
        };

        tracing::info!("✓ Model downloaded successfully");
        progress_fn("Model downloaded successfully!");

        Ok(QwenModelFiles {
            model_paths: vec![model_path_1, model_path_2, model_path_3],
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
    pub model_paths: Vec<PathBuf>,
    pub tokenizer_path: PathBuf,
    pub config_path: PathBuf,
}
