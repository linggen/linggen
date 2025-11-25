use anyhow::Result;
use hf_hub::api::sync::Api;

fn main() -> Result<()> {
    println!("Initializing API...");
    let api = Api::new()?;
    let repo = api.model("Qwen/Qwen3-4B-Instruct-2507".to_string());

    println!("Attempting to download config.json...");
    match repo.get("config.json") {
        Ok(path) => println!("Success! Path: {:?}", path),
        Err(e) => {
            println!("Error downloading config.json:");
            println!("Debug: {:?}", e);
            println!("Display: {}", e);
        }
    }

    Ok(())
}
