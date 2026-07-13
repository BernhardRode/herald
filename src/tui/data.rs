//! JMAP data access for the TUI: connect, fetch per panel, execute pending ops.

use jmap_base_client::JmapClient;
use jmap_mail_client::JmapMailExt;
use serde_json::json;

use crate::auth;
use crate::jmap::{self, JmapResult};

use super::entries::{CalendarEventEntry, ContactEntry, FolderEntry, MailEntry};
use super::state::{App, Panel, PendingAction};

/// Resolve a folder ID using the standard resolution order:
/// defaults (name) → role-tagged → config override.
///
/// Returns `Some(id)` if found, `None` otherwise.
fn resolve_folder_id(
    folders: &[FolderEntry],
    config_override: Option<&str>,
    role: &str,
    default_name: &str,
) -> Option<String> {
    if let Some(configured) = config_override {
        // Highest priority: explicit config name/path
        resolve_folder_path(folders, configured)
    } else if let Some(f) = folders.iter().find(|f| f.role.as_deref() == Some(role)) {
        // Middle: JMAP role
        Some(f.id.clone())
    } else {
        // Lowest: hardcoded default name
        resolve_folder_path(folders, default_name)
    }
}

/// Resolve a folder by name or path (e.g. "Archive" or "Archive/2026").
fn resolve_folder_path(folders: &[FolderEntry], path: &str) -> Option<String> {
    if !path.contains('/') {
        return folders.iter().find(|f| f.name == path).map(|f| f.id.clone());
    }
    let segments: Vec<&str> = path.split('/').collect();
    let mut current_parent_id: Option<String> = None;
    for (i, segment) in segments.iter().enumerate() {
        let f = folders.iter().find(|f| {
            f.name == *segment && f.parent_id.as_deref() == current_parent_id.as_deref()
        })?;
        if i == segments.len() - 1 {
            return Some(f.id.clone());
        }
        current_parent_id = Some(f.id.clone());
    }
    None
}

/// Load data from JMAP for the current panel.
pub async fn load_data_for_panel(app: &mut App) {
    app.loading = true;

    if app.panel != Panel::Profiles && app.client.is_none() {
        if let Err(e) = connect_profile(app).await {
            app.status_message = Some(format!("Auth error: {e}"));
            app.loading = false;
            return;
        }
    }

    match app.panel {
        Panel::Profiles => {
            // Profiles come from config; nothing to fetch
        }
        Panel::Folders => {
            if let Some(client) = &app.client {
                match fetch_folders(client).await {
                    Ok(mut folders) => {
                        tag_action_folders(&mut folders, app);
                        app.folders = folders;
                    }
                    Err(e) => app.status_message = Some(format!("Mailbox error: {e}")),
                }
            }
        }
        Panel::Mails => {
            // If no folder selected, find the inbox
            if app.active_folder_id.is_none() {
                if let Some(client) = &app.client {
                    if let Ok(mut folders) = fetch_folders(client).await {
                        if let Some(inbox) =
                            folders.iter().find(|f| f.role.as_deref() == Some("inbox"))
                        {
                            app.active_folder_id = Some(inbox.id.clone());
                            app.active_folder_name = inbox.name.clone();
                        } else if let Some(first) = folders.first() {
                            app.active_folder_id = Some(first.id.clone());
                            app.active_folder_name = first.name.clone();
                        }
                        tag_action_folders(&mut folders, app);
                        app.folders = folders;
                    }
                }
            }
            if let (Some(client), Some(folder_id)) = (&app.client, &app.active_folder_id) {
                match fetch_mails(client, folder_id).await {
                    Ok(mails) => app.mails = mails,
                    Err(e) => app.status_message = Some(format!("Mail error: {e}")),
                }
            }
        }
        Panel::Contacts => {
            if let Some(client) = &app.client {
                match fetch_contacts(client).await {
                    Ok(contacts) => app.contacts = contacts,
                    Err(e) => app.status_message = Some(format!("Contacts error: {e}")),
                }
            }
        }
        Panel::Calendar => {
            if let Some(client) = &app.client {
                match fetch_events(client).await {
                    Ok(events) => app.events = events,
                    Err(e) => app.status_message = Some(format!("Calendar error: {e}")),
                }
            }
        }
    }

    app.loading = false;
}

