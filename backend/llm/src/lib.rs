use anyhow::Result;
use candle_core::{Device, IndexOp, Tensor};
use candle_transformers::models::qwen3::ModelForCausalLM as Qwen3Model;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokenizers::Tokenizer;

pub mod downloader;
pub mod llm_singleton;
pub mod model_manager;
pub mod model_utils;

use downloader::ModelDownloader;
pub use llm_singleton::LLMSingleton;
pub use model_manager::{FileInfo, ModelInfo, ModelManager, ModelRegistry, ModelStatus};

/// Mini LLM wrapper for running lightweight models (Qwen, Phi, etc.)
pub struct MiniLLM {
    model: Qwen3Model,
    tokenizer: Tokenizer,
    device: Device,
    config: LLMConfig,
}

impl MiniLLM {
    /// Clear the model's KV cache by calling clear_kv_cache
    pub fn clear_cache(&mut self) {
        self.model.clear_kv_cache();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    pub model_path: Option<PathBuf>,
    pub tokenizer_path: Option<PathBuf>,
    pub max_tokens: usize,
    pub temperature: f64,
    pub top_p: f64,
    pub repeat_penalty: f32,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            model_path: None, // Will auto-download if None
            tokenizer_path: None,
            max_tokens: 1024,
            // Temperature 0.7 provides good balance between creativity and coherence
            // (avoids greedy sampling that can cause repetition with small models)
            temperature: 0.7,
            top_p: 0.95,
            // Higher repeat_penalty discourages the model from repeating tokens
            repeat_penalty: 1.3,
        }
    }
}

impl MiniLLM {
    /// Create a new MiniLLM instance
    pub fn new(config: LLMConfig) -> Result<Self> {
        Self::new_with_progress(config, |_| {})
    }

    /// Create a new MiniLLM instance with progress callback
    pub fn new_with_progress<F>(config: LLMConfig, mut progress_fn: F) -> Result<Self>
    where
        F: FnMut(&str),
    {
        // Initialize device (Metal on macOS, CUDA on Linux, CPU fallback)
        progress_fn("Initializing device...");
        let device = Self::get_device()?;

        // Get or download model files
        let (model_paths, tokenizer_path, config_path) =
            if config.model_path.is_none() || config.tokenizer_path.is_none() {
                tracing::info!("No model path provided, downloading from Hugging Face...");
                let downloader = ModelDownloader::new()?;
                let files = downloader.download_qwen_model_with_progress(&mut progress_fn)?;
                (files.model_paths, files.tokenizer_path, files.config_path)
            } else {
                // If paths are provided, assume config is in same directory
                let model_path = config.model_path.clone().unwrap();
                let config_path = model_path
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("Invalid model path"))?
                    .join("config.json");
                (
                    vec![model_path],
                    config.tokenizer_path.clone().unwrap(),
                    config_path,
                )
            };

        tracing::info!("Loading model from {} files", model_paths.len());
        progress_fn("Loading model into memory...");
        let model = Self::load_model(&model_paths, &config_path, &device)?;

        tracing::info!("Loading tokenizer from: {:?}", tokenizer_path);
        progress_fn("Loading tokenizer...");
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

        progress_fn("Model ready!");

