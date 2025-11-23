use anyhow::Result;
use candle_core::{Device, IndexOp, Tensor};
use candle_transformers::models::qwen2::Model as Qwen2Model;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokenizers::Tokenizer;

pub mod downloader;
pub mod model_utils;

use downloader::ModelDownloader;

/// Mini LLM wrapper for running lightweight models (Qwen, Phi, etc.)
pub struct MiniLLM {
    model: Qwen2Model,
    tokenizer: Tokenizer,
    device: Device,
    config: LLMConfig,
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
            max_tokens: 512,
            temperature: 0.7,
            top_p: 0.9,
            repeat_penalty: 1.1,
        }
    }
}

impl MiniLLM {
    /// Create a new MiniLLM instance
    pub fn new(config: LLMConfig) -> Result<Self> {
        // Initialize device (Metal on macOS, CUDA on Linux, CPU fallback)
        let device = Self::get_device()?;

        // Get or download model files
        let (model_path, tokenizer_path, config_path) =
            if config.model_path.is_none() || config.tokenizer_path.is_none() {
                println!("No model path provided, downloading from Hugging Face...");
                let downloader = ModelDownloader::new()?;
                let files = downloader.download_qwen_model()?;
                (files.model_path, files.tokenizer_path, files.config_path)
            } else {
                // If paths are provided, assume config is in same directory
                let model_path = config.model_path.clone().unwrap();
                let config_path = model_path
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("Invalid model path"))?
                    .join("config.json");
                (
                    model_path,
                    config.tokenizer_path.clone().unwrap(),
                    config_path,
                )
            };

        println!("Loading model from: {:?}", model_path);
        let model = Self::load_model(&model_path, &config_path, &device)?;

        println!("Loading tokenizer from: {:?}", tokenizer_path);
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

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
                    println!("Using Metal GPU acceleration");
                    return Ok(device);
                }
                Err(e) => {
                    eprintln!("Metal not available: {}, falling back to CPU", e);
                }
            }
        }

        #[cfg(feature = "cuda")]
        {
            match Device::new_cuda(0) {
                Ok(device) => {
                    println!("Using CUDA GPU acceleration");
                    return Ok(device);
                }
                Err(e) => {
                    eprintln!("CUDA not available: {}, falling back to CPU", e);
                }
            }
        }

        println!("Using CPU");
        Ok(Device::Cpu)
    }

    /// Load the model from file
    fn load_model(
        model_path: &PathBuf,
        config_path: &PathBuf,
        device: &Device,
    ) -> Result<Qwen2Model> {
        model_utils::load_model(model_path, config_path, device)
    }

    /// Generate text from a prompt
    pub async fn generate(&mut self, prompt: &str, max_tokens: usize) -> Result<String> {
        self.generate_with_system("", prompt, max_tokens).await
    }

    /// Generate text with a system prompt
    pub async fn generate_with_system(
        &mut self,
        system: &str,
        user: &str,
        max_tokens: usize,
    ) -> Result<String> {
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

        // Generate tokens
        for _ in 0..max_tokens {
            // Convert tokens to tensor
            let input_ids = Tensor::new(tokens.clone(), &self.device)?;
            let input_ids = input_ids.unsqueeze(0)?; // Add batch dimension

            // Forward pass (position_ids will be auto-generated as None)
            let logits = self.model.forward(&input_ids, 0, None)?;

            // Get logits for last token
            let logits = logits.i((0, tokens.len() - 1))?;

            // Sample next token
            let next_token =
                model_utils::sample_token(&logits, self.config.temperature, self.config.top_p)?;

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
        assert_eq!(config.max_tokens, 512);
        assert_eq!(config.temperature, 0.7);
    }
}
