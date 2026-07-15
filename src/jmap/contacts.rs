//! Contact operations and JSContact field extraction.

use jmap_base_client::JmapClient;
use jmap_contacts_client::JmapContactsExt;
use jmap_contacts_types::ContactCard;
use serde_json::json;

use super::JmapResult;

/// Extract a display name from a ContactCard's `name` field (JSContact Name object).
pub fn extract_contact_name(card: &ContactCard) -> String {
    if let Some(name_val) = &card.name {
        if let Some(full) = name_val.get("full").and_then(|v| v.as_str()) {
            return full.to_string();
        }
        if let Some(arr) = name_val.get("components").and_then(|c| c.as_array()) {
            let parts: Vec<&str> = arr
                .iter()
                .filter_map(|c| c.get("value").and_then(|v| v.as_str()))
                .collect();
            if !parts.is_empty() {
                return parts.join(" ");
            }
        }
    }
    "(no name)".to_string()
}

/// Extract the first email address from a ContactCard's `emails` map.
pub fn extract_first_email(card: &ContactCard) -> String {
    extract_first_property(card.emails.as_ref(), "address")
}

/// Extract the first phone number from a ContactCard's `phones` map.
pub fn extract_first_phone(card: &ContactCard) -> String {
    extract_first_property(card.phones.as_ref(), "number")
}

fn extract_first_property(map: Option<&serde_json::Value>, key: &str) -> String {
    if let Some(obj) = map.and_then(|v| v.as_object()) {
        for entry in obj.values() {
            if let Some(value) = entry.get(key).and_then(|v| v.as_str()) {
                return value.to_string();
            }
        }
    }
    String::new()
}

/// Create a contact card. Empty email/phone are omitted from the card.
pub async fn create_contact(
    client: &JmapClient,
    name: &str,
    email: &str,
    phone: &str,
) -> JmapResult<()> {
    let session = client.fetch_session().await?;
    let sc = client.with_contacts_session(session);

    let mut card = serde_json::Map::new();
    card.insert("name".into(), json!({ "full": name }));
    if !email.trim().is_empty() {
        card.insert(
            "emails".into(),
            json!({ "e1": { "address": email.trim() } }),
        );
    }
    if !phone.trim().is_empty() {
        card.insert("phones".into(), json!({ "p1": { "number": phone.trim() } }));
    }

    let create = json!({ "new1": card });
    let resp = sc.contact_card_set(Some(create), None, None).await?;
    if let Some(not_created) = &resp.not_created {
        if let Some((_key, err)) = not_created.iter().next() {
            return Err(format!("Contact creation failed: {:?}", err).into());
        }
    }
    Ok(())
}

/// Update a contact card in place: name, first email, first phone.
///
/// Empty email/phone remove the respective property from the card.
#[allow(dead_code)]
pub async fn update_contact(
    client: &JmapClient,
    contact_id: &str,
    name: &str,
    email: &str,
    phone: &str,
) -> JmapResult<()> {
    use jmap_types::PatchObject;

    let session = client.fetch_session().await?;
    let sc = client.with_contacts_session(session);

    let mut patch = serde_json::Map::new();
    patch.insert("name".into(), json!({ "full": name }));
    patch.insert(
        "emails".into(),
        if email.trim().is_empty() {
            serde_json::Value::Null
        } else {
            json!({ "e1": { "address": email.trim() } })
        },
    );
    patch.insert(
        "phones".into(),
        if phone.trim().is_empty() {
            serde_json::Value::Null
        } else {
            json!({ "p1": { "number": phone.trim() } })
        },
    );

    let mut update = std::collections::HashMap::new();
    update.insert(
        jmap_types::Id::from(contact_id),
        PatchObject::from_map(patch),
    );

    let resp = sc.contact_card_set(None, Some(update), None).await?;
    if let Some(not_updated) = &resp.not_updated {
        if let Some((_key, err)) = not_updated.iter().next() {
            return Err(format!("Contact update failed: {:?}", err).into());
        }
    }
    Ok(())
}

/// Delete a contact card by id.
pub async fn delete_contact(client: &JmapClient, contact_id: &str) -> JmapResult<()> {
    let session = client.fetch_session().await?;
    let sc = client.with_contacts_session(session);

    let destroy = vec![jmap_types::Id::from(contact_id)];
    let resp = sc.contact_card_set(None, None, Some(destroy)).await?;
    if let Some(not_destroyed) = &resp.not_destroyed {
        if let Some((_key, err)) = not_destroyed.iter().next() {
            return Err(format!("Contact deletion failed: {:?}", err).into());
        }
    }
    Ok(())
}
