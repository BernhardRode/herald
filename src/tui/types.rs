//! Entry types shared between the worker (which fetches them) and the
//! screens (which display them).

/// A mailbox/folder entry.
#[derive(Debug, Clone)]
pub struct FolderEntry {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub role: Option<String>,
    pub total_emails: u32,
    pub unread_emails: u32,
    /// Computed display name with tree indentation.
    pub display_name: String,
    /// Tree depth (0 = root).
    pub depth: usize,
    /// Action tag from config resolution (e.g. "sent", "trash").
    pub action_tag: Option<String>,
}

/// An email list entry.
#[derive(Debug, Clone)]
pub struct MailEntry {
    pub id: String,
    pub subject: String,
    pub from: String,
    pub date: String,
    pub preview: String,
    /// First mailbox id, for folder-name resolution in all-folder views.
    pub folder_id: Option<String>,
    pub is_read: bool,
}

impl MailEntry {
    pub fn is_unread(&self) -> bool {
        !self.is_read
    }
}

/// A contact list entry.
#[derive(Debug, Clone)]
pub struct ContactEntry {
    pub id: String,
    pub name: String,
    pub email: String,
    pub phone: String,
}

/// A calendar event entry.
#[derive(Debug, Clone)]
pub struct EventEntry {
    pub id: String,
    pub title: String,
    /// ISO 8601 local date-time, e.g. "2026-07-13T09:00:00".
    pub start: String,
    /// ISO 8601 duration, e.g. "PT1H".
    pub duration: String,
    pub status: String,
}

impl EventEntry {
    /// The date part of `start` ("YYYY-MM-DD"), if present.
    pub fn start_date(&self) -> &str {
        self.start.get(..10).unwrap_or("")
    }

    /// The time part of `start` ("HH:MM"), if present.
    pub fn start_time(&self) -> &str {
        self.start.get(11..16).unwrap_or("")
    }
}
