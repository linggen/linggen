use anyhow::Result;
use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use hf_hub::{api::sync::Api, Repo, RepoType};
use tokenizers::Tokenizer;
use tracing::info;

/// Embedding model wrapper using Candle
pub struct EmbeddingModel {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    /// Lock to prevent concurrent forward passes on the same device (critical for Metal)
    forward_lock: std::sync::Mutex<()>,
}

impl EmbeddingModel {
    /// Load the all-MiniLM-L6-v2 model from HuggingFace
    pub fn new() -> Result<Self> {
        info!("Loading embedding model: sentence-transformers/all-MiniLM-L6-v2");

        // Determine device
        #[cfg(target_os = "macos")]
        let device = Device::new_metal(0).unwrap_or(Device::Cpu);

        #[cfg(not(target_os = "macos"))]
        let device = {
            #[cfg(feature = "cuda")]
            {
                Device::new_cuda(0).unwrap_or(Device::Cpu)
            }
            #[cfg(not(feature = "cuda"))]
            {
                Device::Cpu
            }
        };

        info!("Attempting to use device: {:?}", device);

        // Try to load and test on the preferred device
        match Self::load_model(device.clone()) {
            Ok(model) => {
                // Test the model with a dummy embedding to ensure device works (e.g. shaders compile)
                match model.embed("test") {
                    Ok(_) => {
                        info!("Successfully initialized embedding model on {:?}", device);
                        Ok(model)
                    }
                    Err(e) => {
                        // Fallback to CPU if test fails
                        tracing::warn!(
                            "Failed to run model on {:?}: {}. Falling back to CPU.",
                            device,
                            e
                        );
                        Self::load_model(Device::Cpu)
                    }
                }
            }
            Err(e) => {
                // Fallback to CPU if load fails
                tracing::warn!(
                    "Failed to load model on {:?}: {}. Falling back to CPU.",
                    device,
                    e
                );
                Self::load_model(Device::Cpu)
            }
        }
    }

