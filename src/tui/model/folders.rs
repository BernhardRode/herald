//! Folder tree building, config-based action tagging, and sorting.
//!
//! Resolution order for every mail action (sent/archive/trash/spam):
//! explicit config override → server JMAP role → hardcoded default name.

use crate::config::FolderMappings;
use crate::tui::types::FolderEntry;

/// A folder as it comes from the server, before tree ordering.
#[derive(Debug, Clone)]
pub struct RawFolder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub role: Option<String>,
    pub sort_order: u32,
    pub total_emails: u32,
    pub unread_emails: u32,
}

/// Build the display list: DFS tree order, action tags resolved from config,
/// and action-target subtrees hoisted to the top in canonical order.
pub fn build_folder_list(raw: &[RawFolder], mappings: &FolderMappings) -> Vec<FolderEntry> {
    let mut folders = Vec::with_capacity(raw.len());
    build_tree(raw, None, 0, &mut folders);
    tag_action_folders(&mut folders, mappings);
    sort_action_folders_first(&mut folders);
    folders
}

fn build_tree(
    raw: &[RawFolder],
    parent_id: Option<&str>,
    depth: usize,
    out: &mut Vec<FolderEntry>,
) {
    let mut children: Vec<&RawFolder> = raw
        .iter()
        .filter(|f| f.parent_id.as_deref() == parent_id)
        .collect();

    children.sort_by(|a, b| {
        role_priority(a.role.as_deref())
            .cmp(&role_priority(b.role.as_deref()))
            .then_with(|| a.sort_order.cmp(&b.sort_order))
            .then_with(|| a.name.cmp(&b.name))
    });

    for f in children {
        let indent = "  ".repeat(depth);
        let prefix = if depth > 0 { "└ " } else { "" };
        out.push(FolderEntry {
            id: f.id.clone(),
            name: f.name.clone(),
            parent_id: f.parent_id.clone(),
            role: f.role.clone(),
            sort_order: f.sort_order,
            total_emails: f.total_emails,
            unread_emails: f.unread_emails,
            display_name: format!("{indent}{prefix}{}", f.name),
            depth,
            action_tag: None,
        });
        build_tree(raw, Some(f.id.as_str()), depth + 1, out);
    }
}

/// Priority order for well-known mailbox roles (initial within-level sort).
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

