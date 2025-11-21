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
}

impl EmbeddingModel {
    /// Load the all-MiniLM-L6-v2 model from HuggingFace
    pub fn new() -> Result<Self> {
        info!("Loading embedding model: sentence-transformers/all-MiniLM-L6-v2");

        // Determine device (CPU for now, could add Metal/CUDA support)
        let device = Device::Cpu;
        info!("Using device: {:?}", device);

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
        })
    }

    /// Embed a single text into a vector
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_batch(&[text])
            .map(|mut batch| batch.pop().unwrap_or_default())
    }

    /// Embed multiple texts in a batch
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Tokenize
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

        let mut embeddings = Vec::new();

        for encoding in encodings {
            let tokens = encoding.get_ids();
            let token_ids = Tensor::new(tokens, &self.device)?.unsqueeze(0)?;

            // Forward pass (no attention mask needed for our use case)
            let outputs = self.model.forward(&token_ids, &token_ids, None)?;

            // Mean pooling
            let embedding = self.mean_pooling(&outputs)?;

            // Normalize
            let embedding = self.normalize(&embedding)?;

            // Convert to Vec<f32>
            let embedding_vec = embedding.to_vec1::<f32>()?;
            embeddings.push(embedding_vec);
        }

        Ok(embeddings)
    }

    /// Mean pooling over token embeddings
    fn mean_pooling(&self, tensor: &Tensor) -> Result<Tensor> {
        // tensor shape: [batch_size, seq_len, hidden_size]
        // Mean over seq_len dimension
        let pooled = tensor.mean(1)?;
        Ok(pooled)
    }

    /// L2 normalize embeddings
    fn normalize(&self, tensor: &Tensor) -> Result<Tensor> {
        let norm = tensor.sqr()?.sum_keepdim(1)?.sqrt()?;
        Ok(tensor.broadcast_div(&norm)?)
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
