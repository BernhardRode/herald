//! `herald mail` subcommands — send, mailboxes.

use std::collections::HashMap;

use clap::Subcommand;
use jmap_base_client::{JmapClient, UploadBlobParams};
use jmap_mail_client::{EmailImportInput, JmapMailExt};
use jmap_types::Id;
use mail_builder::MessageBuilder;
use serde_json::json;

use crate::config::Profile;
use crate::sanitize::sanitize_display;
use crate::validate::validate_header_value;

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
        } => {
            send_email(client, profile, to, subject, body, from.as_deref()).await?;
        }
        MailCommand::Mailboxes => {
            list_mailboxes(client).await?;
        }
        MailCommand::List => {
            list_emails(client).await?;
        }
        MailCommand::Read { id } => {
            read_email(client, id).await?;
        }
    }
    Ok(())
}

async fn send_email(
    client: &JmapClient,
    profile: &Profile,
    to: &str,
    subject: &str,
    body: &str,
    from_override: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session.clone());

    // Determine the sender address
    let from_email = from_override
        .map(|s| s.to_string())
        .or_else(|| profile.from_email.clone())
        .ok_or("no 'from' address specified — use --from or set from_email in profile")?;

    let from_name = profile.from_name.clone().unwrap_or_default();

    // Validate header values against injection (CR/LF)
    validate_header_value("to", to)?;
    validate_header_value("subject", subject)?;
    validate_header_value("from", &from_email)?;
    if !from_name.is_empty() {
        validate_header_value("from_name", &from_name)?;
    }

    // Step 1: Find the identity that matches our from address
    let identities = sc.identity_get(None, None).await?;
    let identity = identities
        .list
        .iter()
        .find(|id| id.email == from_email)
        .or(identities.list.first())
        .ok_or("no identities configured on this account")?;

    let identity_id = identity.id.clone();
    tracing::info!("Using identity: {} <{}>", identity.name, identity.email);

    // Step 2: Build RFC 5322 message using mail-builder (proper RFC 2047 encoding)
    let msg_id = generate_message_id(&from_email);
    // Strip angle brackets for mail-builder (it adds them automatically)
    let msg_id_bare = msg_id.trim_start_matches('<').trim_end_matches('>');

    let message = if from_name.is_empty() {
        MessageBuilder::new()
            .from(from_email.as_str())
            .to(to)
            .subject(subject)
            .message_id(msg_id_bare)
            .text_body(body)
            .write_to_vec()?
    } else {
        MessageBuilder::new()
            .from((from_name.as_str(), from_email.as_str()))
            .to(to)
            .subject(subject)
            .message_id(msg_id_bare)
            .text_body(body)
            .write_to_vec()?
    };

    // Step 3: Upload blob
    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:mail")
        .ok_or("no primary mail account in session")?;

    let blob_resp = client
        .upload_blob(UploadBlobParams {
            upload_url_template: &session.upload_url,
            account_id,
            content_type: "message/rfc822",
            data: bytes::Bytes::from(message),
        })
        .await?;

    let blob_id = blob_resp.blob_id;
    tracing::info!("Uploaded message blob: {}", blob_id.as_ref());

    // Step 4: Find Sent mailbox
    let mailboxes = sc.mailbox_get(None, None).await?;
    let sent_box = mailboxes
        .list
        .iter()
        .find(|m| m.role.as_ref().is_some_and(|r| r.to_wire_str() == "sent"))
        .or_else(|| {
            mailboxes
                .list
                .iter()
                .find(|m| m.role.as_ref().is_some_and(|r| r.to_wire_str() == "drafts"))
        })
        .or(mailboxes.list.first())
        .ok_or("no mailboxes available")?;

    let mailbox_ids: Vec<Id> = vec![sent_box.id.clone()];

    // Step 5: Email/import — create the Email object from the blob
    let mut emails_map: HashMap<String, EmailImportInput<'_>> = HashMap::new();
    emails_map.insert(
        "draft1".to_string(),
        EmailImportInput {
            blob_id: &blob_id,
            mailbox_ids: &mailbox_ids,
            keywords: Some(&["$seen"]),
            received_at: None,
            extra: serde_json::Map::new(),
        },
    );

    let import_resp = sc.email_import(&emails_map, None).await?;

    // Get the created email ID
    let created = import_resp
        .created
        .as_ref()
        .and_then(|c| c.get("draft1"))
        .ok_or_else(|| {
            let err_msg = import_resp
                .not_created
                .as_ref()
                .and_then(|nc| nc.get("draft1"))
                .map(|e| format!("{:?}", e))
                .unwrap_or_else(|| "unknown error".to_string());
            format!("Email/import failed: {err_msg}")
        })?;

    let email_id = &created.id;
    tracing::info!("Created email: {}", email_id.as_ref());

    // Step 6: EmailSubmission/set — send it
    // We build a raw JmapRequest because Stalwart returns partial objects in
    // the `created` map which don't fully deserialize into EmailSubmission.
    let submission_args = json!({
        "accountId": account_id,
        "create": {
            "send1": {
                "identityId": identity_id.as_ref(),
                "emailId": email_id.as_ref()
            }
        },
        "onSuccessUpdateEmail": {
            "#send1": {
                "keywords/$draft": null
            }
        }
    });

    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:mail".to_string(),
            "urn:ietf:params:jmap:submission".to_string(),
        ],
        vec![(
            "EmailSubmission/set".to_string(),
            submission_args,
            "r1".to_string(),
        )],
        None,
    );

    let resp = client.call(session.api_url.as_str(), &request).await?;

    // Check the response for errors
    for (method_name, result, _call_id) in &resp.method_responses {
        if method_name == "error" {
            let error_type = result["type"].as_str().unwrap_or("unknown");
            let description = result["description"].as_str().unwrap_or("");
            return Err(format!("JMAP error: {error_type} — {description}").into());
        }
        if method_name == "EmailSubmission/set" {
            if let Some(not_created) = result["notCreated"].as_object() {
                if let Some((key, err)) = not_created.iter().next() {
                    let err_type = err["type"].as_str().unwrap_or("unknown");
                    let err_desc = err["description"].as_str().unwrap_or("");
                    return Err(format!(
                        "EmailSubmission failed for {key}: {err_type} — {err_desc}"
                    )
                    .into());
                }
            }
        }
    }

    println!("✓ Email sent successfully!");
    println!("  To: {}", sanitize_display(to));
    println!("  Subject: {}", sanitize_display(subject));

    Ok(())
}

