//! `herald contacts` subcommands — list address books, list contacts.

use clap::Subcommand;
use jmap_base_client::JmapClient;
use jmap_contacts_client::JmapContactsExt;

use crate::sanitize::sanitize_display;

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
}

pub async fn handle(
    cmd: &ContactsCommand,
    client: &JmapClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        ContactsCommand::Books => list_address_books(client).await?,
        ContactsCommand::List { limit, all } => list_contacts(client, *limit, *all).await?,
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

/// Extract a display name from a ContactCard's `name` field (JSContact Name object).
fn extract_contact_name(card: &jmap_contacts_types::ContactCard) -> String {
    if let Some(name_val) = &card.name {
        // JSContact Name object has "full" or "given"/"surname" components
        if let Some(full) = name_val.get("full") {
            if let Some(s) = full.as_str() {
                return s.to_string();
            }
        }
        // Try components array
        if let Some(components) = name_val.get("components") {
            if let Some(arr) = components.as_array() {
                let parts: Vec<&str> = arr
                    .iter()
                    .filter_map(|c| c.get("value").and_then(|v| v.as_str()))
                    .collect();
                if !parts.is_empty() {
                    return parts.join(" ");
                }
            }
        }
    }
    "(no name)".to_string()
}

/// Extract the first email address from a ContactCard's `emails` map.
fn extract_first_email(card: &jmap_contacts_types::ContactCard) -> String {
    if let Some(emails_val) = &card.emails {
        if let Some(obj) = emails_val.as_object() {
            for (_key, email_obj) in obj {
                if let Some(addr) = email_obj.get("address").and_then(|v| v.as_str()) {
                    return addr.to_string();
                }
            }
        }
    }
    "".to_string()
}
