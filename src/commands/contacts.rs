//! `herald contacts` subcommands — list address books, list contacts, and
//! create, update, or delete contacts.

use clap::Subcommand;
use jmap_base_client::JmapClient;
use jmap_contacts_client::JmapContactsExt;

use crate::jmap::contacts::{self, extract_contact_name, extract_first_email};
use crate::text::sanitize_display;

#[derive(Debug, Subcommand)]
pub enum ContactsCommand {
    /// List address books
    Books,
    /// List contacts
    List {
        /// Maximum number of contacts to display
        #[arg(long, default_value = "100")]
        limit: u32,
        /// Fetch all contacts (no limit)
        #[arg(long)]
        all: bool,
    },
    /// Create a contact
    Create {
        /// Full name
        #[arg(long)]
        name: String,
        /// Email address
        #[arg(long)]
        email: Option<String>,
        /// Phone number
        #[arg(long)]
        phone: Option<String>,
    },
    /// Update a contact's name, email, or phone
    Update {
        /// Contact ID
        #[arg(long)]
        id: String,
        /// Full name
        #[arg(long)]
        name: String,
        /// Email address (empty clears it)
        #[arg(long, default_value = "")]
        email: String,
        /// Phone number (empty clears it)
        #[arg(long, default_value = "")]
        phone: String,
    },
    /// Delete a contact
    Delete {
        /// Contact ID
        #[arg(long)]
        id: String,
    },
}

pub async fn handle(
    cmd: &ContactsCommand,
    client: &JmapClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        ContactsCommand::Books => list_address_books(client).await?,
        ContactsCommand::List { limit, all } => list_contacts(client, *limit, *all).await?,
        ContactsCommand::Create { name, email, phone } => {
            contacts::create_contact(
                client,
                name,
                email.as_deref().unwrap_or(""),
                phone.as_deref().unwrap_or(""),
            )
            .await?;
            println!("✓ Contact created: {}", sanitize_display(name));
        }
        ContactsCommand::Update {
            id,
            name,
            email,
            phone,
        } => {
            contacts::update_contact(client, id, name, email, phone).await?;
            println!("✓ Contact updated: {}", sanitize_display(name));
        }
        ContactsCommand::Delete { id } => {
            contacts::delete_contact(client, id).await?;
            println!("✓ Contact deleted: {}", sanitize_display(id));
        }
    }
    Ok(())
}

async fn list_address_books(
    client: &JmapClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_contacts_session(session);
    let resp = sc.address_book_get(None, None).await?;

    println!("{:<12} {:<30} Name", "ID", "Description");
    println!("{}", "-".repeat(60));
    for book in &resp.list {
        let id = book.id.as_ref();
        let name = sanitize_display(&book.name);
        let desc = sanitize_display(book.description.as_deref().unwrap_or(""));
        println!("{:<12} {:<30} {}", id, desc, name);
    }
    Ok(())
}

async fn list_contacts(
    client: &JmapClient,
    limit: u32,
    all: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_contacts_session(session);

    // Use ContactCard/query with limit to get IDs, then fetch those cards.
    // When --all, paginate through all results without a limit cap.
    let cards = if all {
        // Paginate through all contacts
        let mut all_cards = Vec::new();
        let mut position: u64 = 0;
        let page_size: u64 = 500;
        loop {
            let query_resp = sc
                .contact_card_query(None, None, Some(position), Some(page_size))
                .await?;
            if query_resp.ids.is_empty() {
                break;
            }
            let resp = sc
                .contact_card_get(Some(&query_resp.ids), Some(&["id", "name", "emails"]))
                .await?;
            all_cards.extend(resp.list);
            // If we got fewer than page_size, we're done
            if (query_resp.ids.len() as u64) < page_size {
                break;
            }
            position += query_resp.ids.len() as u64;
        }
        all_cards
    } else {
        // Bounded query with the specified limit
        let query_resp = sc
            .contact_card_query(None, None, None, Some(limit as u64))
            .await?;
        if query_resp.ids.is_empty() {
            Vec::new()
        } else {
            let resp = sc
                .contact_card_get(Some(&query_resp.ids), Some(&["id", "name", "emails"]))
                .await?;
            resp.list
        }
    };

    println!("{:<12} {:<30} Email", "ID", "Name");
    println!("{}", "-".repeat(60));
    for card in &cards {
        let id = card.id.as_ref().map(|i| i.as_ref()).unwrap_or("-");

        // Extract full name from the JSContact Name object and sanitize
        let name = sanitize_display(&extract_contact_name(card));

        // Extract first email and sanitize
        let email = sanitize_display(&extract_first_email(card));

        println!("{:<12} {:<30} {}", id, name, email);
    }
    Ok(())
}