async fn list_mailboxes(
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
            m.name,
            role,
            m.total_emails,
            m.unread_emails,
        );
    }
    Ok(())
}

async fn list_emails(client: &JmapClient) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

async fn read_email(
    client: &JmapClient,
    id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session);

    let email_id = Id::from(id);
    let ids = [email_id];

    // Fetch the email with body values
    let params = jmap_mail_client::EmailGetParams {
        fetch_text_body_values: Some(true),
        max_body_value_bytes: Some(4096),
        ..Default::default()
    };
    let resp = sc
        .email_get(
            Some(&ids),
            Some(&[
                "id",
                "subject",
                "from",
                "to",
                "sentAt",
                "receivedAt",
                "textBody",
                "bodyValues",
                "preview",
            ]),
            Some(params),
        )
        .await?;

    let email = resp
        .list
        .first()
        .ok_or_else(|| format!("email not found: {id}"))?;

    // Display header
    let subject = email.subject.as_deref().unwrap_or("(no subject)");
    let from_str = email
        .from
        .as_ref()
        .and_then(|addrs| addrs.first())
        .map(|a| match &a.name {
            Some(name) => format!("{name} <{}>", a.email),
            None => a.email.clone(),
        })
        .unwrap_or_else(|| "(unknown)".into());

    let to_str = email
        .to
        .as_ref()
        .map(|addrs| {
            addrs
                .iter()
                .map(|a| match &a.name {
                    Some(name) => format!("{name} <{}>", a.email),
                    None => a.email.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "(unknown)".into());

    let date = email
        .sent_at
        .as_ref()
        .map(|d| d.as_ref().to_string())
        .unwrap_or_else(|| email.received_at.as_ref().to_string());

    println!("Subject: {}", sanitize_display(subject));
    println!("From:    {}", sanitize_display(&from_str));
    println!("To:      {}", sanitize_display(&to_str));
    println!("Date:    {date}");
    println!("{}", "-".repeat(60));

    // Display body: try textBody bodyValues first, fall back to preview
    let body_text = email
        .text_body
        .first()
        .and_then(|part| part.part_id.as_ref())
        .and_then(|part_id| email.body_values.get(part_id))
        .map(|bv| bv.value.as_str());

    if let Some(text) = body_text {
        println!("{}", sanitize_display(text));
    } else if let Some(preview) = &email.preview {
        println!("[Preview] {}", sanitize_display(preview));
    } else {
        println!("(no text body available)");
    }

    Ok(())
}

/// Truncate a string to `max_len` characters, appending "…" if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        truncated.push('…');
        truncated
    }
}

/// Generate a Message-ID using the from-address domain.
fn generate_message_id(from_email: &str) -> String {
    let domain = from_email
        .rsplit_once('@')
        .map(|(_, d)| d)
        .unwrap_or("localhost");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let random: u64 = rand::random();
    format!("<{timestamp}.{random:016x}@{domain}>")
}
