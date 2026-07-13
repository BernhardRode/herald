//! `herald mail` subcommands — send, mailboxes, list, read.

mod list;
mod read;
mod send;

use clap::Subcommand;
use jmap_base_client::JmapClient;

use crate::config::Profile;

#[derive(Debug, Subcommand)]
pub enum MailCommand {
    /// Send an email
    Send {
        /// Recipient email address
        #[arg(long)]
        to: String,
        /// Email subject
        #[arg(long)]
        subject: String,
        /// Email body (plain text)
        #[arg(long)]
        body: String,
        /// From address (overrides profile default)
        #[arg(long)]
        from: Option<String>,
    },
    /// List mailboxes
    Mailboxes,
    /// List recent emails in inbox
    List,
    /// Read a specific email by ID
    Read {
        /// Email ID to read
        #[arg(long)]
        id: String,
    },
}

pub async fn handle(
    cmd: &MailCommand,
    client: &JmapClient,
    profile: &Profile,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        MailCommand::Send {
            to,
            subject,
            body,
            from,
        } => send::send_email(client, profile, to, subject, body, from.as_deref()).await,
        MailCommand::Mailboxes => list::list_mailboxes(client).await,
        MailCommand::List => list::list_emails(client).await,
        MailCommand::Read { id } => read::read_email(client, id).await,
    }
}
