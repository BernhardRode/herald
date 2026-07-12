//! `herald auth` subcommands — login, status.

use clap::Subcommand;
use jmap_base_client::JmapClient;

use crate::config::Profile;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Test authentication by fetching a JMAP session
    Login,
    /// Show current session info
    Status,
}

pub async fn handle(
    cmd: &AuthCommand,
    client: &JmapClient,
    _profile: &Profile,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        AuthCommand::Login => {
            println!("Authenticating...");
            let session = client.fetch_session().await?;
            println!("✓ Login successful!");
            println!("  Username: {}", session.username.expose_unredacted());
            println!("  Accounts: {}", session.accounts.len());
            for (id, info) in &session.accounts {
                println!("    {} — {}", id, info.name.expose_unredacted());
            }
        }
        AuthCommand::Status => match client.fetch_session().await {
            Ok(session) => {
                println!("✓ Authenticated");
                println!("  Username: {}", session.username.expose_unredacted());
                let caps: Vec<&str> = session.capabilities.keys().map(|s| s.as_str()).collect();
                println!("  Capabilities: {}", caps.join(", "));
            }
            Err(e) => {
                println!("✗ Not authenticated: {e}");
            }
        },
    }
    Ok(())
}
