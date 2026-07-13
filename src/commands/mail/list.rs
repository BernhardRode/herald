//! `herald mail mailboxes` and `herald mail list`.

use jmap_base_client::JmapClient;
use jmap_mail_client::JmapMailExt;
use serde_json::json;

use crate::text::{sanitize_display, truncate_str};

pub async fn list_mailboxes(
    client: &JmapClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session);

    let resp = sc.mailbox_get(None, None).await?;

    println!(
        "{:<12} {:<20} {:<10} {:>8} {:>8}",
        "ID", "Name", "Role", "Total", "Unread"
    );
    println!("{}", "-".repeat(62));
    for m in &resp.list {
        let role = m
            .role
            .as_ref()
            .map(|r| r.to_wire_str().to_string())
            .unwrap_or_else(|| "-".into());
        println!(
            "{:<12} {:<20} {:<10} {:>8} {:>8}",
            m.id.as_ref(),
            sanitize_display(&m.name),
            role,
            m.total_emails,
            m.unread_emails,
        );
    }
    Ok(())
}

pub async fn list_emails(
    client: &JmapClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session.clone());

    // Find the Inbox mailbox ID
    let mailboxes = sc.mailbox_get(None, None).await?;
    let inbox = mailboxes
        .list
        .iter()
        .find(|m| m.role.as_ref().is_some_and(|r| r.to_wire_str() == "inbox"))
        .ok_or("no Inbox mailbox found")?;
    let inbox_id = inbox.id.clone();

    // Query for the 20 most recent emails in Inbox, sorted by receivedAt descending
    let filter = json!({ "inMailbox": inbox_id.as_ref() });
    let sort = json!([{ "property": "receivedAt", "isAscending": false }]);
    let query_resp = sc
        .email_query(Some(filter), Some(sort), Some(0), Some(20), None)
        .await?;

    if query_resp.ids.is_empty() {
        println!("No emails in Inbox.");
        return Ok(());
    }

    // Fetch subject, from, receivedAt for those IDs
    let email_resp = sc
        .email_get(
            Some(&query_resp.ids),
            Some(&["id", "subject", "from", "receivedAt"]),
            None,
        )
        .await?;

    println!("{:<12} {:<30} {:<25} Subject", "ID", "From", "Date");
    println!("{}", "-".repeat(90));
    for email in &email_resp.list {
        let from_str = email
            .from
            .as_ref()
            .and_then(|addrs| addrs.first())
            .map(|a| a.name.as_deref().unwrap_or(&a.email).to_string())
            .unwrap_or_else(|| "(unknown)".into());

        let subject = email.subject.as_deref().unwrap_or("(no subject)");
        let date = email.received_at.as_ref();

        let from_str = sanitize_display(&from_str);
        let subject = sanitize_display(subject);

        println!(
            "{:<12} {:<30} {:<25} {}",
            email.id.as_ref(),
            truncate_str(&from_str, 28),
            truncate_str(date, 23),
            truncate_str(&subject, 50),
        );
    }
    Ok(())
}
