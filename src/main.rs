//! Herald — an opinionated JMAP CLI for Stalwart Mail Server.
//!
//! Usage:
//!   herald auth login --profile default
//!   herald mail send --to user@example.com --subject "Hello" --body "Hi there"
//!   herald mail mailboxes
//!   herald config show

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod auth;
mod commands;
mod config;
mod output;

#[derive(Debug, Parser)]
#[command(
    name = "herald",
    version,
    about = "Herald — JMAP CLI for Stalwart Mail Server"
)]
struct Cli {
    /// Profile name to use (overrides default_profile in config)
    #[arg(long, global = true)]
    profile: Option<String>,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Authentication commands
    #[command(subcommand)]
    Auth(commands::auth::AuthCommand),
    /// Mail commands (send, list mailboxes)
    #[command(subcommand)]
    Mail(commands::mail::MailCommand),
    /// Configuration management
    #[command(subcommand)]
    Config(commands::config::ConfigCommand),
}

#[tokio::main]
async fn main() {
    // Load .env file if present (for local dev)
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    // Initialize tracing
    let filter = if cli.verbose {
        EnvFilter::new("herald=debug,stalwart_rs=debug,jmap_base_client=debug")
    } else {
        EnvFilter::new("herald=info,stalwart_rs=warn")
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();

    if let Err(e) = run(cli).await {
        eprintln!("Error: {e}");
        let mut source: &dyn std::error::Error = e.as_ref();
        while let Some(next) = source.source() {
            eprintln!("  Caused by: {next}");
            source = next;
        }
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match &cli.command {
        Command::Config(cmd) => {
            commands::config::handle(cmd).await?;
        }
        Command::Auth(cmd) => {
            let config = config::Config::resolve()?;
            let profile = config.get_profile(cli.profile.as_deref())?;
            let client = auth::create_client(profile).await?;
            commands::auth::handle(cmd, &client, profile).await?;
        }
        Command::Mail(cmd) => {
            let config = config::Config::resolve()?;
            let profile = config.get_profile(cli.profile.as_deref())?;
            let client = auth::create_client(profile).await?;
            commands::mail::handle(cmd, &client, profile).await?;
        }
    }
    Ok(())
}