        Ok(Self {
            model,
            tokenizer,
            device,
            config,
        })
    }

    /// Get the best available device
    fn get_device() -> Result<Device> {
        #[cfg(feature = "metal")]
        {
            match Device::new_metal(0) {
                Ok(device) => {
                    tracing::info!("Using Metal GPU acceleration");
                    return Ok(device);
                }
                Err(e) => {
                    tracing::warn!("Metal not available: {}, falling back to CPU", e);
                }
            }
        }

        #[cfg(feature = "cuda")]
        {
            match Device::new_cuda(0) {
                Ok(device) => {
                    tracing::info!("Using CUDA GPU acceleration");
                    return Ok(device);
                }
                Err(e) => {
                    tracing::warn!("CUDA not available: {}, falling back to CPU", e);
                }
            }
        }

        tracing::info!("Using CPU");
        Ok(Device::Cpu)
    }

    /// Load the model from multiple files
    fn load_model(
        model_paths: &[PathBuf],
        config_path: &PathBuf,
        device: &Device,
    ) -> Result<Qwen3Model> {
        model_utils::load_model(model_paths, config_path, device)
    }

    /// Generate text from a prompt
    pub async fn generate(&mut self, prompt: &str, max_tokens: usize) -> Result<String> {
        self.generate_with_system("", prompt, max_tokens).await
    }

    /// Generate text with a system prompt
    /// Generate text with a system prompt
    pub async fn generate_with_system(
        &mut self,
        system: &str,
        user: &str,
        max_tokens: usize,
    ) -> Result<String> {
        // Try fast generation first
        match self.generate_fast(system, user, max_tokens).await {
            Ok(text) => Ok(text),
            Err(e) => {
                tracing::warn!(
                    "Fast generation failed: {}. Falling back to robust (slow) generation.",
                    e
                );
                self.generate_robust(system, user, max_tokens).await
            }
        }
    }

    /// Fast generation using KV caching (O(N))
    async fn generate_fast(
        &mut self,
        system: &str,
        user: &str,
        max_tokens: usize,
    ) -> Result<String> {
        self.clear_cache();

        // Format prompt
        let formatted_prompt = if system.is_empty() {
            format!(
                "<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
                user
            )
        } else {
            format!(
                "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
                system, user
            )
        };

        // Tokenize
        let encoding = self
            .tokenizer
            .encode(formatted_prompt.clone(), true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

        let prompt_tokens = encoding.get_ids().to_vec();
        let eos_token = self.tokenizer.token_to_id("<|im_end|>").unwrap_or(151645);

        let mut all_tokens = prompt_tokens.clone();
        let mut last_token: Option<u32> = None;
        let mut repeat_run: u32 = 0;

        // 1. Process prompt (prefill)
        let mut input_ids = Tensor::new(prompt_tokens.as_slice(), &self.device)?.unsqueeze(0)?;
        let mut logits = self.model.forward(&input_ids, 0)?;
        let mut start_pos = prompt_tokens.len();

        // 2. Generation loop
        for _ in 0..max_tokens {
            // Get logits for the last token only
            let seq_len = logits.dim(1)?;
            let next_token_logits = logits.i((0, seq_len - 1))?; // [batch, vocab]

            // Sample next token
            let next_token_logits = next_token_logits.to_dtype(candle_core::DType::F32)?;
            let next_token = model_utils::sample_token(
                &next_token_logits,
                self.config.temperature,
                self.config.top_p,
                self.config.repeat_penalty,
                &all_tokens[prompt_tokens.len()..], // Only penalize generated tokens
            )?;

            // Repetition check
            if Some(next_token) == last_token {
                repeat_run += 1;
            } else {
                repeat_run = 0;
                last_token = Some(next_token);
            }
            if repeat_run >= 32 {
                tracing::warn!("Fast generation stopped early due to repetition");
                break;
            }

            if next_token == eos_token {
                break;
            }

            all_tokens.push(next_token);

            // Prepare next input (single token)
            input_ids = Tensor::new(&[next_token], &self.device)?.unsqueeze(0)?;

            // Forward pass with KV cache
            logits = self.model.forward(&input_ids, start_pos)?;
            start_pos += 1;
        }

        // Decode
        let generated_part = &all_tokens[prompt_tokens.len()..];
        let text = self
            .tokenizer
            .decode(generated_part, true)
            .map_err(|e| anyhow::anyhow!("Decoding failed: {}", e))?;

        tracing::debug!("Fast generation finished. Length: {}", text.len());
        Ok(text)
    }

    /// Robust (but slow) generation that re-processes context every step (O(N^2))
    /// Used as fallback if fast generation fails due to shape/cache errors.
    async fn generate_robust(
        &mut self,
        system: &str,
        user: &str,
        max_tokens: usize,
    ) -> Result<String> {
        // Clear KV cache before each generation to avoid state issues
        self.clear_cache();

        // Format prompt in Qwen chat template format
        let formatted_prompt = if system.is_empty() {
            format!(
                "<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
                user
            )
        } else {
            format!(
                "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
                system, user
            )
        };

        // Tokenize
        let encoding = self
            .tokenizer
            .encode(formatted_prompt.clone(), true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

        let mut tokens = encoding.get_ids().to_vec();
        let eos_token = self.tokenizer.token_to_id("<|im_end|>").unwrap_or(151645); // Qwen2 default EOS token

        // Initial setup for generation. We avoid manual start_pos bookkeeping here
        // because incorrect offsets can cause runtime shape errors inside the model
        // on some backends (e.g., "narrow invalid args start > dim_len").
        //
        // Instead, on each step we feed the *entire* token sequence to the model
        // with start_pos = 0. This is slightly less efficient but much more robust.
        let mut input_ids; // = Tensor::new(tokens.clone(), &self.device)?.unsqueeze(0)?;
                           // Simple repetition guard: track runs of the same token to avoid degeneracy
        let mut last_token: Option<u32> = None;
        let mut repeat_run: u32 = 0;

        // Generate tokens one at a time
        for _ in 0..max_tokens {
            // Since we are feeding the full sequence every time with start_pos=0
            // (to avoid the shape issues we saw earlier with incremental generation),
            // we MUST clear the KV cache on each step so it doesn't accumulate duplicate history.
            self.clear_cache();

            // Always feed the full sequence so far. This avoids mismatches between
            // the model's internal KV cache and the external start_pos we pass.
            input_ids = Tensor::new(tokens.clone(), &self.device)?.unsqueeze(0)?;

            // Forward pass with KV caching
            // start_pos tracks where we are in the sequence, allowing the model to use cached keys/values
            // We always pass start_pos = 0 here to avoid out-of-bounds issues on
            // some devices when manually tracking the offset across incremental calls.
            let logits = self.model.forward(&input_ids, 0)?;

            let seq_len = input_ids.dim(1)?;

            // Get logits for last token
            // Handle case where model returns [batch, 1, vocab] (just last token)
            // vs [batch, seq_len, vocab] (full sequence)
            let logits = if logits.dim(1)? == 1 {
                logits.i((0, 0))?
            } else {
                logits.i((0, seq_len - 1))?
            };

            // Cast logits to F32 for sampling (Candle sampling usually requires F32)
            let logits = logits.to_dtype(candle_core::DType::F32)?;

            // Sample next token with repetition penalty applied to all generated tokens
            let prompt_len = encoding.get_ids().len();
            let generated_tokens = &tokens[prompt_len..];
            let next_token = model_utils::sample_token(
                &logits,
                self.config.temperature,
                self.config.top_p,
                self.config.repeat_penalty,
                generated_tokens,
            )?;

            // Repetition guard: if we see a long run of the same token, stop early
            if Some(next_token) == last_token {
                repeat_run += 1;
            } else {
                repeat_run = 0;
                last_token = Some(next_token);
            }
            if repeat_run >= 32 {
                tracing::warn!(
                    "LLM generation stopped early due to repeated token run (token={}, run={})",
                    next_token,
                    repeat_run
                );
                break;
            }

            // Check for EOS
            if next_token == eos_token {
                break;
            }

            tokens.push(next_token);
        }

        // Decode generated tokens (skip the prompt)
        let prompt_len = encoding.get_ids().len();
        let generated_tokens = &tokens[prompt_len..];

        let text = self
            .tokenizer
            .decode(generated_tokens, true)
            .map_err(|e| anyhow::anyhow!("Decoding failed: {}", e))?;

        // Log length only to avoid dumping huge, potentially noisy outputs.
        tracing::debug!(
            "LLM Generated Output length: {} chars",
            text.chars().count()
        );

        Ok(text)
    }
}

/// Get the shared LLM instance (lazy initialized)
pub fn get_llm() -> Result<Arc<MiniLLM>> {
    use std::sync::Mutex;

    static INSTANCE: Mutex<Option<Arc<MiniLLM>>> = Mutex::new(None);

    let mut guard = INSTANCE.lock().unwrap();
    if guard.is_none() {
        let config = LLMConfig::default();
        *guard = Some(Arc::new(MiniLLM::new(config)?));
    }

    Ok(guard.as_ref().unwrap().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_selection() {
        let device = MiniLLM::get_device();
        assert!(device.is_ok());
    }

    #[test]
    fn test_default_config() {
        let config = LLMConfig::default();
        assert_eq!(config.max_tokens, 1024);
        assert_eq!(config.temperature, 0.7);
    }
}
