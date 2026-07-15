//! `herald mail` subcommands — send, mailboxes, list, read, move,
//! folder-delete, mark-read, mark-unread, delete.

mod list;
mod read;
mod send;
mod watch;

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
    /// Watch a folder and print each incoming email as it arrives
    Watch {
        /// Folder to watch, by name or role (default: Inbox)
        #[arg(long, conflicts_with = "all")]
        folder: Option<String>,
        /// Watch all folders
        #[arg(long)]
        all: bool,
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
    /// Mark an email as read
    MarkRead {
        /// Email ID
        #[arg(long)]
        id: String,
    },
    /// Mark an email as unread
    MarkUnread {
        /// Email ID
        #[arg(long)]
        id: String,
    },
    /// Permanently delete an email
    Delete {
        /// Email ID
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
        MailCommand::Watch { folder, all } => watch::watch(client, folder.as_deref(), *all).await,
        MailCommand::Move { from, to } => handle_move(client, from, to).await,
        MailCommand::FolderDelete { id, force } => handle_folder_delete(client, id, *force).await,
        MailCommand::MarkRead { id } => {
            mail::mark_read(client, id).await?;
            println!("✓ Marked as read: {}", sanitize_display(id));
            Ok(())
        }
        MailCommand::MarkUnread { id } => {
            mail::mark_unread(client, id).await?;
            println!("✓ Marked as unread: {}", sanitize_display(id));
            Ok(())
        }
        MailCommand::Delete { id } => {
            mail::delete_email(client, id).await?;
            println!("✓ Email deleted: {}", sanitize_display(id));
            Ok(())
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
        if force {
            " (force: removing emails)"
        } else {
            ""
        }
    );
    mail::destroy_mailbox(client, mailbox_id, force).await?;
    println!("✓ Mailbox deleted");
    Ok(())
}
