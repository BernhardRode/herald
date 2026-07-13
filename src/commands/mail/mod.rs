//! `herald mail` subcommands — send, mailboxes, list, read, move, folder-delete.

mod list;
mod read;
mod send;

use clap::Subcommand;
use jmap_base_client::JmapClient;

use crate::config::Profile;
use crate::jmap::mail;
use crate::text::sanitize_display;

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
    /// Move all emails from one mailbox to another
    Move {
        /// Source mailbox ID
        #[arg(long)]
        from: String,
        /// Target mailbox ID
        #[arg(long)]
        to: String,
    },
    /// Delete a mailbox (must be empty, or use --force)
    FolderDelete {
        /// Mailbox ID to delete
        #[arg(long)]
        id: String,
        /// Remove remaining emails in the folder before deleting
        #[arg(long)]
        force: bool,
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
        MailCommand::Move { from, to } => handle_move(client, from, to).await,
        MailCommand::FolderDelete { id, force } => {
            handle_folder_delete(client, id, *force).await
        }
    }
}

/// Move all emails from one mailbox to another.
async fn handle_move(
    client: &JmapClient,
    source_id: &str,
    target_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "Querying emails in mailbox {}...",
        sanitize_display(source_id)
    );
    let ids = mail::query_mailbox_emails(client, source_id, 5000).await?;

    if ids.is_empty() {
        println!("No emails to move.");
        return Ok(());
    }

    println!(
        "Moving {} emails from {} → {}...",
        ids.len(),
        sanitize_display(source_id),
        sanitize_display(target_id)
    );
    let moved = mail::move_emails_bulk(client, &ids, source_id, target_id).await?;
    println!("✓ Moved {} emails", moved);
    Ok(())
}

/// Delete a mailbox.
async fn handle_folder_delete(
    client: &JmapClient,
    mailbox_id: &str,
    force: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "Deleting mailbox {}{}...",
        sanitize_display(mailbox_id),
        if force { " (force: removing emails)" } else { "" }
    );
    mail::destroy_mailbox(client, mailbox_id, force).await?;
    println!("✓ Mailbox deleted");
    Ok(())
}
