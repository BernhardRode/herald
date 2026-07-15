//! Herald — an opinionated JMAP CLI for Stalwart Mail Server.
//!
//! Usage:
//!   herald auth login --profile default
//!   herald mail send --to user@example.com --subject "Hello" --body "Hi there"
//!   herald mail mailboxes
//!   herald config show

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod auth;
mod commands;
mod config;
mod jmap;
pub mod secret;
pub mod text;
mod tui;
pub mod validate;

#[derive(Debug, Parser)]
#[command(
    name = "herald",
    version,
    about = "Herald — JMAP CLI for Stalwart Mail Server"
)]
struct Cli {
    /// Load environment variables from a specific .env file
    #[arg(long, global = true)]
    env_file: Option<PathBuf>,

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
    /// Contacts commands (list address books, list contacts)
    #[command(subcommand)]
    Contacts(commands::contacts::ContactsCommand),
    /// Calendar commands (list calendars, list events)
    #[command(subcommand)]
    Calendar(commands::calendar::CalendarCommand),
    /// Configuration management
    #[command(subcommand)]
    Config(commands::config::ConfigCommand),
    /// Launch the interactive TUI
    Tui,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize tracing
    let filter = if cli.verbose {
        EnvFilter::new("herald=debug,herald_jmap_stalwart=debug,jmap_base_client=debug")
    } else {
        EnvFilter::new("herald=info,herald_jmap_stalwart=warn")
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();

    // Load env from explicit path, or from CWD in debug builds only
    load_env(cli.env_file.as_deref());

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

fn load_env(env_file: Option<&std::path::Path>) {
    if let Some(path) = env_file {
        match dotenvy::from_path(path) {
            Ok(_) => tracing::info!("Loaded env from {:?}", path),
            Err(e) => tracing::warn!("Failed to load {:?}: {}", path, e),
        }
    } else {
        // A .env planted in an untrusted CWD could redirect credentials to an
        // attacker's server, so release builds require the explicit --env-file.
        #[cfg(debug_assertions)]
        if let Ok(path) = dotenvy::dotenv() {
            tracing::warn!("Loaded .env from {:?} (dev builds only)", path);
        }
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match &cli.command {
        Command::Config(cmd) => {
            commands::config::handle(cmd).await?;
        }
        Command::Tui => {
            let config = config::Config::resolve()?;
            tui::run(config, cli.profile.as_deref())?;
        }
        // All remaining commands need a resolved profile and connected client
        cmd => {
            let config = config::Config::resolve()?;
            let (profile_name, profile) = config.get_profile_with_name(cli.profile.as_deref())?;
            let client = auth::create_client(profile, profile_name).await?;
            match cmd {
                Command::Auth(cmd) => commands::auth::handle(cmd, &client, profile).await?,
                Command::Mail(cmd) => commands::mail::handle(cmd, &client, profile).await?,
                Command::Contacts(cmd) => commands::contacts::handle(cmd, &client).await?,
                Command::Calendar(cmd) => commands::calendar::handle(cmd, &client).await?,
                Command::Config(_) | Command::Tui => unreachable!("handled above"),
            }
        }
    }
    Ok(())
}
