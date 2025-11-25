use anyhow::Result;
use candle_core::{Device, Tensor};
use candle_transformers::models::qwen3::ModelForCausalLM as Qwen3Model;
use std::path::PathBuf;
use tokenizers::Tokenizer;

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging
    // tracing_subscriber::fmt::init();

    // Hardcoded paths - adjust as needed or use auto-download logic if simpler
    // Using the same logic as MiniLLM to find/download if needed would be best,
    // but for a quick debug script, let's try to use the cache paths seen in logs
    let cache_dir = dirs::home_dir().unwrap().join(".cache/huggingface/hub/models--Qwen--Qwen3-4B-Instruct-2507/snapshots/cdbee75f17c01a7cc42f958dc650907174af0554");

    let tokenizer_path = cache_dir.join("tokenizer.json");
    let config_path = cache_dir.join("config.json");
    let model_paths = vec![
        cache_dir.join("model-00001-of-00003.safetensors"),
        cache_dir.join("model-00002-of-00003.safetensors"),
        cache_dir.join("model-00003-of-00003.safetensors"),
    ];

    println!("Loading tokenizer from {:?}", tokenizer_path);
    let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow::anyhow!(e))?;

    println!("Loading model...");
    let device = Device::new_metal(0)?; // Force Metal as that's where the crash is

    // Load config
    let config_str = std::fs::read_to_string(&config_path)?;
    let config: candle_transformers::models::qwen3::Config = serde_json::from_str(&config_str)?;

    // Load weights
    let vb = unsafe {
        candle_nn::VarBuilder::from_mmaped_safetensors(
            &model_paths,
            candle_core::DType::BF16,
            &device,
        )?
    };

    let mut model = Qwen3Model::new(&config, vb)?;

    // Create a dummy input of length ~1675 to reproduce the crash
    let seq_len = 1675;
    println!("Creating input of length {}", seq_len);

    // Create dummy tokens (just repeat token ID 100)
    let tokens = vec![100u32; seq_len];
    let input_ids = Tensor::new(tokens.as_slice(), &device)?.unsqueeze(0)?;

    println!("Running forward pass...");

    // Try the exact call that failed
    // "narrow invalid args start > dim_len: [1, 151936], dim: 0, start: 1675, len:1"
    // This error suggests something is indexing into a [1, 151936] tensor using '1675' on dim 0.
    // [1, 151936] is typically [batch, vocab] (logits for ONE token).
    // If we pass a sequence of 1675 tokens, we expect [1, 1675, 151936] logits.
    // If the model implementation does something specific with start_pos=0 vs start_pos=seq_len...

    match model.forward(&input_ids, 0) {
        Ok(logits) => {
            println!("Success! Logits shape: {:?}", logits.dims());

            // Simulate MiniLLM post-processing
            use candle_core::IndexOp;
            let seq_len = input_ids.dim(1)?;
            println!("seq_len: {}", seq_len);

            // This is what MiniLLM does:
            // let logits = logits.i((0, seq_len - 1))?;

            match logits.i((0, seq_len - 1)) {
                Ok(last_token_logits) => println!(
                    "Successfully extracted last token logits: {:?}",
                    last_token_logits.dims()
                ),
                Err(e) => println!("Failed to extract last token logits: {}", e),
            }
        }
        Err(e) => {
            println!("Crash reproduced: {}", e);
        }
    }

    Ok(())
}