/// The actions with their config override, JMAP role, and default name.
fn actions(mappings: &FolderMappings) -> [(&'static str, Option<&str>, &'static str, &'static str); 6] {
    [
        ("inbox", None, "inbox", "Inbox"),
        ("drafts", None, "drafts", "Drafts"),
        ("sent", mappings.sent.as_deref(), "sent", "Sent"),
        ("archive", mappings.archive.as_deref(), "archive", "Archive"),
        ("trash", mappings.trash.as_deref(), "trash", "Trash"),
        ("spam", mappings.spam.as_deref(), "junk", "Junk"),
    ]
}

/// Resolve the target folder id for a mail action ("sent", "archive",
/// "trash", "spam", "inbox", "drafts").
pub fn resolve_action_folder(
    folders: &[FolderEntry],
    mappings: &FolderMappings,
    action: &str,
) -> Option<String> {
    let (_, config_override, role, default_name) = actions(mappings)
        .into_iter()
        .find(|(tag, ..)| *tag == action)?;

    if let Some(configured) = config_override {
        resolve_folder_path(folders, configured)
    } else if let Some(f) = folders.iter().find(|f| f.role.as_deref() == Some(role)) {
        Some(f.id.clone())
    } else {
        resolve_folder_path(folders, default_name)
    }
}

/// Resolve a folder by name or slash-separated path (e.g. "Archive/2026").
pub fn resolve_folder_path(folders: &[FolderEntry], path: &str) -> Option<String> {
    if !path.contains('/') {
        return folders.iter().find(|f| f.name == path).map(|f| f.id.clone());
    }
    let segments: Vec<&str> = path.split('/').collect();
    let mut current_parent: Option<String> = None;
    for (i, segment) in segments.iter().enumerate() {
        let f = folders.iter().find(|f| {
            f.name == *segment && f.parent_id.as_deref() == current_parent.as_deref()
        })?;
        if i == segments.len() - 1 {
            return Some(f.id.clone());
        }
        current_parent = Some(f.id.clone());
    }
    None
}

/// Tag exactly one folder per action with the resolved action tag.
fn tag_action_folders(folders: &mut [FolderEntry], mappings: &FolderMappings) {
    let targets: Vec<(String, Option<String>)> = actions(mappings)
        .into_iter()
        .map(|(tag, ..)| {
            (
                tag.to_string(),
                resolve_action_folder(folders, mappings, tag),
            )
        })
        .collect();
    for (tag, target) in targets {
        if let Some(id) = target {
            if let Some(f) = folders.iter_mut().find(|f| f.id == id) {
                f.action_tag = Some(tag);
            }
        }
    }
}

/// Hoist top-level subtrees containing an action target to the top, in
/// canonical action order, keeping subtree blocks intact.
fn sort_action_folders_first(folders: &mut Vec<FolderEntry>) {
    let mut blocks: Vec<Vec<FolderEntry>> = Vec::new();
    for f in folders.drain(..) {
        if f.depth == 0 || blocks.is_empty() {
            blocks.push(vec![f]);
        } else {
            blocks.last_mut().unwrap().push(f);
        }
    }
    blocks.sort_by_key(|block| {
        block
            .iter()
            .map(|f| action_tag_priority(f.action_tag.as_deref()))
            .min()
            .unwrap_or(u8::MAX)
    });
    *folders = blocks.into_iter().flatten().collect();
}

fn action_tag_priority(tag: Option<&str>) -> u8 {
    match tag {
        Some("inbox") => 0,
        Some("drafts") => 1,
        Some("sent") => 2,
        Some("archive") => 3,
        Some("trash") => 4,
        Some("spam") => 5,
        _ => u8::MAX,
    }
}

/// Format a folder row: display name, action tag, counts.
pub fn format_folder(f: &FolderEntry) -> String {
    let unread = if f.unread_emails > 0 {
        format!(" •{}", f.unread_emails)
    } else {
        String::new()
    };
    match f.action_tag.as_deref() {
        Some(tag) => format!("{}  [{}]  ({}{})", f.display_name, tag, f.total_emails, unread),
        None => format!("{}  ({}{})", f.display_name, f.total_emails, unread),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: &str, name: &str, parent: Option<&str>, role: Option<&str>) -> RawFolder {
        RawFolder {
            id: id.into(),
            name: name.into(),
            parent_id: parent.map(String::from),
            role: role.map(String::from),
            sort_order: 0,
            total_emails: 0,
            unread_emails: 0,
        }
    }

    fn sample() -> Vec<RawFolder> {
        vec![
            raw("in", "Inbox", None, Some("inbox")),
            raw("dr", "Drafts", None, Some("drafts")),
            raw("si", "Sent Items", None, Some("sent")),
            raw("sm", "Sent Messages", None, None),
            raw("ar", "Archive", None, None),
            raw("a26", "2026", Some("ar"), None),
            raw("di", "Deleted Items", None, Some("trash")),
            raw("dm", "Deleted Messages", None, None),
            raw("jm", "Junk Mail", None, Some("junk")),
            raw("tv", "tgv-gz", None, None),
        ]
    }

    fn mappings() -> FolderMappings {
        FolderMappings {
            sent: Some("Sent Messages".into()),
            archive: Some("Archive/2026".into()),
            trash: Some("Deleted Messages".into()),
            spam: None,
        }
    }

    fn tag_of<'a>(folders: &'a [FolderEntry], id: &str) -> Option<&'a str> {
        folders
            .iter()
            .find(|f| f.id == id)
            .and_then(|f| f.action_tag.as_deref())
    }

    #[test]
    fn config_override_beats_server_role() {
        let folders = build_folder_list(&sample(), &mappings());
        assert_eq!(tag_of(&folders, "sm"), Some("sent"));
        assert_eq!(tag_of(&folders, "si"), None, "role folder must lose its tag");
        assert_eq!(tag_of(&folders, "dm"), Some("trash"));
        assert_eq!(tag_of(&folders, "di"), None);
    }

    #[test]
    fn role_wins_without_config_override() {
        let folders = build_folder_list(&sample(), &mappings());
        // spam has no override → Junk Mail keeps it via its junk role
        assert_eq!(tag_of(&folders, "jm"), Some("spam"));
        assert_eq!(tag_of(&folders, "in"), Some("inbox"));
    }

    #[test]
    fn nested_path_resolution() {
        let folders = build_folder_list(&sample(), &mappings());
        assert_eq!(tag_of(&folders, "a26"), Some("archive"));
        assert_eq!(
            resolve_folder_path(&folders, "Archive/2026"),
            Some("a26".to_string())
        );
        assert_eq!(resolve_folder_path(&folders, "Archive/1999"), None);
    }

    #[test]
    fn action_subtrees_hoisted_in_canonical_order() {
        let folders = build_folder_list(&sample(), &mappings());
        let order: Vec<&str> = folders
            .iter()
            .filter(|f| f.depth == 0)
            .map(|f| f.id.as_str())
            .collect();
        // inbox, drafts, sent(Sent Messages), archive(Archive: nested target),
        // trash(Deleted Messages), spam(Junk Mail), then the rest
        assert_eq!(&order[..6], &["in", "dr", "sm", "ar", "dm", "jm"]);
        assert!(order[6..].contains(&"si"));
        assert!(order[6..].contains(&"tv"));
    }

    #[test]
    fn subtree_stays_with_its_root() {
        let folders = build_folder_list(&sample(), &mappings());
        let ar_pos = folders.iter().position(|f| f.id == "ar").unwrap();
        assert_eq!(folders[ar_pos + 1].id, "a26", "child follows parent");
        assert_eq!(folders[ar_pos + 1].depth, 1);
    }

    #[test]
    fn resolve_action_folder_precedence() {
        let folders = build_folder_list(&sample(), &mappings());
        let m = mappings();
        assert_eq!(
            resolve_action_folder(&folders, &m, "sent"),
            Some("sm".into())
        );
        assert_eq!(
            resolve_action_folder(&folders, &m, "spam"),
            Some("jm".into()),
            "falls back to junk role"
        );
        assert_eq!(
            resolve_action_folder(&folders, &m, "archive"),
            Some("a26".into())
        );
    }

    #[test]
    fn only_tagged_folder_shows_bracket() {
        let folders = build_folder_list(&sample(), &mappings());
        let tagged = folders
            .iter()
            .filter(|f| f.action_tag.as_deref() == Some("sent"))
            .count();
        assert_eq!(tagged, 1);
        let f = folders.iter().find(|f| f.id == "sm").unwrap();
        assert!(format_folder(f).contains("[sent]"));
        let f = folders.iter().find(|f| f.id == "si").unwrap();
        assert!(!format_folder(f).contains("[sent]"));
    }
}