/// Reload the mail list based on the current search mode (all-folder or single-folder).
pub async fn load_search_mails(app: &mut App) {
    app.loading = true;

    if app.client.is_none() {
        app.loading = false;
        return;
    }

    if app.search_all_folders {
        // Ensure we have the folder list for name resolution
        if app.folders.is_empty() {
            if let Some(client) = &app.client {
                if let Ok(folders) = fetch_folders(client).await {
                    app.folders = folders;
                }
            }
        }
        if let Some(client) = &app.client {
            match fetch_all_mails(client, &app.folders).await {
                Ok(mails) => app.mails = mails,
                Err(e) => app.status_message = Some(format!("Search error: {e}")),
            }
        }
    } else {
        // Revert to single-folder view
        if let (Some(client), Some(folder_id)) = (&app.client, &app.active_folder_id) {
            match fetch_mails(client, folder_id).await {
                Ok(mails) => app.mails = mails,
                Err(e) => app.status_message = Some(format!("Mail error: {e}")),
            }
        }
    }

    app.loading = false;
}

/// Execute all queued server-side operations, reporting into the status bar.
pub async fn execute_pending(app: &mut App) {
    let actions: Vec<PendingAction> = app.pending.drain(..).collect();
    for action in actions {
        let result = execute_one(app, &action).await;
        app.status_message = Some(match result {
            Ok(msg) => msg,
            Err(e) => format!("{}: {e}", action_label(&action)),
        });
    }
}

fn action_label(action: &PendingAction) -> &'static str {
    match action {
        PendingAction::Move { .. } => "move",
        PendingAction::SendMail { .. } => "send",
        PendingAction::CreateContact { .. } => "contact",
        PendingAction::DeleteContact(_) => "contact",
        PendingAction::CreateEvent { .. } => "event",
        PendingAction::DeleteEvent(_) => "event",
    }
}

async fn execute_one(app: &App, action: &PendingAction) -> JmapResult<String> {
    let client = app.client.as_ref().ok_or("no JMAP client")?;
    match action {
        PendingAction::Move {
            email_id,
            source_mailbox_id,
            target_mailbox_id,
            action_name,
        } => {
            jmap::mail::move_email(client, email_id, source_mailbox_id, target_mailbox_id).await?;
            Ok(format!("✓ {action_name} done"))
        }
        PendingAction::SendMail {
            to,
            cc,
            bcc,
            subject,
            body,
        } => {
            let profile = app
                .config
                .profiles
                .get(&app.active_profile_name)
                .ok_or("active profile not found")?;
            let from_email = profile
                .from_email
                .as_deref()
                .ok_or("set from_email in the profile to send mail")?;
            let from_name = profile.from_name.as_deref().unwrap_or("");

            // Resolve sent folder: config override > role > default
            let sent_mailbox_id = resolve_folder_id(
                &app.folders,
                profile.folders.sent.as_deref(),
                "sent",
                "Sent",
            );

            jmap::mail::send_message(
                client,
                &jmap::mail::OutgoingMail {
                    from_email,
                    from_name,
                    to,
                    cc,
                    bcc,
                    subject,
                    body,
                },
                sent_mailbox_id.as_deref(),
            )
            .await?;
            Ok("✓ Email sent".to_string())
        }
        PendingAction::CreateContact { name, email, phone } => {
            jmap::contacts::create_contact(client, name, email, phone).await?;
            Ok("✓ Contact created".to_string())
        }
        PendingAction::DeleteContact(id) => {
            jmap::contacts::delete_contact(client, id).await?;
            Ok("✓ Contact deleted".to_string())
        }
        PendingAction::CreateEvent {
            title,
            start,
            duration,
        } => {
            jmap::calendar::create_event(client, title, start, duration).await?;
            Ok("✓ Event created".to_string())
        }
        PendingAction::DeleteEvent(id) => {
            jmap::calendar::delete_event(client, id).await?;
            Ok("✓ Event deleted".to_string())
        }
    }
}

/// Connect to the active profile.
async fn connect_profile(app: &mut App) -> JmapResult<()> {
    let profile = app.config.get_profile(Some(&app.active_profile_name))?;
    let client = auth::create_client(profile, &app.active_profile_name).await?;
    app.client = Some(client);
    Ok(())
}

/// Fetch mailbox list via JMAP and build a tree-ordered list.
async fn fetch_folders(client: &JmapClient) -> JmapResult<Vec<FolderEntry>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session);
    let resp = sc.mailbox_get(None, None).await?;

    // Build flat entries first
    let raw: Vec<RawFolder> = resp
        .list
        .iter()
        .map(|m| RawFolder {
            id: m.id.as_ref().to_string(),
            name: m.name.clone(),
            parent_id: m.parent_id.as_ref().map(|p| p.as_ref().to_string()),
            role: m.role.as_ref().map(|r| r.to_wire_str().to_string()),
            sort_order: m.sort_order,
            total_emails: m.total_emails,
            unread_emails: m.unread_emails,
        })
        .collect();

    // Build tree-ordered output using DFS
    let mut result = Vec::with_capacity(raw.len());
    build_folder_tree(&raw, None, 0, &mut result);
    Ok(result)
}

