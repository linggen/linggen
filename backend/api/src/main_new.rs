use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
        /// Path to the directory to index
        path: PathBuf,

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
            let api_client = cli::ApiClient::new(cli_args.api_url);
            cli::handle_status(&api_client, limit).await?;
        }
    }

    Ok(())
}
