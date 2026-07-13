//! `herald mail read` — display a single email.

use jmap_base_client::JmapClient;
use jmap_mail_client::JmapMailExt;
use jmap_types::Id;

use crate::text::sanitize_display;

pub async fn read_email(
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

    let subject = email.subject.as_deref().unwrap_or("(no subject)");
    let from_str = email
        .from
        .as_ref()
        .and_then(|addrs| addrs.first())
        .map(|a| format_address(a.name.as_deref(), &a.email))
        .unwrap_or_else(|| "(unknown)".into());

    let to_str = email
        .to
        .as_ref()
        .map(|addrs| {
            addrs
                .iter()
                .map(|a| format_address(a.name.as_deref(), &a.email))
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
    println!("Date:    {}", sanitize_display(&date));
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

fn format_address(name: Option<&str>, email: &str) -> String {
    match name {
        Some(name) => format!("{name} <{email}>"),
        None => email.to_string(),
    }
}
