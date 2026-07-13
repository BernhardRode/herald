//! Mail operations: send a message, move a message between mailboxes.

use std::collections::HashMap;

use jmap_base_client::{JmapClient, UploadBlobParams};
use jmap_mail_client::{EmailImportInput, JmapMailExt};
use jmap_types::Id;
use mail_builder::MessageBuilder;
use serde_json::json;

use super::{check_set_response, JmapResult};
use crate::validate::validate_header_value;

/// An outgoing message. `cc`/`bcc` may be empty; addresses are comma-separated.
pub struct OutgoingMail<'a> {
    pub from_email: &'a str,
    pub from_name: &'a str,
    pub to: &'a str,
    pub cc: &'a str,
    pub bcc: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
}

/// Send an email: build RFC 5322 via mail-builder, upload, import, submit.
///
/// If `sent_mailbox_id` is provided, the imported message is filed there
/// instead of the server's role-tagged sent folder.
pub async fn send_message(
    client: &JmapClient,
    mail: &OutgoingMail<'_>,
    sent_mailbox_id: Option<&str>,
) -> JmapResult<()> {
    // Validate all header values against CR/LF injection
    validate_header_value("to", mail.to)?;
    validate_header_value("cc", mail.cc)?;
    validate_header_value("bcc", mail.bcc)?;
    validate_header_value("subject", mail.subject)?;
    validate_header_value("from", mail.from_email)?;
    validate_header_value("from_name", mail.from_name)?;

    if mail.to.trim().is_empty() {
        return Err("no recipient — set the To field".into());
    }

    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session.clone());

    // Find the identity that matches our from address
    let identities = sc.identity_get(None, None).await?;
    let identity = identities
        .list
        .iter()
        .find(|id| id.email == mail.from_email)
        .or(identities.list.first())
        .ok_or("no identities configured on this account")?;
    let identity_id = identity.id.clone();
    tracing::info!("Using identity: {} <{}>", identity.name, identity.email);

    // Build the RFC 5322 message (mail-builder handles RFC 2047 encoding)
    let msg_id = generate_message_id(mail.from_email);
    let mut builder = MessageBuilder::new()
        .to(split_addresses(mail.to))
        .subject(mail.subject)
        .message_id(msg_id.as_str())
        .text_body(mail.body);
    builder = if mail.from_name.is_empty() {
        builder.from(mail.from_email)
    } else {
        builder.from((mail.from_name, mail.from_email))
    };
    if !mail.cc.trim().is_empty() {
        builder = builder.cc(split_addresses(mail.cc));
    }
    if !mail.bcc.trim().is_empty() {
        builder = builder.bcc(split_addresses(mail.bcc));
    }
    let message = builder.write_to_vec()?;

    // Upload the blob
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

    // Import into Sent folder (config override > role > drafts > any)
    let mailboxes = sc.mailbox_get(None, None).await?;
    let sent_box = if let Some(override_id) = sent_mailbox_id {
        mailboxes
            .list
            .iter()
            .find(|m| m.id.as_ref() == override_id)
    } else {
        None
    }
    .or_else(|| {
        mailboxes
            .list
            .iter()
            .find(|m| m.role.as_ref().is_some_and(|r| r.to_wire_str() == "sent"))
    })
    .or_else(|| {
        mailboxes
            .list
            .iter()
            .find(|m| m.role.as_ref().is_some_and(|r| r.to_wire_str() == "drafts"))
    })
    .or(mailboxes.list.first())
    .ok_or("no mailboxes available")?;
    let mailbox_ids: Vec<Id> = vec![sent_box.id.clone()];

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

    // EmailSubmission/set — send it. Raw request because Stalwart returns
    // partial objects in `created` that don't deserialize into EmailSubmission.
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
    check_set_response(&resp, "EmailSubmission/set", "notCreated")
}

/// Move an email between mailboxes (Email/set with a mailboxIds patch).
pub async fn move_email(
    client: &JmapClient,
    email_id: &str,
    source_mailbox_id: &str,
    target_mailbox_id: &str,
) -> JmapResult<()> {
    let session = client.fetch_session().await?;
    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:mail")
        .ok_or("no primary mail account in session")?;

    let update_patch = json!({
        format!("mailboxIds/{source_mailbox_id}"): null,
        format!("mailboxIds/{target_mailbox_id}"): true
    });
    let request_args = json!({
        "accountId": account_id,
        "update": { email_id: update_patch }
    });
    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:mail".to_string(),
        ],
        vec![("Email/set".to_string(), request_args, "move1".to_string())],
        None,
    );
    let resp = client.call(session.api_url.as_str(), &request).await?;
    check_set_response(&resp, "Email/set", "notUpdated")
}

/// Split a comma-separated address list into trimmed parts.
fn split_addresses(s: &str) -> Vec<&str> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

/// Generate a Message-ID using the from-address domain (no angle brackets —
/// mail-builder adds them).
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
    format!("{timestamp}.{random:016x}@{domain}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_id_uses_from_domain() {
        let id = generate_message_id("alice@example.com");
        assert!(id.ends_with("@example.com"));
        assert!(!id.contains('<') && !id.contains('>'));
    }

    #[test]
    fn split_addresses_trims_and_skips_empty() {
        assert_eq!(
            split_addresses("a@b.c, d@e.f ,, g@h.i"),
            vec!["a@b.c", "d@e.f", "g@h.i"]
        );
    }
}