struct RawFolder {
    id: String,
    name: String,
    parent_id: Option<String>,
    role: Option<String>,
    sort_order: u32,
    total_emails: u32,
    unread_emails: u32,
}

/// Recursively build a tree-ordered folder list via DFS.
fn build_folder_tree(
    raw: &[RawFolder],
    parent_id: Option<&str>,
    depth: usize,
    out: &mut Vec<FolderEntry>,
) {
    let mut children: Vec<&RawFolder> = raw
        .iter()
        .filter(|f| f.parent_id.as_deref() == parent_id)
        .collect();

    // Sort: default role folders first (in canonical order), then by sort_order, then name
    children.sort_by(|a, b| {
        role_priority(a.role.as_deref())
            .cmp(&role_priority(b.role.as_deref()))
            .then_with(|| a.sort_order.cmp(&b.sort_order))
            .then_with(|| a.name.cmp(&b.name))
    });

    for f in children {
        let indent = "  ".repeat(depth);
        let prefix = if depth > 0 { "└ " } else { "" };
        let display_name = format!("{indent}{prefix}{}", f.name);

        out.push(FolderEntry {
            id: f.id.clone(),
            name: f.name.clone(),
            parent_id: f.parent_id.clone(),
            role: f.role.clone(),
            sort_order: f.sort_order,
            total_emails: f.total_emails,
            unread_emails: f.unread_emails,
            display_name,
            depth,
            action_tag: None,
        });

        build_folder_tree(raw, Some(f.id.as_str()), depth + 1, out);
    }
}

/// Priority order for well-known mailbox roles.
/// Lower number = appears first. Folders without a known role get 99.
fn role_priority(role: Option<&str>) -> u8 {
    match role {
        Some("inbox") => 0,
        Some("drafts") => 1,
        Some("sent") => 2,
        Some("archive") => 3,
        Some("trash") => 4,
        Some("junk") => 5,
        _ => 99,
    }
}

/// Tag folders with their resolved action roles based on config.
/// Uses the same resolution order: defaults → role → config override.
pub fn tag_action_folders(folders: &mut [FolderEntry], app: &App) {
    use crate::config::FolderMappings;

    let mappings = app
        .config
        .profiles
        .get(&app.active_profile_name)
        .map(|p| &p.folders);
    let empty = FolderMappings::default();
    let mappings = mappings.unwrap_or(&empty);

    // For each action, resolve which folder it targets and tag it
    let actions: &[(&str, Option<&str>, &str, &str)] = &[
        ("inbox", None, "inbox", "Inbox"),
        ("drafts", None, "drafts", "Drafts"),
        ("sent", mappings.sent.as_deref(), "sent", "Sent"),
        ("archive", mappings.archive.as_deref(), "archive", "Archive"),
        ("trash", mappings.trash.as_deref(), "trash", "Trash"),
        ("spam", mappings.spam.as_deref(), "junk", "Junk"),
    ];

    for &(tag, config_override, role, default_name) in actions {
        let target_id = if let Some(configured) = config_override {
            resolve_folder_path(folders, configured)
        } else if let Some(f) = folders.iter().find(|f| f.role.as_deref() == Some(role)) {
            Some(f.id.clone())
        } else {
            resolve_folder_path(folders, default_name)
        };

        if let Some(id) = target_id {
            if let Some(f) = folders.iter_mut().find(|f| f.id == id) {
                f.action_tag = Some(tag.to_string());
            }
        }
    }
}

