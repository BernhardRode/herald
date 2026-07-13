//! List entry types for each hierarchy level, plus their display formatting.

/// A profile entry (from config).
#[derive(Debug, Clone)]
pub struct ProfileEntry {
    pub name: String,
    pub server_url: String,
}

/// A mailbox/folder entry.
#[derive(Debug, Clone)]
pub struct FolderEntry {
    pub id: String,
    pub name: String,
    #[allow(dead_code)]
    pub parent_id: Option<String>,
    pub role: Option<String>,
    #[allow(dead_code)]
    pub sort_order: u32,
    pub total_emails: u32,
    pub unread_emails: u32,
    /// Computed display name with tree indentation.
    pub display_name: String,
    /// Tree depth (0 = root).
    #[allow(dead_code)]
    pub depth: usize,
    /// Action tag from config resolution (e.g. "sent", "trash", "archive", "spam").
    /// Shows which action targets this folder per the user's config.
    pub action_tag: Option<String>,
}

/// An email entry (list view).
#[derive(Debug, Clone)]
pub struct MailEntry {
    pub id: String,
    pub subject: String,
    pub from: String,
    pub date: String,
    pub preview: String,
    /// Folder name for cross-folder search results. `None` for single-folder views.
    pub folder_name: Option<String>,
}

/// A contact entry (list view).
#[derive(Debug, Clone)]
pub struct ContactEntry {
    pub id: String,
    pub name: String,
    pub email: String,
    pub phone: String,
}

/// A calendar event entry (list view).
#[derive(Debug, Clone)]
pub struct CalendarEventEntry {
    pub id: String,
    pub title: String,
    pub start: String,
    pub duration: String,
    pub status: String,
}

pub fn format_folder(f: &FolderEntry) -> String {
    // Show action tag (from config resolution) if set, otherwise server role
    let tag = f
        .action_tag
        .as_deref()
        .or(f.role.as_deref())
        .unwrap_or("");
    let unread = if f.unread_emails > 0 {
        format!(" •{}", f.unread_emails)
    } else {
        String::new()
    };
    if tag.is_empty() {
        format!("{}  ({}{})", f.display_name, f.total_emails, unread)
    } else {
        format!(
            "{}  [{}]  ({}{})",
            f.display_name, tag, f.total_emails, unread
        )
    }
}

pub fn format_mail(m: &MailEntry) -> String {
    if let Some(ref folder) = m.folder_name {
        format!("[{}] {} — {}", folder, m.from, m.subject)
    } else {
        format!("{} — {}", m.from, m.subject)
    }
}

pub fn format_contact(c: &ContactEntry) -> String {
    if c.email.is_empty() {
        c.name.clone()
    } else {
        format!("{} <{}>", c.name, c.email)
    }
}

pub fn format_event(e: &CalendarEventEntry) -> String {
    if e.start.is_empty() {
        e.title.clone()
    } else {
        format!("{} — {}", e.start, e.title)
    }
}
