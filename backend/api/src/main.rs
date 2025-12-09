use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

mod analytics;
mod cli;
mod handlers;
mod job_manager;
mod server;

#[derive(Parser)]
#[command(name = "linggen")]
#[command(about = "Linggen - AI-powered code intelligence", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// API URL for CLI commands
    #[arg(
        long,
        env = "LINGGEN_API_URL",
        default_value = "http://127.0.0.1:8787",
        global = true
    )]
    api_url: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Linggen server (default if no command specified)
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "8787")]
        port: u16,
    },

    /// Check backend status and optionally start it
    Start,

    /// Index a local directory
    Index {
        /// Path to the directory to index (defaults to current directory)
        path: Option<PathBuf>,

        /// Indexing mode: auto, full, or incremental
        #[arg(long, default_value = "auto")]
        mode: String,

        /// Override the default source name
        #[arg(long)]
        name: Option<String>,

        /// Include patterns (glob patterns)
        #[arg(long = "include")]
        include_patterns: Vec<String>,

        /// Exclude patterns (glob patterns)
        #[arg(long = "exclude")]
        exclude_patterns: Vec<String>,

        /// Wait for the indexing job to complete
        #[arg(long)]
        wait: bool,
    },

    /// Show system status and recent jobs
    Status {
        /// Number of recent jobs to show
        #[arg(long, default_value = "10")]
        limit: usize,
    },
}

/// Ensure the Linggen backend is running at the given API URL.
/// If it is not reachable, this will start it in the background
/// and wait until it responds (or time out with an error).
async fn ensure_backend_running(api_url: &str) -> Result<()> {
    let api_client = cli::ApiClient::new(api_url.to_string());

    // Fast path: backend already running
    if api_client.get_status().await.is_ok() {
        return Ok(());
    }

    // Try to derive the port from the URL, fall back to default 8787
    let port = extract_port_from_url(api_url).unwrap_or(8787);

    // Start backend server in the background.
    // We don't await this here because start_server runs the server loop.
    tokio::spawn(async move {
        if let Err(e) = server::start_server(port).await {
            eprintln!("Failed to start Linggen backend on port {}: {}", port, e);
        }
    });

    // Poll until backend becomes available, with a timeout.
    let max_attempts = 30;
    for _ in 0..max_attempts {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if api_client.get_status().await.is_ok() {
            return Ok(());
        }
    }

    Err(anyhow::Error::msg(format!(
        "Timed out waiting for Linggen backend to start at {}",
        api_url
    )))
}

/// Best-effort extraction of a port number from an API URL string.
/// Examples:
/// - "http://127.0.0.1:8787" -> Some(8787)
/// - "http://localhost" -> None
fn extract_port_from_url(api_url: &str) -> Option<u16> {
    // Strip scheme if present
    let without_scheme = if let Some(pos) = api_url.find("://") {
        &api_url[pos + 3..]
    } else {
        api_url
    };

    // For hosts like "127.0.0.1:8787" or "[::1]:8787"
    if let Some((_, port_str)) = without_scheme.rsplit_once(':') {
        port_str.parse().ok()
    } else {
        None
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli_args = Cli::parse();

    match cli_args.command {
        // Default: start server
        None => {
            server::start_server(8787).await?;
        }

        // Explicit serve command
        Some(Commands::Serve { port }) => {
            server::start_server(port).await?;
        }

        // CLI commands
        Some(Commands::Start) => {
            ensure_backend_running(&cli_args.api_url).await?;
            let api_client = cli::ApiClient::new(cli_args.api_url);
            cli::handle_start(&api_client).await?;
        }

        Some(Commands::Index {
            path,
            mode,
            name,
            include_patterns,
            exclude_patterns,
            wait,
        }) => {
            ensure_backend_running(&cli_args.api_url).await?;
            let api_client = cli::ApiClient::new(cli_args.api_url);
            cli::handle_index(
                &api_client,
                path,
                mode,
                name,
                include_patterns,
                exclude_patterns,
                wait,
            )
            .await?;
        }

        Some(Commands::Status { limit }) => {
            ensure_backend_running(&cli_args.api_url).await?;
            let api_client = cli::ApiClient::new(cli_args.api_url);
            cli::handle_status(&api_client, limit).await?;
        }
    }

    Ok(())
}