/// Fetch the 50 most recent emails in a folder.
async fn fetch_mails(client: &JmapClient, folder_id: &str) -> JmapResult<Vec<MailEntry>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session.clone());

    let filter = json!({ "inMailbox": folder_id });
    let sort = json!([{ "property": "receivedAt", "isAscending": false }]);
    let query_resp = sc
        .email_query(Some(filter), Some(sort), Some(0), Some(50), None)
        .await?;

    if query_resp.ids.is_empty() {
        return Ok(Vec::new());
    }

    // Include all required non-optional fields to avoid parse errors
    let email_resp = sc
        .email_get(
            Some(&query_resp.ids),
            Some(&[
                "id",
                "blobId",
                "threadId",
                "mailboxIds",
                "size",
                "receivedAt",
                "subject",
                "from",
                "preview",
            ]),
            None,
        )
        .await?;

    let mails = email_resp
        .list
        .iter()
        .map(|e| {
            let from = e
                .from
                .as_ref()
                .and_then(|addrs| addrs.first())
                .map(|a| a.name.as_deref().unwrap_or(&a.email).to_string())
                .unwrap_or_else(|| "(unknown)".into());

            MailEntry {
                id: e.id.as_ref().to_string(),
                subject: e.subject.as_deref().unwrap_or("(no subject)").to_string(),
                from,
                date: e.received_at.as_ref().to_string(),
                preview: e.preview.as_deref().unwrap_or("").to_string(),
                folder_name: None,
            }
        })
        .collect();

    Ok(mails)
}

/// Fetch the 50 most recent emails across ALL folders, with folder names resolved.
pub async fn fetch_all_mails(
    client: &JmapClient,
    folders: &[FolderEntry],
) -> JmapResult<Vec<MailEntry>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session.clone());

    // Query without inMailbox filter → returns mails from all folders
    let sort = json!([{ "property": "receivedAt", "isAscending": false }]);
    let query_resp = sc
        .email_query(None, Some(sort), Some(0), Some(50), None)
        .await?;

    if query_resp.ids.is_empty() {
        return Ok(Vec::new());
    }

    let email_resp = sc
        .email_get(
            Some(&query_resp.ids),
            Some(&[
                "id",
                "blobId",
                "threadId",
                "mailboxIds",
                "size",
                "receivedAt",
                "subject",
                "from",
                "preview",
            ]),
            None,
        )
        .await?;

    // Build a lookup from folder id → folder name
    let folder_lookup: std::collections::HashMap<&str, &str> = folders
        .iter()
        .map(|f| (f.id.as_str(), f.name.as_str()))
        .collect();

    let mails = email_resp
        .list
        .iter()
        .map(|e| {
            let from = e
                .from
                .as_ref()
                .and_then(|addrs| addrs.first())
                .map(|a| a.name.as_deref().unwrap_or(&a.email).to_string())
                .unwrap_or_else(|| "(unknown)".into());

            // Resolve folder name from mailboxIds (use first matching folder)
            let folder_name = e
                .mailbox_ids
                .keys()
                .find_map(|id| folder_lookup.get(id.as_ref()).copied())
                .map(str::to_string);

            MailEntry {
                id: e.id.as_ref().to_string(),
                subject: e.subject.as_deref().unwrap_or("(no subject)").to_string(),
                from,
                date: e.received_at.as_ref().to_string(),
                preview: e.preview.as_deref().unwrap_or("").to_string(),
                folder_name,
            }
        })
        .collect();

    Ok(mails)
}

/// Fetch contacts.
async fn fetch_contacts(client: &JmapClient) -> JmapResult<Vec<ContactEntry>> {
    use jmap_contacts_client::JmapContactsExt;

    let session = client.fetch_session().await?;
    let sc = client.with_contacts_session(session);
    let resp = sc
        .contact_card_get(None, Some(&["id", "name", "emails", "phones"]))
        .await?;

    let contacts = resp
        .list
        .iter()
        .map(|card| ContactEntry {
            id: card
                .id
                .as_ref()
                .map(|i| i.as_ref().to_string())
                .unwrap_or_default(),
            name: jmap::contacts::extract_contact_name(card),
            email: jmap::contacts::extract_first_email(card),
            phone: jmap::contacts::extract_first_phone(card),
        })
        .collect();

    Ok(contacts)
}

/// Fetch calendar events.
async fn fetch_events(client: &JmapClient) -> JmapResult<Vec<CalendarEventEntry>> {
    use jmap_calendars_client::JmapCalendarsExt;

    let session = client.fetch_session().await?;
    let sc = client.with_calendars_session(session);
    let resp = sc
        .calendar_event_get(
            None,
            Some(&["id", "title", "start", "duration", "status"]),
            None,
        )
        .await?;

    let events = resp
        .list
        .iter()
        .map(|e| CalendarEventEntry {
            id: e
                .id
                .as_ref()
                .map(|i| i.as_ref().to_string())
                .unwrap_or_default(),
            title: e.title.as_deref().unwrap_or("(no title)").to_string(),
            start: e.start.as_deref().unwrap_or("").to_string(),
            duration: e.duration.as_deref().unwrap_or("").to_string(),
            status: e.status.as_deref().unwrap_or("").to_string(),
        })
        .collect();

    Ok(events)
}