    fn load_model(device: Device) -> Result<Self> {
        // Ensure HuggingFace hub cache is writable in headless/systemd environments.
        //
        // On Linux, when running as a system service (e.g. systemd DynamicUser), `HOME` can be unset
        // or point to a non-writable location. hf_hub then may try to write into a read-only
        // filesystem and fail with "Read-only file system (os error 30)".
        //
        // We prefer explicit env vars if already set; otherwise derive from LINGGEN_DATA_DIR.
        if std::env::var_os("HF_HOME").is_none()
            && std::env::var_os("HF_HUB_CACHE").is_none()
            && std::env::var_os("HUGGINGFACE_HUB_CACHE").is_none()
        {
            if let Ok(base) = std::env::var("LINGGEN_DATA_DIR") {
                let base = std::path::PathBuf::from(base);
                let hf_home = base.join("hf");
                let hub = hf_home.join("hub");
                let _ = std::fs::create_dir_all(&hub);
                std::env::set_var("HF_HOME", &hf_home);
                std::env::set_var("HF_HUB_CACHE", &hub);
                std::env::set_var("HUGGINGFACE_HUB_CACHE", &hub);
            }
        }

        // Download model from HuggingFace Hub
        let api = Api::new()?;
        let repo = api.repo(Repo::new(
            "sentence-transformers/all-MiniLM-L6-v2".to_string(),
            RepoType::Model,
        ));

        let config_path = repo.get("config.json")?;
        let tokenizer_path = repo.get("tokenizer.json")?;
        let weights_path = repo.get("model.safetensors")?;

        // Load config and model
        let config = std::fs::read_to_string(config_path)?;
        let config: Config = serde_json::from_str(&config)?;
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, &device)? };
        let model = BertModel::load(vb, &config)?;

        // Load tokenizer
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

        Ok(Self {
            model,
            tokenizer,
            device,
            forward_lock: std::sync::Mutex::new(()),
        })
    }

    /// Embed a single text into a vector
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_batch(&[text])
            .map(|mut batch| batch.pop().unwrap_or_default())
    }

    /// Embed multiple texts in a batch (TRUE batching - single forward pass)
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Tokenize all texts
        info!(
            "EmbeddingModel::embed_batch: tokenizing {} texts",
            texts.len()
        );
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;
        info!(
            "EmbeddingModel::embed_batch: obtained {} encodings",
            encodings.len()
        );

        // Find max sequence length for padding
        let max_len = encodings.iter().map(|e| e.len()).max().unwrap_or(0);
        info!(
            "EmbeddingModel::embed_batch: max_len for padding = {}",
            max_len
        );
        // Prepare batch tensors with padding
        let batch_size = texts.len();
        let mut batch_token_ids = Vec::with_capacity(batch_size * max_len);
        let mut batch_attention_mask = Vec::with_capacity(batch_size * max_len);

        for encoding in &encodings {
            let ids = encoding.get_ids();
            let attention_mask = encoding.get_attention_mask();

            // Add tokens
            batch_token_ids.extend_from_slice(ids);
            // Pad with zeros
            batch_token_ids.extend(vec![0u32; max_len - ids.len()]);

            // Add attention mask
            batch_attention_mask.extend_from_slice(attention_mask);
            // Pad mask with zeros (ignore padding tokens)
            batch_attention_mask.extend(vec![0u32; max_len - attention_mask.len()]);
        }

        // Acquire lock before any device operations to prevent concurrent Metal command encoder conflicts
        let _lock = self
            .forward_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("Forward lock poisoned: {}", e))?;
        tracing::debug!("Acquired forward lock for embedding batch pass");

        // Create batch tensors [batch_size, max_len]
        let token_ids = Tensor::from_vec(batch_token_ids, (batch_size, max_len), &self.device)?;
        info!(
            "EmbeddingModel::embed_batch: token_ids shape = {:?}",
            token_ids.dims()
        );
        let token_type_ids =
            Tensor::zeros((batch_size, max_len), candle_core::DType::U32, &self.device)?;
        let attention_mask =
            Tensor::from_vec(batch_attention_mask, (batch_size, max_len), &self.device)?;
        info!(
            "EmbeddingModel::embed_batch: attention_mask shape = {:?}",
            attention_mask.dims()
        );

        // Single forward pass for entire batch! 🚀
        let outputs = self
            .model
            .forward(&token_ids, &token_type_ids, Some(&attention_mask))?;
        info!(
            "EmbeddingModel::embed_batch: model output shape = {:?}",
            outputs.dims()
        );
        // outputs shape: [batch_size, seq_len, hidden_size]
        // Apply mean pooling with attention mask
        let embeddings_tensor = self.mean_pooling_with_mask(&outputs, &attention_mask)?;
        info!(
            "EmbeddingModel::embed_batch: embeddings_tensor shape after pooling = {:?}",
            embeddings_tensor.dims()
        );

        // embeddings_tensor shape: [batch_size, hidden_size]
        // Normalize each embedding
        let normalized = self.normalize_batch(&embeddings_tensor)?;
        info!(
            "EmbeddingModel::embed_batch: normalized tensor shape = {:?}",
            normalized.dims()
        );

        // Convert to Vec<Vec<f32>>
        let mut embeddings = Vec::with_capacity(batch_size);
        for i in 0..batch_size {
            let embedding = normalized.get(i)?;
            let embedding_vec = embedding.to_vec1::<f32>()?;
            embeddings.push(embedding_vec);
        }

        Ok(embeddings)
    }

    /// Mean pooling with attention mask (for batches)
    fn mean_pooling_with_mask(&self, tensor: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
        // tensor shape: [batch_size, seq_len, hidden_size]
        // attention_mask shape: [batch_size, seq_len]

        let (batch_size, seq_len, hidden_size) = tensor.dims3()?;

        // Expand attention mask to match hidden size: [batch_size, seq_len, hidden_size]
        let mask_expanded = attention_mask
            .unsqueeze(2)? // [batch_size, seq_len, 1]
            .broadcast_as((batch_size, seq_len, hidden_size))?;

        // Convert mask to float for multiplication
        let mask_float = mask_expanded.to_dtype(candle_core::DType::F32)?;
        let tensor_float = tensor.to_dtype(candle_core::DType::F32)?;

        // Apply mask: zero out padding tokens
        let masked = tensor_float.mul(&mask_float)?;

        // Sum over sequence length
        let summed = masked.sum(1)?; // [batch_size, hidden_size]

        // Count non-padding tokens per sequence
        let mask_sum = mask_float.sum(1)?; // [batch_size, hidden_size]

        // Avoid division by zero
        let mask_sum_safe = mask_sum
            .maximum(&Tensor::new(&[1e-9f32], &self.device)?.broadcast_as(mask_sum.shape())?)?;

        // Average: divide by number of non-padding tokens
        let pooled = summed.div(&mask_sum_safe)?;

        Ok(pooled)
    }

    /// L2 normalize batch of embeddings
    fn normalize_batch(&self, tensor: &Tensor) -> Result<Tensor> {
        // tensor shape: [batch_size, hidden_size]
        let (batch_size, hidden_size) = tensor.dims2()?;

        // Calculate L2 norm for each embedding: sqrt(sum(x^2))
        let squared = tensor.sqr()?; // [batch_size, hidden_size]
        let sum_squared = squared.sum(1)?; // [batch_size]
        let norms = sum_squared.sqrt()?; // [batch_size]

        // Expand norms to [batch_size, hidden_size] for division
        let norms_expanded = norms
            .unsqueeze(1)? // [batch_size, 1]
            .broadcast_as((batch_size, hidden_size))?;

        // Avoid division by zero
        let norms_safe = norms_expanded.maximum(
            &Tensor::new(&[1e-12f32], &self.device)?.broadcast_as(norms_expanded.shape())?,
        )?;

        // Normalize: divide each embedding by its norm
        let normalized = tensor.div(&norms_safe)?;

        Ok(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires model download
    fn test_embed_single() -> Result<()> {
        let model = EmbeddingModel::new()?;
        let embedding = model.embed("Hello world")?;

        // all-MiniLM-L6-v2 produces 384-dimensional embeddings
        assert_eq!(embedding.len(), 384);

        // Check that embedding is normalized (L2 norm ≈ 1)
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);

        Ok(())
    }

    #[test]
    #[ignore] // Requires model download
    fn test_embed_batch() -> Result<()> {
        let model = EmbeddingModel::new()?;
        let embeddings = model.embed_batch(&["Hello", "World", "Test"])?;

        assert_eq!(embeddings.len(), 3);
        for emb in embeddings {
            assert_eq!(emb.len(), 384);
        }

        Ok(())
    }
}
