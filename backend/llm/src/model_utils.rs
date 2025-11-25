use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen3::{Config as Qwen3Config, ModelForCausalLM as Qwen3Model};
use std::path::PathBuf;

/// Load Qwen3 model from multiple safetensors files
pub fn load_model(
    model_paths: &[PathBuf],
    config_path: &PathBuf,
    device: &Device,
) -> Result<Qwen3Model> {
    tracing::info!("Loading model config from: {:?}", config_path);

    // Load config
    let config_str = std::fs::read_to_string(config_path).context("Failed to read config file")?;
    let config: Qwen3Config =
        serde_json::from_str(&config_str).context("Failed to parse Qwen3 config")?;

    tracing::info!("Loading model weights from {} files...", model_paths.len());

    // Load model weights from safetensors with BF16 for best Metal performance
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(model_paths, DType::BF16, device)
            .context("Failed to load model from safetensors")?
    };

    tracing::info!("Building Qwen3 model...");
    let model = Qwen3Model::new(&config, vb).context("Failed to build Qwen3 model")?;

    tracing::info!("✓ Model loaded successfully");
    Ok(model)
}

/// Sample next token from logits with repetition penalty
pub fn sample_token(
    logits: &Tensor,
    temperature: f64,
    top_p: f64,
    repeat_penalty: f32,
    previous_tokens: &[u32],
) -> Result<u32> {
    let mut logits = logits.to_vec1::<f32>()?;

    // Apply repetition penalty to previously generated tokens
    if repeat_penalty != 1.0 {
        for &token_id in previous_tokens {
            let idx = token_id as usize;
            if idx < logits.len() {
                // Penalize by dividing if logit is positive, multiplying if negative
                if logits[idx] > 0.0 {
                    logits[idx] /= repeat_penalty;
                } else {
                    logits[idx] *= repeat_penalty;
                }
            }
        }
    }

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
