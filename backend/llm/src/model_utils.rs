use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen2::{Config as Qwen2Config, Model as Qwen2Model};
use std::path::PathBuf;

/// Load Qwen2 model from safetensors file
pub fn load_model(
    model_path: &PathBuf,
    config_path: &PathBuf,
    device: &Device,
) -> Result<Qwen2Model> {
    println!("Loading model config from: {:?}", config_path);

    // Load config
    let config_str = std::fs::read_to_string(config_path).context("Failed to read config file")?;
    let config: Qwen2Config =
        serde_json::from_str(&config_str).context("Failed to parse Qwen2 config")?;

    println!("Loading model weights from: {:?}", model_path);

    // Load model weights from safetensors
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[model_path.clone()], DType::F32, device)
            .context("Failed to load model from safetensors")?
    };

    println!("Building Qwen2 model...");
    let model = Qwen2Model::new(&config, vb).context("Failed to build Qwen2 model")?;

    println!("✓ Model loaded successfully");
    Ok(model)
}

/// Sample next token from logits
pub fn sample_token(logits: &Tensor, temperature: f64, top_p: f64) -> Result<u32> {
    let logits = logits.to_vec1::<f32>()?;

    if temperature <= 0.0 {
        // Greedy sampling
        let mut best_idx = 0;
        let mut best_logit = logits[0];
        for (idx, &logit) in logits.iter().enumerate().skip(1) {
            if logit > best_logit {
                best_logit = logit;
                best_idx = idx;
            }
        }
        return Ok(best_idx as u32);
    }

    // Apply temperature
    let mut probs: Vec<f32> = logits
        .iter()
        .map(|&l| (l / temperature as f32).exp())
        .collect();

    // Normalize to probabilities
    let sum: f32 = probs.iter().sum();
    for p in probs.iter_mut() {
        *p /= sum;
    }

    // Top-p (nucleus) sampling
    if top_p < 1.0 {
        let mut indexed_probs: Vec<(usize, f32)> =
            probs.iter().enumerate().map(|(i, &p)| (i, p)).collect();

        // Sort by probability (descending), handle NaN
        indexed_probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Find nucleus
        let mut cumsum = 0.0;
        let mut nucleus_size = 0;
        for (_, p) in indexed_probs.iter() {
            cumsum += p;
            nucleus_size += 1;
            if cumsum >= top_p as f32 {
                break;
            }
        }

        // Zero out probabilities outside nucleus
        let nucleus_indices: std::collections::HashSet<usize> = indexed_probs
            .iter()
            .take(nucleus_size)
            .map(|(i, _)| *i)
            .collect();

        for (i, p) in probs.iter_mut().enumerate() {
            if !nucleus_indices.contains(&i) {
                *p = 0.0;
            }
        }

        // Renormalize
        let sum: f32 = probs.iter().sum();
        if sum > 0.0 {
            for p in probs.iter_mut() {
                *p /= sum;
            }
        }
    }

    // Sample from distribution
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let sample: f32 = rng.gen();

    let mut cumsum = 0.0;
    for (idx, &p) in probs.iter().enumerate() {
        cumsum += p;
        if sample <= cumsum {
            return Ok(idx as u32);
        }
    }

    // Fallback
    Ok((probs.len() - 1) as u32)
}
