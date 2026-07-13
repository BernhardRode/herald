//! `herald mail send` — resolve the sender from the profile and send.

use jmap_base_client::JmapClient;

use crate::config::Profile;
use crate::jmap::mail::{send_message, OutgoingMail};
use crate::text::sanitize_display;

pub async fn send_email(
    client: &JmapClient,
    profile: &Profile,
    to: &str,
    subject: &str,
    body: &str,
    from_override: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let from_email = from_override
        .map(|s| s.to_string())
        .or_else(|| profile.from_email.clone())
        .ok_or("no 'from' address specified — use --from or set from_email in profile")?;
    let from_name = profile.from_name.clone().unwrap_or_default();

    send_message(
        client,
        &OutgoingMail {
            from_email: &from_email,
            from_name: &from_name,
            to,
            cc: "",
            bcc: "",
            subject,
            body,
        },
        None, // CLI doesn't resolve folder overrides; server role is used
    )
    .await?;

    println!("✓ Email sent successfully!");
    println!("  To: {}", sanitize_display(to));
    println!("  Subject: {}", sanitize_display(subject));
    Ok(())
}
