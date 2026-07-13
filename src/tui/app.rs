//! Main TUI application state and event loop.
//!
//! Implements hierarchical navigation:
//!   Profiles → Folders (Mailboxes) → Mails
//!
//! Default view shows Inbox mails. Arrow left goes up the hierarchy,
//! Arrow right / Enter goes deeper.

use std::io;
use std::time::Duration;

use crossterm::event::Event;
use ratatui::widgets::ListState;
use ratatui::DefaultTerminal;

use jmap_base_client::JmapClient;
use jmap_mail_client::JmapMailExt;
use serde_json::json;

use crate::auth;
use crate::config::Config;

use super::event::{self, Action};
use super::popout::{HeaderField, Popout, PopoutKind, PopoutManager};
use super::search::{MatchedItem, Matcher};
use super::ui::preview::PreviewContent;
use super::ui::{input, layout::Layout, preview, results};

// ---------------------------------------------------------------------------
// Data types for the three hierarchy levels
// ---------------------------------------------------------------------------

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
}

/// An email entry (list view).
#[derive(Debug, Clone)]
pub struct MailEntry {
    #[allow(dead_code)] // Used for future full-body fetch on Enter
    pub id: String,
    pub subject: String,
    pub from: String,
    pub date: String,
    pub preview: String,
}

/// A contact entry (list view).
#[derive(Debug, Clone)]
pub struct ContactEntry {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    pub email: String,
    pub phone: String,
}

/// A calendar event entry (list view).
#[derive(Debug, Clone)]
pub struct CalendarEventEntry {
    #[allow(dead_code)]
    pub id: String,
    pub title: String,
    pub start: String,
    pub duration: String,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Panel / Navigation state
// ---------------------------------------------------------------------------

/// Top-level mode — switch between Mail, Contacts, Calendar with Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Mail,
    Contacts,
    Calendar,
}

impl Mode {
    pub const fn all() -> &'static [Mode] {
        &[Mode::Mail, Mode::Contacts, Mode::Calendar]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Mode::Mail => "Mail",
            Mode::Contacts => "Contacts",
            Mode::Calendar => "Calendar",
        }
    }

    pub const fn next(self) -> Mode {
        match self {
            Mode::Mail => Mode::Contacts,
            Mode::Contacts => Mode::Calendar,
            Mode::Calendar => Mode::Mail,
        }
    }

    pub const fn prev(self) -> Mode {
        match self {
            Mode::Mail => Mode::Calendar,
            Mode::Contacts => Mode::Mail,
            Mode::Calendar => Mode::Contacts,
        }
    }
}

/// Which level of the hierarchy is currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Profiles,
    Folders,
    Mails,
    Contacts,
    Calendar,
}

impl Panel {
    pub fn title(self) -> &'static str {
        match self {
            Panel::Profiles => "Profiles",
            Panel::Folders => "Folders",
            Panel::Mails => "Inbox",
            Panel::Contacts => "Contacts",
            Panel::Calendar => "Calendar",
        }
    }
}

// ---------------------------------------------------------------------------
// Mail action state
// ---------------------------------------------------------------------------

/// A pending mail move operation to be executed async.
#[derive(Debug, Clone)]
pub struct PendingMove {
    pub email_id: String,
    pub target_mailbox_id: String,
    pub source_mailbox_id: String,
    pub action_name: String,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

/// The main application state.
pub struct App {
    /// Current search input string.
    pub input: String,
    /// Vi-style input mode (Normal / Insert).
    pub input_mode: event::InputMode,
    /// Top-level mode (Mail / Contacts / Calendar).
    pub mode: Mode,
    /// Which panel is active.
    pub panel: Panel,
    /// Whether the matcher is still running.
    pub matcher_running: bool,
    /// Total items in the matcher.
    pub total_count: u32,
    /// Matched items count.
    pub matched_count: u32,
    /// Current page of results for display.
    pub results: Vec<MatchedItem<String>>,
    /// Selection state for the list widget.
    pub list_state: ListState,
    /// Preview content for the right pane.
    pub preview_content: Option<PreviewContent>,
    /// Height of the results area.
    pub results_height: u16,

    // Internal state
    should_quit: bool,
    matcher: Matcher<String>,

    // Data stores
    profiles: Vec<ProfileEntry>,
    folders: Vec<FolderEntry>,
    mails: Vec<MailEntry>,
    contacts: Vec<ContactEntry>,
    events: Vec<CalendarEventEntry>,

    // Context
    active_profile_name: String,
    active_folder_id: Option<String>,
    active_folder_name: String,

    // JMAP client (created on profile selection)
    client: Option<JmapClient>,
    config: Config,

    // Async state
    loading: bool,
    error_message: Option<String>,

    // Mail actions
    pub needs_reload: bool,
    pending_move: Option<PendingMove>,
    pending_compose: bool,
    /// Currently opened/selected email (EmailOpen mode).
    pub selected_mail: Option<MailEntry>,
    // Contact/Calendar actions
    pending_contact_create: bool,
    pending_contact_delete: bool,
    pending_event_create: bool,
    pending_event_delete: bool,
    // Popout windows
    pub popouts: PopoutManager,
    // Quit confirmation
    pub show_quit_confirm: bool,
    /// The input mode to restore when cancelling a destructive action confirmation.
    pre_confirm_mode: Option<event::InputMode>,
}

impl App {
    /// Create a new app with config, connect to the default profile.
    pub fn new(config: Config, profile_name: Option<&str>) -> Self {
        // Build profile entries
        let profiles: Vec<ProfileEntry> = config
            .profiles
            .iter()
            .map(|(name, p)| ProfileEntry {
                name: name.clone(),
                server_url: p.server_url.clone(),
            })
            .collect();

        let active_profile_name = profile_name
            .map(|s| s.to_string())
            .or_else(|| config.default_profile.clone())
            .unwrap_or_else(|| profiles.first().map(|p| p.name.clone()).unwrap_or_default());

        let matcher = Matcher::new();

        Self {
            input: String::new(),
            input_mode: event::InputMode::Normal,
            mode: Mode::Mail,
            panel: Panel::Mails,
            matcher_running: false,
            total_count: 0,
            matched_count: 0,
            results: Vec::new(),
            list_state: ListState::default().with_selected(Some(0)),
            preview_content: None,
            results_height: 20,
            should_quit: false,
            matcher,
            profiles,
            folders: Vec::new(),
            mails: Vec::new(),
            contacts: Vec::new(),
            events: Vec::new(),
            active_profile_name,
            active_folder_id: None,
            active_folder_name: "Inbox".to_string(),
            client: None,
            config,
            loading: false,
            error_message: None,
            needs_reload: false,
            pending_move: None,
            pending_compose: false,
            selected_mail: None,
            pending_contact_create: false,
            pending_contact_delete: false,
            pending_event_create: false,
            pending_event_delete: false,
            popouts: PopoutManager::new(),
            show_quit_confirm: false,
            pre_confirm_mode: None,
        }
    }

    /// The display name for the current context (shown in search bar title).
    pub fn context_title(&self) -> String {
        match self.panel {
            Panel::Profiles => "Select Profile".to_string(),
            Panel::Folders => format!("{}  ❯  Folders", self.active_profile_name),
            Panel::Mails => format!(
                "{}  ❯  {}",
                self.active_profile_name, self.active_folder_name
            ),
            Panel::Contacts => format!("{}  ❯  Contacts", self.active_profile_name),
            Panel::Calendar => format!("{}  ❯  Calendar", self.active_profile_name),
        }
    }

    /// Process a user action.
    fn handle_action(&mut self, action: Action) {
        // If quit confirm dialog is showing, handle it first
        if self.show_quit_confirm {
            match action {
                Action::NavigateRight | Action::OpenItem => {
                    // Enter confirms quit
                    self.should_quit = true;
                }
                _ => {
                    // Any other key dismisses the dialog
                    self.show_quit_confirm = false;
                }
            }
            return;
        }

        // Handle confirmation state — y executes, n/Esc cancels, everything else is ignored
        if let event::InputMode::Confirm(ref confirm_action) = self.input_mode {
            match action {
                Action::ConfirmYes => {
                    // Execute the confirmed action
                    let target = confirm_action.target_name().to_string();
                    let email_id = confirm_action.email_id().to_string();
                    self.input_mode = self
                        .pre_confirm_mode
                        .take()
                        .unwrap_or(event::InputMode::Normal);
                    self.execute_confirmed_action(&target, &email_id);
                }
                Action::ConfirmNo => {
                    // Cancel — return to previous state
                    self.input_mode = self
                        .pre_confirm_mode
                        .take()
                        .unwrap_or(event::InputMode::Normal);
                    self.error_message = Some("Cancelled".to_string());
                }
                _ => {
                    // Ignore all other keys in Confirm mode
                }
            }
            return;
        }

        match action {
            Action::Quit => {
                self.show_quit_confirm = true;
            }
            Action::SelectPrev => self.select_prev(),
            Action::SelectNext => self.select_next(),
            Action::NavigateRight => self.navigate_right(),
            Action::NavigateLeft => self.navigate_left(),
            Action::SwitchModeNext => self.switch_mode(self.mode.next()),
            Action::SwitchModePrev => self.switch_mode(self.mode.prev()),
            Action::EnterInsert => {
                // If in EmailOpen with an editor popout, go to Editing mode
                if self.input_mode == event::InputMode::EmailOpen {
                    if let Some(p) = self.popouts.focused_popout() {
                        if p.is_editor() {
                            self.input_mode = event::InputMode::Editing;
                            return;
                        }
                    }
                }
                self.input_mode = event::InputMode::Insert;
            }
            Action::ExitToNormal => {
                self.input_mode = event::InputMode::Normal;
                self.selected_mail = None;
            }
            Action::OpenItem => self.open_item(),
            Action::InsertChar(c) => {
                self.input.push(c);
                self.update_search();
            }
            Action::Backspace => {
                self.input.pop();
                self.update_search();
            }
            Action::ClearInput => {
                self.input.clear();
                self.update_search();
            }
            Action::ExecuteCommand => self.execute_input_command(),
            // Mail actions (from list view and EmailOpen)
            Action::Reply => {
                // Don't reply from a draft/reply popout — only from email view or list
                if let Some(p) = self.popouts.focused_popout() {
                    if p.is_editor() {
                        self.error_message = Some("Already composing".to_string());
                        return;
                    }
                }
                self.slash_reply();
            }
            Action::Forward => {
                if let Some(p) = self.popouts.focused_popout() {
                    if p.is_editor() {
                        self.error_message = Some("Already composing".to_string());
                        return;
                    }
                }
                self.slash_forward();
            }
            Action::Archive => self.request_confirm_action("archive"),
            Action::Delete => {
                // If focused popout is a draft/reply/forward, discard it
                if let Some(popout) = self.popouts.focused_popout() {
                    if popout.is_editor() {
                        self.popouts.close_focused();
                        self.error_message = Some("Draft discarded".to_string());
                        if self.popouts.popouts.is_empty() {
                            self.input_mode = event::InputMode::Normal;
                            self.selected_mail = None;
                        }
                        return;
                    }
                }
                self.request_confirm_action("delete");
            }
            Action::Spam => self.request_confirm_action("spam"),
            Action::Compose => {
                let sig = self.get_signature();
                let popout = Popout::compose(0, sig.as_deref());
                self.popouts.open(popout);
                self.input_mode = event::InputMode::Editing;
            }
            Action::Send => {
                // Send the focused draft/reply/forward
                self.error_message = Some("✓ Sending...".to_string());
                self.popouts.close_focused();
                if self.popouts.popouts.is_empty() {
                    self.input_mode = event::InputMode::Normal;
                    self.selected_mail = None;
                }
            }
            // Header field editing
            Action::EditTo => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    if p.is_editor() {
                        p.editing_field = Some(HeaderField::To);
                        self.input_mode = event::InputMode::Editing;
                    }
                }
            }
            Action::EditSubject => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    if p.is_editor() {
                        p.editing_field = Some(HeaderField::Subject);
                        self.input_mode = event::InputMode::Editing;
                    }
                }
            }
            Action::EditCc => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    if p.is_editor() {
                        p.editing_field = Some(HeaderField::Cc);
                        self.input_mode = event::InputMode::Editing;
                    }
                }
            }
            Action::EditBcc => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    if p.is_editor() {
                        p.editing_field = Some(HeaderField::Bcc);
                        self.input_mode = event::InputMode::Editing;
                    }
                }
            }
            // Popout management
            Action::PopoutClose => {
                self.popouts.close_focused();
                if self.popouts.popouts.is_empty() {
                    self.input_mode = event::InputMode::Normal;
                    self.selected_mail = None;
                }
            }
            Action::PopoutToggleMax => self.popouts.toggle_maximize(),
            Action::PopoutMinimize => self.popouts.minimize_focused(),
            Action::PopoutSwitchFocus => self.popouts.switch_focus(),
            // Editor actions (typing in compose/reply popout)
            Action::EditorChar(c) => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    match p.editing_field {
                        Some(HeaderField::To) => p.to.push(c),
                        Some(HeaderField::Subject) => p.subject.push(c),
                        Some(HeaderField::Cc) => p.cc.push(c),
                        Some(HeaderField::Bcc) => p.bcc.push(c),
                        None => {
                            p.editor_buffer.insert(p.editor_cursor, c);
                            p.editor_cursor += c.len_utf8();
                        }
                    }
                    update_editor_body(p);
                }
            }
            Action::EditorBackspace => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    match p.editing_field {
                        Some(HeaderField::To) => {
                            p.to.pop();
                        }
                        Some(HeaderField::Subject) => {
                            p.subject.pop();
                        }
                        Some(HeaderField::Cc) => {
                            p.cc.pop();
                        }
                        Some(HeaderField::Bcc) => {
                            p.bcc.pop();
                        }
                        None => {
                            if p.editor_cursor > 0 {
                                let mut idx = p.editor_cursor;
                                loop {
                                    idx -= 1;
                                    if p.editor_buffer.is_char_boundary(idx) {
                                        break;
                                    }
                                }
                                p.editor_buffer.remove(idx);
                                p.editor_cursor = idx;
                            }
                        }
                    }
                    update_editor_body(p);
                }
            }
            Action::EditorNewline => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    match p.editing_field {
                        Some(_) => {
                            // Enter on a header field: done editing that field, go to body
                            p.editing_field = None;
                        }
                        None => {
                            p.editor_buffer.insert(p.editor_cursor, '\n');
                            p.editor_cursor += 1;
                        }
                    }
                    update_editor_body(p);
                }
            }
            Action::EditorEscape => {
                // Esc from editing → go to EmailOpen where 's' sends or 'Esc' closes
                self.input_mode = event::InputMode::EmailOpen;
                self.error_message = Some("Draft saved. 's' to send, Esc to close".to_string());
            }
            Action::ConfirmYes | Action::ConfirmNo => {
                // Handled above in the Confirm state check; no-op here.
            }
            Action::None => {}
        }
    }

    /// Navigate deeper into the hierarchy.
    fn navigate_right(&mut self) {
        let selected = self.list_state.selected().unwrap_or(0);
        match self.panel {
            Panel::Profiles => {
                // Select this profile and move to Folders
                if let Some(item) = self.results.get(selected) {
                    let profile_name = item.inner.clone();
                    self.active_profile_name = profile_name;
                    self.client = None; // Will be re-created
                    self.panel = Panel::Folders;
                    self.reset_for_panel();
                }
            }
            Panel::Folders => {
                // Select this folder and move to Mails — resolve by JMAP id
                if let Some(item) = self.results.get(selected) {
                    if let Some(folder) = self.folders.iter().find(|f| f.id == item.inner) {
                        self.active_folder_id = Some(folder.id.clone());
                        self.active_folder_name = folder.name.clone();
                        self.panel = Panel::Mails;
                        self.reset_for_panel();
                    }
                }
            }
            Panel::Mails => {
                // Enter = show preview (already shown on selection, noop)
            }
            Panel::Contacts | Panel::Calendar => {
                // No deeper level; preview is already shown on selection
            }
        }
    }

    /// Navigate back up the hierarchy.
    fn navigate_left(&mut self) {
        match self.panel {
            Panel::Mails => {
                self.panel = Panel::Folders;
                self.reset_for_panel();
            }
            Panel::Folders => {
                self.panel = Panel::Profiles;
                self.reset_for_panel();
            }
            Panel::Contacts | Panel::Calendar => {
                // Go back to profiles from contacts/calendar
                self.panel = Panel::Profiles;
                self.reset_for_panel();
            }
            Panel::Profiles => {
                // Already at the top
            }
        }
    }

    /// Switch top-level mode (Mail / Contacts / Calendar).
    fn switch_mode(&mut self, new_mode: Mode) {
        if new_mode == self.mode {
            return;
        }
        self.mode = new_mode;
        self.panel = match new_mode {
            Mode::Mail => Panel::Mails,
            Mode::Contacts => Panel::Contacts,
            Mode::Calendar => Panel::Calendar,
        };
        self.reset_for_panel();
    }

    /// Reset state when switching panels.
    fn reset_for_panel(&mut self) {
        self.input.clear();
        self.list_state.select(Some(0));
        self.results.clear();
        self.preview_content = None;
        // Recreate matcher (flush all items)
        self.matcher = Matcher::new();
    }

    fn update_search(&mut self) {
        self.matcher.find(&self.input);
    }

    /// Get the signature from the active profile config.
    fn get_signature(&self) -> Option<String> {
        self.config
            .profiles
            .get(&self.active_profile_name)
            .and_then(|p| p.signature.clone())
    }

    fn select_prev(&mut self) {
        let selected = self.list_state.selected().unwrap_or(0);
        if selected > 0 {
            self.list_state.select(Some(selected - 1));
            self.update_preview();
        }
    }

    fn select_next(&mut self) {
        let selected = self.list_state.selected().unwrap_or(0);
        let max = self.results.len().saturating_sub(1);
        if selected < max {
            self.list_state.select(Some(selected + 1));
            self.update_preview();
        }
    }

    /// Reply action — for now just shows a message in the status bar.
    /// Open/select the current item. For mails: opens in a popout panel.
    fn open_item(&mut self) {
        match self.panel {
            Panel::Mails => {
                let selected = self.list_state.selected().unwrap_or(0);
                if let Some(item) = self.results.get(selected) {
                    // Resolve by JMAP email id stored in item.inner
                    if let Some(mail) = self.mails.iter().find(|m| m.id == item.inner) {
                        self.selected_mail = Some(mail.clone());
                        // Build popout body
                        let mut body = vec![
                            ratatui::text::Line::from(format!("From:    {}", mail.from)),
                            ratatui::text::Line::from(format!("Date:    {}", mail.date)),
                            ratatui::text::Line::from(format!("Subject: {}", mail.subject)),
                            ratatui::text::Line::from(""),
                            ratatui::text::Line::from("─".repeat(40)),
                            ratatui::text::Line::from(""),
                        ];
                        for line in mail.preview.lines() {
                            body.push(ratatui::text::Line::from(line.to_string()));
                        }
                        let popout =
                            Popout::email_view(0, mail.subject.clone(), body, mail.id.clone());
                        self.popouts.open(popout);
                        self.input_mode = event::InputMode::EmailOpen;
                    }
                }
            }
            Panel::Folders => {
                self.navigate_right();
            }
            Panel::Profiles => {
                self.navigate_right();
            }
            Panel::Contacts | Panel::Calendar => {
                // Nothing deeper to open
            }
        }
    }

    /// Execute the current input as a slash command or regular search Enter.
    fn execute_input_command(&mut self) {
        if self.input.starts_with('/') {
            let cmd = self.input.trim_start_matches('/').to_lowercase();
            let cmd = cmd.trim().to_string();
            self.execute_slash_command(&cmd);
            self.input.clear();
            self.update_search();
            self.input_mode = event::InputMode::Normal;
        } else {
            // Regular Enter in insert mode: open the selected item
            self.open_item();
            self.input_mode = event::InputMode::Normal;
        }
    }

    /// Execute a slash command by name.
    fn execute_slash_command(&mut self, cmd: &str) {
        match cmd {
            // --- Mail commands ---
            "reply" | "r" => self.slash_reply(),
            "forward" | "f" => self.slash_forward(),
            "archive" | "a" => self.slash_action("archive"),
            "delete" | "del" | "d" => self.slash_action("delete"),
            "spam" | "s" => self.slash_action("spam"),
            "compose" | "new" | "c" => {
                let sig = self.get_signature();
                let popout = Popout::compose(0, sig.as_deref());
                self.popouts.open(popout);
                self.input_mode = event::InputMode::Editing;
            }
            "mark-read" => self.slash_mark_read(true),
            "mark-unread" => self.slash_mark_read(false),
            "mark-folder-read" => self.slash_mark_folder_read(),
            "mark-spam-read" => self.slash_mark_spam_read(),

            // --- Contact commands ---
            "add-contact" | "new-contact" => {
                self.pending_contact_create = true;
                self.needs_reload = true;
            }
            "edit-contact" => {
                self.error_message = Some("edit-contact: select a contact first, then /edit-contact (not yet implemented)".to_string());
            }
            "delete-contact" => {
                self.pending_contact_delete = true;
                self.needs_reload = true;
            }

            // --- Calendar commands ---
            "add-event" | "new-event" => {
                self.pending_event_create = true;
                self.needs_reload = true;
            }
            "edit-event" => {
                self.error_message = Some(
                    "edit-event: select an event first, then /edit-event (not yet implemented)"
                        .to_string(),
                );
            }
            "delete-event" => {
                self.pending_event_delete = true;
                self.needs_reload = true;
            }

            _ => {
                self.error_message = Some(format!("Unknown command: /{cmd}"));
            }
        }
    }

    /// Reply to the selected/open email — opens reply in a popout in edit mode.
    fn slash_reply(&mut self) {
        let mail = self.selected_mail.clone().or_else(|| {
            let sel = self.list_state.selected().unwrap_or(0);
            self.results
                .get(sel)
                .and_then(|item| self.mails.iter().find(|m| m.id == item.inner).cloned())
        });
        if let Some(mail) = mail {
            let sig = self.get_signature();
            let popout = Popout::reply(
                0,
                mail.id.clone(),
                &mail.from,
                &mail.subject,
                &mail.preview,
                sig.as_deref(),
            );
            self.popouts.open(popout);
            self.input_mode = event::InputMode::Editing;
        } else {
            self.error_message = Some("No email selected".to_string());
        }
    }

    /// Forward the selected/open email — opens forward in a popout in edit mode.
    fn slash_forward(&mut self) {
        let mail = self.selected_mail.clone().or_else(|| {
            let sel = self.list_state.selected().unwrap_or(0);
            self.results
                .get(sel)
                .and_then(|item| self.mails.iter().find(|m| m.id == item.inner).cloned())
        });
        if let Some(mail) = mail {
            let sig = self.get_signature();
            let popout = Popout::forward(
                0,
                mail.id.clone(),
                &mail.subject,
                &mail.preview,
                sig.as_deref(),
            );
            self.popouts.open(popout);
            self.input_mode = event::InputMode::Editing;
        } else {
            self.error_message = Some("No email selected".to_string());
        }
    }

    /// Move the selected/open email to a target folder.
    fn slash_action(&mut self, target: &str) {
        if self.panel != Panel::Mails {
            self.error_message = Some("Mail actions only available in Mail view".to_string());
            return;
        }

        // Get the email ID from selected_mail or from the list (resolved by JMAP id)
        let mail_id = self
            .selected_mail
            .as_ref()
            .map(|m| m.id.clone())
            .or_else(|| {
                let sel = self.list_state.selected().unwrap_or(0);
                self.results
                    .get(sel)
                    .and_then(|item| self.mails.iter().find(|m| m.id == item.inner))
                    .map(|m| m.id.clone())
            });

        let Some(mail_id) = mail_id else {
            self.error_message = Some("No email selected".to_string());
            return;
        };

        // Resolve the target folder name from config
        let profile = self.config.profiles.get(&self.active_profile_name);
        let target_folder_name = match target {
            "archive" => profile
                .and_then(|p| p.folders.archive.as_deref())
                .unwrap_or("Archive"),
            "spam" => profile
                .and_then(|p| p.folders.spam.as_deref())
                .unwrap_or("Junk"),
            "delete" | "trash" => profile
                .and_then(|p| p.folders.trash.as_deref())
                .unwrap_or("Trash"),
            _ => {
                self.error_message = Some(format!("Unknown target: {target}"));
                return;
            }
        };

        // Find the target mailbox ID
        let target_folder_id = self
            .folders
            .iter()
            .find(|f| f.name == target_folder_name)
            .map(|f| f.id.clone());

        let Some(target_folder_id) = target_folder_id else {
            self.error_message = Some(format!("Folder not found: {target_folder_name}"));
            return;
        };

        self.pending_move = Some(PendingMove {
            email_id: mail_id,
            target_mailbox_id: target_folder_id,
            source_mailbox_id: self.active_folder_id.clone().unwrap_or_default(),
            action_name: target.to_string(),
        });
        self.selected_mail = None;
        self.input_mode = event::InputMode::Normal;
        self.needs_reload = true;
    }

    /// Transition to Confirm state for a destructive action (delete/archive/spam).
    /// Resolves the email id first; if no email is selected, shows an error.
    fn request_confirm_action(&mut self, target: &str) {
        if self.panel != Panel::Mails {
            self.error_message = Some("Mail actions only available in Mail view".to_string());
            return;
        }

        // Get the email ID from selected_mail or from the list (resolved by JMAP id)
        let mail_id = self
            .selected_mail
            .as_ref()
            .map(|m| m.id.clone())
            .or_else(|| {
                let sel = self.list_state.selected().unwrap_or(0);
                self.results
                    .get(sel)
                    .and_then(|item| self.mails.iter().find(|m| m.id == item.inner))
                    .map(|m| m.id.clone())
            });

        let Some(mail_id) = mail_id else {
            self.error_message = Some("No email selected".to_string());
            return;
        };

        let confirm_action = match target {
            "delete" => event::ConfirmAction::Delete(mail_id),
            "archive" => event::ConfirmAction::Archive(mail_id),
            "spam" => event::ConfirmAction::Spam(mail_id),
            _ => {
                self.error_message = Some(format!("Unknown target: {target}"));
                return;
            }
        };

        // Save current mode so we can restore on cancel
        self.pre_confirm_mode = Some(self.input_mode.clone());
        self.input_mode = event::InputMode::Confirm(confirm_action);
    }

    /// Execute a confirmed destructive action (after user pressed 'y').
    fn execute_confirmed_action(&mut self, target: &str, email_id: &str) {
        // Resolve the target folder name from config
        let profile = self.config.profiles.get(&self.active_profile_name);
        let target_folder_name = match target {
            "archive" => profile
                .and_then(|p| p.folders.archive.as_deref())
                .unwrap_or("Archive"),
            "spam" => profile
                .and_then(|p| p.folders.spam.as_deref())
                .unwrap_or("Junk"),
            "delete" | "trash" => profile
                .and_then(|p| p.folders.trash.as_deref())
                .unwrap_or("Trash"),
            _ => {
                self.error_message = Some(format!("Unknown target: {target}"));
                return;
            }
        };

        // Find the target mailbox ID
        let target_folder_id = self
            .folders
            .iter()
            .find(|f| f.name == target_folder_name)
            .map(|f| f.id.clone());

        let Some(target_folder_id) = target_folder_id else {
            self.error_message = Some(format!("Folder not found: {target_folder_name}"));
            return;
        };

        self.pending_move = Some(PendingMove {
            email_id: email_id.to_string(),
            target_mailbox_id: target_folder_id,
            source_mailbox_id: self.active_folder_id.clone().unwrap_or_default(),
            action_name: target.to_string(),
        });
        self.selected_mail = None;
        self.needs_reload = true;
    }

    /// Mark selected email as read or unread.
    fn slash_mark_read(&mut self, _read: bool) {
        // Will set/remove $seen keyword via Email/set
        self.error_message = Some("mark-read: not yet implemented".to_string());
    }

    /// Mark all emails in the current folder as read.
    fn slash_mark_folder_read(&mut self) {
        if self.panel != Panel::Mails {
            self.error_message = Some("Only available in Mail view".to_string());
            return;
        }
        self.error_message = Some(format!(
            "Marking all in '{}' as read (not yet implemented)",
            self.active_folder_name
        ));
    }

    /// Mark all emails in Spam/Junk as read.
    fn slash_mark_spam_read(&mut self) {
        self.error_message = Some("Marking all spam as read (not yet implemented)".to_string());
    }

    /// Tick the matcher and refresh results.
    fn tick(&mut self) {
        self.matcher.tick();
        self.matcher_running = self.matcher.running;
        self.total_count = self.matcher.total_item_count;
        self.matched_count = self.matcher.matched_item_count;

        self.results = self.matcher.results(self.results_height as u32, 0);

        // Clamp selection
        if let Some(sel) = self.list_state.selected() {
            if sel >= self.results.len() && !self.results.is_empty() {
                self.list_state.select(Some(self.results.len() - 1));
            }
        }

        // Auto-select when there's exactly 1 result and nothing selected
        if self.results.len() == 1 && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }
        // Ensure something is always selected when results exist
        if !self.results.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }

        self.update_preview();
    }

    /// Update the preview based on current panel and selection.
    fn update_preview(&mut self) {
        let selected = self.list_state.selected().unwrap_or(0);

        match self.panel {
            Panel::Profiles => {
                self.preview_content = self.results.get(selected).and_then(|item| {
                    let profile = self.profiles.iter().find(|p| p.name == item.inner)?;
                    Some(PreviewContent {
                        title: profile.name.clone(),
                        body: vec![ratatui::text::Line::from(format!(
                            "Server: {}",
                            profile.server_url
                        ))],
                    })
                });
            }
            Panel::Folders => {
                self.preview_content = self.results.get(selected).and_then(|item| {
                    let folder = self.folders.iter().find(|f| f.id == item.inner)?;
                    Some(PreviewContent {
                        title: folder.name.clone(),
                        body: vec![
                            ratatui::text::Line::from(format!("ID: {}", folder.id)),
                            ratatui::text::Line::from(format!(
                                "Role: {}",
                                folder.role.as_deref().unwrap_or("-")
                            )),
                            ratatui::text::Line::from(format!("Total: {}", folder.total_emails)),
                            ratatui::text::Line::from(format!("Unread: {}", folder.unread_emails)),
                        ],
                    })
                });
            }
            Panel::Mails => {
                // If an email is open, show it prominently
                if let Some(mail) = &self.selected_mail {
                    let mut body = vec![
                        ratatui::text::Line::from(ratatui::text::Span::styled(
                            "📧 EMAIL OPEN",
                            ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
                        )),
                        ratatui::text::Line::from(""),
                        ratatui::text::Line::from(format!("From:    {}", mail.from)),
                        ratatui::text::Line::from(format!("Date:    {}", mail.date)),
                        ratatui::text::Line::from(format!("Subject: {}", mail.subject)),
                        ratatui::text::Line::from(""),
                        ratatui::text::Line::from("─".repeat(40)),
                        ratatui::text::Line::from(""),
                    ];
                    for line in mail.preview.lines() {
                        body.push(ratatui::text::Line::from(line.to_string()));
                    }
                    self.preview_content = Some(PreviewContent {
                        title: mail.subject.clone(),
                        body,
                    });
                } else {
                    self.preview_content = self.results.get(selected).and_then(|item| {
                        let mail = self.mails.iter().find(|m| m.id == item.inner)?;
                        let mut body = vec![
                            ratatui::text::Line::from(format!("From: {}", mail.from)),
                            ratatui::text::Line::from(format!("Date: {}", mail.date)),
                            ratatui::text::Line::from(format!("Subject: {}", mail.subject)),
                            ratatui::text::Line::from(""),
                            ratatui::text::Line::from("─".repeat(40)),
                            ratatui::text::Line::from(""),
                        ];
                        for line in mail.preview.lines() {
                            body.push(ratatui::text::Line::from(line.to_string()));
                        }
                        Some(PreviewContent {
                            title: mail.subject.clone(),
                            body,
                        })
                    });
                }
            }
            Panel::Contacts => {
                self.preview_content = self.results.get(selected).and_then(|item| {
                    let contact = self.contacts.iter().find(|c| c.id == item.inner)?;
                    Some(PreviewContent {
                        title: contact.name.clone(),
                        body: vec![
                            ratatui::text::Line::from(format!("Name: {}", contact.name)),
                            ratatui::text::Line::from(format!("Email: {}", contact.email)),
                            ratatui::text::Line::from(format!("Phone: {}", contact.phone)),
                        ],
                    })
                });
            }
            Panel::Calendar => {
                self.preview_content = self.results.get(selected).and_then(|item| {
                    let event = self.events.iter().find(|e| e.id == item.inner)?;
                    Some(PreviewContent {
                        title: event.title.clone(),
                        body: vec![
                            ratatui::text::Line::from(format!("Title: {}", event.title)),
                            ratatui::text::Line::from(format!("Start: {}", event.start)),
                            ratatui::text::Line::from(format!("Duration: {}", event.duration)),
                            ratatui::text::Line::from(format!("Status: {}", event.status)),
                        ],
                    })
                });
            }
        }
    }

    /// Inject items into the matcher based on the current panel.
    ///
    /// For each item type, we store the stable JMAP identifier as `inner`
    /// so that resolution uses the id rather than the display string.
    fn inject_items(&self) {
        let injector = self.matcher.injector();
        match self.panel {
            Panel::Profiles => {
                for profile in &self.profiles {
                    let display = profile.name.clone();
                    // Profile name IS the unique identifier
                    injector.push(display.clone(), |item, cols| {
                        cols[0] = item.as_str().into();
                    });
                }
            }
            Panel::Folders => {
                for folder in &self.folders {
                    let display = format_folder(folder);
                    let id = folder.id.clone(); // JMAP mailbox id
                    injector.push(id, |_item, cols| {
                        cols[0] = display.as_str().into();
                    });
                }
            }
            Panel::Mails => {
                for mail in &self.mails {
                    let display = format_mail(mail);
                    let id = mail.id.clone(); // JMAP email id
                    injector.push(id, |_item, cols| {
                        cols[0] = display.as_str().into();
                    });
                }
            }
            Panel::Contacts => {
                for contact in &self.contacts {
                    let display = format_contact(contact);
                    let id = contact.id.clone(); // JMAP contact id
                    injector.push(id, |_item, cols| {
                        cols[0] = display.as_str().into();
                    });
                }
            }
            Panel::Calendar => {
                for event in &self.events {
                    let display = format_event(event);
                    let id = event.id.clone(); // JMAP event id
                    injector.push(id, |_item, cols| {
                        cols[0] = display.as_str().into();
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Display formatting helpers
// ---------------------------------------------------------------------------

fn format_folder(f: &FolderEntry) -> String {
    let role_str = f.role.as_deref().unwrap_or("");
    let unread = if f.unread_emails > 0 {
        format!(" •{}", f.unread_emails)
    } else {
        String::new()
    };
    if role_str.is_empty() {
        format!("{}  ({}{})", f.display_name, f.total_emails, unread)
    } else {
        format!(
            "{}  [{}]  ({}{})",
            f.display_name, role_str, f.total_emails, unread
        )
    }
}

fn format_mail(m: &MailEntry) -> String {
    format!("{} — {}", m.from, m.subject)
}

fn format_contact(c: &ContactEntry) -> String {
    if c.email.is_empty() {
        c.name.clone()
    } else {
        format!("{} <{}>", c.name, c.email)
    }
}

fn format_event(e: &CalendarEventEntry) -> String {
    if e.start.is_empty() {
        e.title.clone()
    } else {
        format!("{} — {}", e.start, e.title)
    }
}

// ---------------------------------------------------------------------------
// Entry point and event loop
// ---------------------------------------------------------------------------

/// Entry point: initialize terminal, run the event loop, restore terminal.
pub fn run(config: Config, profile_name: Option<&str>) -> io::Result<()> {
    // We're already inside a tokio runtime (from #[tokio::main]),
    // so use block_in_place to run the synchronous TUI loop without
    // conflicting with the outer runtime.
    tokio::task::block_in_place(|| {
        let mut terminal = ratatui::init();
        let result = run_loop(&mut terminal, config, profile_name);
        ratatui::restore();
        result
    })
}

/// Main event loop with async JMAP data fetching.
fn run_loop(
    terminal: &mut DefaultTerminal,
    config: Config,
    profile_name: Option<&str>,
) -> io::Result<()> {
    let handle = tokio::runtime::Handle::current();

    let mut app = App::new(config, profile_name);

    // Initial data load: connect to default profile and load inbox
    handle.block_on(async {
        load_data_for_panel(&mut app).await;
    });
    app.inject_items();
    app.matcher.find(&app.input);
    app.matcher.tick();

    let tick_rate = Duration::from_millis(50);

    loop {
        // Draw
        terminal.draw(|frame| {
            let visible_count = app.popouts.visible_popouts().len();
            let has_maximized = app.popouts.has_maximized();
            let has_minimized = !app.popouts.minimized_popouts().is_empty();

            let layout = Layout::build(frame.area(), visible_count, has_maximized, has_minimized);
            app.results_height = layout.results.height.saturating_sub(2);

            // Draw main panels (unless maximized popout hides everything)
            if !has_maximized {
                results::draw(frame, &mut app, layout.results);
                input::draw(frame, &app, layout.input);
                if let Some(preview_area) = layout.preview {
                    preview::draw(frame, &app, preview_area);
                }
            }

            // Draw popout panels
            let visible = app.popouts.visible_popouts();
            for (i, popout) in visible.iter().enumerate() {
                if let Some(area) = layout.popout_areas.get(i) {
                    draw_popout(frame, popout, app.popouts.focused == Some(i), *area);
                }
            }

            // Draw minimized tab bar
            if let Some(min_area) = layout.minimized_bar {
                draw_minimized_bar(frame, &app.popouts, min_area);
            }

            draw_status_bar(frame, layout.status_bar, &app);

            // Quit confirmation popup
            if app.show_quit_confirm {
                draw_quit_confirm(frame, frame.area());
            }

            // Cursor management based on mode
            match app.input_mode {
                event::InputMode::Editing => {
                    // Show cursor inside the focused popout at the actual edit position
                    if let Some(idx) = app.popouts.focused {
                        if let Some(area) = layout.popout_areas.get(idx) {
                            let popout = &app.popouts.popouts[idx];
                            let inner_x = area.x + 1; // 1 for border
                            let inner_y = area.y + 1; // 1 for border

                            if let Some(field) = popout.editing_field {
                                // Editing a header field — position cursor at end of that field
                                use super::popout::HeaderField;
                                let (field_row, field_text) = match field {
                                    HeaderField::To => (0, &popout.to),
                                    HeaderField::Subject => {
                                        // Subject is after To + optional CC/BCC
                                        let mut row = 1;
                                        if !popout.cc.is_empty()
                                            || popout.editing_field == Some(HeaderField::Cc)
                                        {
                                            row += 1;
                                        }
                                        if !popout.bcc.is_empty()
                                            || popout.editing_field == Some(HeaderField::Bcc)
                                        {
                                            row += 1;
                                        }
                                        (row, &popout.subject)
                                    }
                                    HeaderField::Cc => (1, &popout.cc),
                                    HeaderField::Bcc => {
                                        let row = if !popout.cc.is_empty()
                                            || popout.editing_field == Some(HeaderField::Cc)
                                        {
                                            2
                                        } else {
                                            1
                                        };
                                        (row, &popout.bcc)
                                    }
                                };
                                // "▶ To:      " = 11 chars prefix
                                let prefix_len = 11u16;
                                let cursor_x = inner_x + prefix_len + field_text.len() as u16;
                                let cursor_y = inner_y + field_row as u16;
                                frame.set_cursor_position((
                                    cursor_x.min(area.right().saturating_sub(2)),
                                    cursor_y.min(area.bottom().saturating_sub(2)),
                                ));
                            } else {
                                // Editing body — find cursor position from editor_cursor byte offset
                                let text_before_cursor =
                                    &popout.editor_buffer[..popout.editor_cursor];
                                let lines_before: Vec<&str> = text_before_cursor.lines().collect();
                                let cursor_line = if text_before_cursor.ends_with('\n') {
                                    lines_before.len()
                                } else {
                                    lines_before.len().saturating_sub(1)
                                };
                                let cursor_col = lines_before.last().map(|l| l.len()).unwrap_or(0);

                                // Header takes: To + optional CC/BCC + Subject + empty + separator + empty = variable rows
                                let header_rows = popout
                                    .body
                                    .iter()
                                    .position(|l| l.to_string().starts_with('─'))
                                    .map(|i| i + 2) // separator + empty line after
                                    .unwrap_or(4)
                                    as u16;

                                let cursor_x = inner_x + cursor_col as u16;
                                let cursor_y = inner_y + header_rows + cursor_line as u16;
                                frame.set_cursor_position((
                                    cursor_x.min(area.right().saturating_sub(2)),
                                    cursor_y.min(area.bottom().saturating_sub(2)),
                                ));
                            }
                        }
                    }
                }
                event::InputMode::Insert => {
                    // Cursor is set in input::draw already
                }
                event::InputMode::Normal | event::InputMode::EmailOpen => {
                    // No cursor shown
                }
                event::InputMode::Confirm(_) => {
                    // No cursor shown during confirmation
                }
            }
        })?;

        // Set cursor shape based on mode
        match app.input_mode {
            event::InputMode::Editing => {
                use crossterm::cursor::SetCursorStyle;
                use crossterm::execute;
                let _ = execute!(std::io::stdout(), SetCursorStyle::BlinkingBar);
            }
            event::InputMode::Insert => {
                use crossterm::cursor::SetCursorStyle;
                use crossterm::execute;
                let _ = execute!(std::io::stdout(), SetCursorStyle::BlinkingBar);
            }
            event::InputMode::Normal | event::InputMode::EmailOpen => {
                use crossterm::cursor::SetCursorStyle;
                use crossterm::execute;
                let _ = execute!(std::io::stdout(), SetCursorStyle::BlinkingBlock);
            }
            event::InputMode::Confirm(_) => {
                use crossterm::cursor::SetCursorStyle;
                use crossterm::execute;
                let _ = execute!(std::io::stdout(), SetCursorStyle::BlinkingBlock);
            }
        }

        // Events
        if let Some(Event::Key(key)) = event::poll_event(tick_rate)? {
            let action = event::map_key(key, &app.input_mode);
            let panel_before = app.panel;
            app.handle_action(action);

            // If panel changed, reload data
            if app.panel != panel_before {
                handle.block_on(async {
                    load_data_for_panel(&mut app).await;
                });
                app.inject_items();
                // Force the matcher to start matching all items
                app.matcher.find(&app.input);
                // Give nucleo workers a moment to process
                app.matcher.tick();
            }

            // If a mail action was performed, reload mails
            if app.needs_reload {
                app.needs_reload = false;
                // Execute pending compose (create draft)
                if app.pending_compose {
                    app.pending_compose = false;
                    handle.block_on(async {
                        if let Err(e) = execute_compose_draft(
                            &app.client,
                            &app.config,
                            &app.active_profile_name,
                            &app.folders,
                        )
                        .await
                        {
                            app.error_message = Some(format!("Draft error: {e}"));
                        } else {
                            app.error_message = Some("✓ Draft created".to_string());
                        }
                    });
                }
                // Execute pending mail move if any
                if let Some(pending) = app.pending_move.take() {
                    handle.block_on(async {
                        if let Err(e) = execute_mail_move(&app.client, &pending).await {
                            app.error_message = Some(format!("{}: {e}", pending.action_name));
                        }
                    });
                }
                // Execute pending contact create
                if app.pending_contact_create {
                    app.pending_contact_create = false;
                    handle.block_on(async {
                        if let Err(e) = execute_contact_create(&app.client).await {
                            app.error_message = Some(format!("Contact error: {e}"));
                        } else {
                            app.error_message = Some("✓ Contact created".to_string());
                        }
                    });
                }
                // Execute pending contact delete
                if app.pending_contact_delete {
                    app.pending_contact_delete = false;
                    let contact_id = app.list_state.selected().unwrap_or(0);
                    let id = app.contacts.get(contact_id).map(|c| c.id.clone());
                    if let Some(id) = id {
                        handle.block_on(async {
                            if let Err(e) = execute_contact_delete(&app.client, &id).await {
                                app.error_message = Some(format!("Delete error: {e}"));
                            } else {
                                app.error_message = Some("✓ Contact deleted".to_string());
                            }
                        });
                    }
                }
                // Execute pending event create
                if app.pending_event_create {
                    app.pending_event_create = false;
                    handle.block_on(async {
                        if let Err(e) = execute_event_create(&app.client).await {
                            app.error_message = Some(format!("Event error: {e}"));
                        } else {
                            app.error_message = Some("✓ Event created".to_string());
                        }
                    });
                }
                // Execute pending event delete
                if app.pending_event_delete {
                    app.pending_event_delete = false;
                    let event_idx = app.list_state.selected().unwrap_or(0);
                    let id = app.events.get(event_idx).map(|e| e.id.clone());
                    if let Some(id) = id {
                        handle.block_on(async {
                            if let Err(e) = execute_event_delete(&app.client, &id).await {
                                app.error_message = Some(format!("Delete error: {e}"));
                            } else {
                                app.error_message = Some("✓ Event deleted".to_string());
                            }
                        });
                    }
                }
                handle.block_on(async {
                    load_data_for_panel(&mut app).await;
                });
                app.matcher = Matcher::new();
                app.inject_items();
                app.matcher.find(&app.input);
                app.matcher.tick();
            }
        }

        // Tick the matcher
        app.tick();

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Load data from JMAP for the current panel.
async fn load_data_for_panel(app: &mut App) {
    app.loading = true;
    app.error_message = None;

    match app.panel {
        Panel::Profiles => {
            // Profiles are already loaded from config, nothing async needed
        }
        Panel::Folders => {
            // Ensure we have a client for the active profile
            if app.client.is_none() {
                if let Err(e) = connect_profile(app).await {
                    app.error_message = Some(format!("Auth error: {e}"));
                    app.loading = false;
                    return;
                }
            }
            // Fetch mailboxes
            if let Some(client) = &app.client {
                match fetch_folders(client).await {
                    Ok(folders) => app.folders = folders,
                    Err(e) => app.error_message = Some(format!("Mailbox error: {e}")),
                }
            }
        }
        Panel::Mails => {
            // Ensure we have a client
            if app.client.is_none() {
                if let Err(e) = connect_profile(app).await {
                    app.error_message = Some(format!("Auth error: {e}"));
                    app.loading = false;
                    return;
                }
            }
            // If no folder selected, find inbox
            if app.active_folder_id.is_none() {
                if let Some(client) = &app.client {
                    if let Ok(folders) = fetch_folders(client).await {
                        if let Some(inbox) =
                            folders.iter().find(|f| f.role.as_deref() == Some("inbox"))
                        {
                            app.active_folder_id = Some(inbox.id.clone());
                            app.active_folder_name = inbox.name.clone();
                        } else if let Some(first) = folders.first() {
                            app.active_folder_id = Some(first.id.clone());
                            app.active_folder_name = first.name.clone();
                        }
                        app.folders = folders;
                    }
                }
            }
            // Fetch mails
            if let (Some(client), Some(folder_id)) = (&app.client, &app.active_folder_id) {
                match fetch_mails(client, folder_id).await {
                    Ok(mails) => app.mails = mails,
                    Err(e) => app.error_message = Some(format!("Mail error: {e}")),
                }
            }
        }
        Panel::Contacts => {
            if app.client.is_none() {
                if let Err(e) = connect_profile(app).await {
                    app.error_message = Some(format!("Auth error: {e}"));
                    app.loading = false;
                    return;
                }
            }
            if let Some(client) = &app.client {
                match fetch_contacts(client).await {
                    Ok(contacts) => app.contacts = contacts,
                    Err(e) => app.error_message = Some(format!("Contacts error: {e}")),
                }
            }
        }
        Panel::Calendar => {
            if app.client.is_none() {
                if let Err(e) = connect_profile(app).await {
                    app.error_message = Some(format!("Auth error: {e}"));
                    app.loading = false;
                    return;
                }
            }
            if let Some(client) = &app.client {
                match fetch_events(client).await {
                    Ok(events) => app.events = events,
                    Err(e) => app.error_message = Some(format!("Calendar error: {e}")),
                }
            }
        }
    }

    app.loading = false;
}

/// Connect to the active profile.
async fn connect_profile(app: &mut App) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let profile = app
        .config
        .get_profile(Some(&app.active_profile_name))
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

    let client = auth::create_client(profile, &app.active_profile_name).await?;
    app.client = Some(client);
    Ok(())
}

/// Fetch mailbox list via JMAP and build a tree-ordered list.
async fn fetch_folders(
    client: &JmapClient,
) -> Result<Vec<FolderEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session);
    let resp = sc.mailbox_get(None, None).await?;

    // Build flat entries first
    let raw: Vec<_> = resp
        .list
        .iter()
        .map(|m| {
            (
                m.id.as_ref().to_string(),
                m.name.clone(),
                m.parent_id.as_ref().map(|p| p.as_ref().to_string()),
                m.role.as_ref().map(|r| r.to_wire_str().to_string()),
                m.sort_order,
                m.total_emails,
                m.unread_emails,
            )
        })
        .collect();

    // Build tree-ordered output using DFS
    let mut result = Vec::with_capacity(raw.len());
    build_folder_tree(&raw, None, 0, &mut result);

    Ok(result)
}

/// Recursively build a tree-ordered folder list via DFS.
#[allow(clippy::type_complexity)]
fn build_folder_tree(
    raw: &[(
        String,
        String,
        Option<String>,
        Option<String>,
        u32,
        u32,
        u32,
    )],
    parent_id: Option<&str>,
    depth: usize,
    out: &mut Vec<FolderEntry>,
) {
    // Find children of this parent
    let mut children: Vec<_> = raw
        .iter()
        .filter(|(_, _, pid, _, _, _, _)| pid.as_deref() == parent_id)
        .collect();

    // Sort: default role folders first (in canonical order), then by sort_order, then name
    children.sort_by(|a, b| {
        let priority_a = role_priority(a.3.as_deref());
        let priority_b = role_priority(b.3.as_deref());
        priority_a
            .cmp(&priority_b)
            .then_with(|| a.4.cmp(&b.4))
            .then_with(|| a.1.cmp(&b.1))
    });

    for (id, name, pid, role, sort_order, total, unread) in children {
        // Build indented display name
        let indent = "  ".repeat(depth);
        let prefix = if depth > 0 { "└ " } else { "" };
        let display_name = format!("{indent}{prefix}{name}");

        out.push(FolderEntry {
            id: id.clone(),
            name: name.clone(),
            parent_id: pid.clone(),
            role: role.clone(),
            sort_order: *sort_order,
            total_emails: *total,
            unread_emails: *unread,
            display_name,
            depth,
        });

        // Recurse into children
        build_folder_tree(raw, Some(id.as_str()), depth + 1, out);
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

/// Fetch recent emails in a folder via JMAP.
async fn fetch_mails(
    client: &JmapClient,
    folder_id: &str,
) -> Result<Vec<MailEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session.clone());

    // Query for the 50 most recent emails in this folder
    let filter = json!({ "inMailbox": folder_id });
    let sort = json!([{ "property": "receivedAt", "isAscending": false }]);
    let query_resp = sc
        .email_query(Some(filter), Some(sort), Some(0), Some(50), None)
        .await?;

    if query_resp.ids.is_empty() {
        return Ok(Vec::new());
    }

    // Fetch email details — include all required non-optional fields to avoid parse errors
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

            let subject = e.subject.as_deref().unwrap_or("(no subject)").to_string();
            let date = e.received_at.as_ref().to_string();
            let preview_text = e.preview.as_deref().unwrap_or("").to_string();

            MailEntry {
                id: e.id.as_ref().to_string(),
                subject,
                from,
                date,
                preview: preview_text,
            }
        })
        .collect();

    Ok(mails)
}

/// Fetch contacts via JMAP.
async fn fetch_contacts(
    client: &JmapClient,
) -> Result<Vec<ContactEntry>, Box<dyn std::error::Error + Send + Sync>> {
    use jmap_contacts_client::JmapContactsExt;

    let session = client.fetch_session().await?;
    let sc = client.with_contacts_session(session);
    let resp = sc
        .contact_card_get(None, Some(&["id", "name", "emails", "phones"]))
        .await?;

    let contacts = resp
        .list
        .iter()
        .map(|card| {
            let id = card
                .id
                .as_ref()
                .map(|i| i.as_ref().to_string())
                .unwrap_or_default();
            let name = extract_contact_name(card);
            let email = extract_first_email(card);
            let phone = extract_first_phone(card);
            ContactEntry {
                id,
                name,
                email,
                phone,
            }
        })
        .collect();

    Ok(contacts)
}

/// Extract display name from a ContactCard.
fn extract_contact_name(card: &jmap_contacts_types::ContactCard) -> String {
    if let Some(name_val) = &card.name {
        if let Some(full) = name_val.get("full") {
            if let Some(s) = full.as_str() {
                return s.to_string();
            }
        }
        if let Some(components) = name_val.get("components") {
            if let Some(arr) = components.as_array() {
                let parts: Vec<&str> = arr
                    .iter()
                    .filter_map(|c| c.get("value").and_then(|v| v.as_str()))
                    .collect();
                if !parts.is_empty() {
                    return parts.join(" ");
                }
            }
        }
    }
    "(no name)".to_string()
}

/// Extract first email from a ContactCard.
fn extract_first_email(card: &jmap_contacts_types::ContactCard) -> String {
    if let Some(emails_val) = &card.emails {
        if let Some(obj) = emails_val.as_object() {
            for (_key, email_obj) in obj {
                if let Some(addr) = email_obj.get("address").and_then(|v| v.as_str()) {
                    return addr.to_string();
                }
            }
        }
    }
    String::new()
}

/// Extract first phone from a ContactCard.
fn extract_first_phone(card: &jmap_contacts_types::ContactCard) -> String {
    if let Some(phones_val) = &card.phones {
        if let Some(obj) = phones_val.as_object() {
            for (_key, phone_obj) in obj {
                if let Some(number) = phone_obj.get("number").and_then(|v| v.as_str()) {
                    return number.to_string();
                }
            }
        }
    }
    String::new()
}

/// Fetch calendar events via JMAP.
async fn fetch_events(
    client: &JmapClient,
) -> Result<Vec<CalendarEventEntry>, Box<dyn std::error::Error + Send + Sync>> {
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
        .map(|e| {
            let id =
                e.id.as_ref()
                    .map(|i| i.as_ref().to_string())
                    .unwrap_or_default();
            let title = e.title.as_deref().unwrap_or("(no title)").to_string();
            let start = e.start.as_deref().unwrap_or("").to_string();
            let duration = e.duration.as_deref().unwrap_or("").to_string();
            let status = e.status.as_deref().unwrap_or("").to_string();
            CalendarEventEntry {
                id,
                title,
                start,
                duration,
                status,
            }
        })
        .collect();

    Ok(events)
}

/// Draw a centered quit confirmation popup.
fn draw_quit_confirm(frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};

    // Center a small popup
    let popup_width = 40u16.min(area.width.saturating_sub(4));
    let popup_height = 5u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Quit Herald? ");

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Enter",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" quit    "),
            Span::styled("any key", Style::default().fg(Color::DarkGray)),
            Span::raw(" cancel"),
        ]),
    ];

    let paragraph = Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(Color::White));

    frame.render_widget(paragraph, popup_area);
}

/// Update a popout's body to reflect the current editor_buffer and header fields.
fn update_editor_body(popout: &mut Popout) {
    use super::popout::HeaderField;

    popout.body.clear();

    // Render header fields with active indicator
    let active = popout.editing_field;

    let to_prefix = if active == Some(HeaderField::To) {
        "▶ To:      "
    } else {
        "  To:      "
    };
    let subj_prefix = if active == Some(HeaderField::Subject) {
        "▶ Subject: "
    } else {
        "  Subject: "
    };

    popout.body.push(ratatui::text::Line::from(format!(
        "{to_prefix}{}",
        popout.to
    )));

    // Show CC/BCC only if they have content or are being edited
    if !popout.cc.is_empty() || active == Some(HeaderField::Cc) {
        let cc_prefix = if active == Some(HeaderField::Cc) {
            "▶ Cc:      "
        } else {
            "  Cc:      "
        };
        popout.body.push(ratatui::text::Line::from(format!(
            "{cc_prefix}{}",
            popout.cc
        )));
    }
    if !popout.bcc.is_empty() || active == Some(HeaderField::Bcc) {
        let bcc_prefix = if active == Some(HeaderField::Bcc) {
            "▶ Bcc:     "
        } else {
            "  Bcc:     "
        };
        popout.body.push(ratatui::text::Line::from(format!(
            "{bcc_prefix}{}",
            popout.bcc
        )));
    }

    popout.body.push(ratatui::text::Line::from(format!(
        "{subj_prefix}{}",
        popout.subject
    )));
    popout.body.push(ratatui::text::Line::from(""));
    popout.body.push(ratatui::text::Line::from("─".repeat(30)));
    popout.body.push(ratatui::text::Line::from(""));

    // Render editor buffer content
    let lines: Vec<&str> = popout.editor_buffer.lines().collect();

    for line in &lines {
        let styled_line =
            if line.starts_with("> ") || line.starts_with("-- ") || line.starts_with("---") {
                ratatui::text::Line::from(ratatui::text::Span::styled(
                    line.to_string(),
                    ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray),
                ))
            } else {
                ratatui::text::Line::from(line.to_string())
            };
        popout.body.push(styled_line);
    }

    // Show cursor when editing body (not a header field) and buffer is empty
    if active.is_none() && popout.editor_buffer.is_empty() {
        popout
            .body
            .push(ratatui::text::Line::from(ratatui::text::Span::styled(
                "▊",
                ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
            )));
    }
}

/// Draw a single popout panel.
fn draw_popout(
    frame: &mut ratatui::Frame,
    panel: &Popout,
    is_focused: bool,
    area: ratatui::layout::Rect,
) {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let title_style = if is_focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let kind_indicator = match &panel.kind {
        PopoutKind::EmailView { .. } => "📧",
        PopoutKind::Compose => "✏️",
        PopoutKind::Reply { .. } => "↩️",
        PopoutKind::Forward { .. } => "➡️",
    };

    let title = format!(" {kind_indicator} {} ", panel.title);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(ratatui::text::Span::styled(title, title_style));

    let paragraph = Paragraph::new(panel.body.clone())
        .block(block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));

    frame.render_widget(paragraph, area);
}

/// Draw the minimized popout tab bar.
fn draw_minimized_bar(
    frame: &mut ratatui::Frame,
    manager: &PopoutManager,
    area: ratatui::layout::Rect,
) {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;

    let mut spans: Vec<Span<'_>> = vec![Span::raw(" ")];
    for popout in manager.minimized_popouts() {
        let kind = match &popout.kind {
            PopoutKind::EmailView { .. } => "📧",
            PopoutKind::Compose => "✏️",
            PopoutKind::Reply { .. } => "↩️",
            PopoutKind::Forward { .. } => "➡️",
        };
        spans.push(Span::styled(
            format!(" {kind} {} ", truncate_title(&popout.title, 20)),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
        spans.push(Span::raw("│"));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Truncate a title to max_len with ellipsis.
fn truncate_title(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max_len - 1).collect();
        t.push('…');
        t
    }
}

/// Draw the bottom status bar.
fn draw_status_bar(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;

    // Mode tabs indicator
    let mut spans: Vec<Span<'_>> = vec![Span::raw(" ")];
    for mode in Mode::all() {
        let style = if *mode == app.mode {
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(format!(" {} ", mode.label()), style));
    }
    spans.push(Span::raw("  "));

    match app.input_mode {
        event::InputMode::Normal => {
            spans.push(Span::styled(
                "q",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" quit  "));
            spans.push(Span::styled(
                "j/k",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" nav  "));
            spans.push(Span::styled(
                "h/l",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" drill  "));
            spans.push(Span::styled(
                "Enter",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" open  "));
            spans.push(Span::styled(
                "/",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" cmd  "));
            if app.panel == Panel::Mails {
                spans.push(Span::styled(
                    "c",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" new  "));
                spans.push(Span::styled(
                    "r",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" reply  "));
                spans.push(Span::styled(
                    "f",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" fwd  "));
                spans.push(Span::styled(
                    "a",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" arch  "));
                spans.push(Span::styled(
                    "d",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" del  "));
                spans.push(Span::styled(
                    "s",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" spam"));
            }
        }
        event::InputMode::Insert => {
            spans.push(Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" back  "));
            spans.push(Span::styled(
                "↑↓",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" nav  "));
            spans.push(Span::raw("  "));
            spans.push(Span::styled("/", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw("reply "));
            spans.push(Span::styled("/", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw("archive "));
            spans.push(Span::styled("/", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw("delete "));
            spans.push(Span::styled("/", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw("compose "));
            spans.push(Span::styled("/", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw("mark-folder-read"));
        }
        event::InputMode::EmailOpen => {
            spans.push(Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" close  "));
            spans.push(Span::styled(
                "s",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" send  "));
            spans.push(Span::styled(
                "d",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" discard  "));
            spans.push(Span::styled(
                "i",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" edit  "));
            spans.push(Span::styled(
                "r",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" reply  "));
            spans.push(Span::styled(
                "f",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" fwd  "));
            spans.push(Span::styled(
                "a",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" archive  "));
            spans.push(Span::styled(
                "m",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" max  "));
            spans.push(Span::styled(
                "Tab",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" switch"));
        }
        event::InputMode::Editing => {
            spans.push(Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" save draft & close  "));
            spans.push(Span::raw("Type to compose..."));
        }
        event::InputMode::Confirm(ref action) => {
            spans.push(Span::styled(
                format!(" ⚠ {} ", action.prompt()),
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "y",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" confirm  "));
            spans.push(Span::styled(
                "n/Esc",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" cancel"));
        }
    }

    if let Some(err) = &app.error_message {
        spans.push(Span::styled(
            format!("  ⚠ {err}"),
            Style::default().fg(Color::Red),
        ));
    }

    if app.loading {
        spans.push(Span::styled(
            "  ⏳ loading...",
            Style::default().fg(Color::Green),
        ));
    }

    let status = Line::from(spans);
    frame.render_widget(Paragraph::new(status), area);
}

/// Execute a mail move (Email/set with mailboxIds update) via JMAP.
async fn execute_mail_move(
    client: &Option<JmapClient>,
    pending: &PendingMove,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = client.as_ref().ok_or("no JMAP client")?;
    let session = client.fetch_session().await?;

    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:mail")
        .ok_or("no primary mail account in session")?;

    // Build Email/set to move the email:
    // Remove from source mailbox, add to target mailbox
    let update_patch = json!({
        format!("mailboxIds/{}", pending.source_mailbox_id): null,
        format!("mailboxIds/{}", pending.target_mailbox_id): true
    });

    let request_args = json!({
        "accountId": account_id,
        "update": {
            pending.email_id.clone(): update_patch
        }
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

    // Check for errors
    for (method_name, result, _call_id) in &resp.method_responses {
        if method_name == "error" {
            let error_type = result["type"].as_str().unwrap_or("unknown");
            let description = result["description"].as_str().unwrap_or("");
            return Err(format!("JMAP error: {error_type} — {description}").into());
        }
        if method_name == "Email/set" {
            if let Some(not_updated) = result["notUpdated"].as_object() {
                if let Some((_key, err)) = not_updated.iter().next() {
                    let err_type = err["type"].as_str().unwrap_or("unknown");
                    let err_desc = err["description"].as_str().unwrap_or("");
                    return Err(format!("Move failed: {err_type} — {err_desc}").into());
                }
            }
        }
    }

    Ok(())
}

/// Create a new draft email via JMAP (Email/import into Drafts with $draft keyword).
async fn execute_compose_draft(
    client: &Option<JmapClient>,
    config: &Config,
    profile_name: &str,
    folders: &[FolderEntry],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use jmap_base_client::UploadBlobParams;
    use jmap_mail_client::{EmailImportInput, JmapMailExt};

    let client = client.as_ref().ok_or("no JMAP client")?;
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session.clone());

    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:mail")
        .ok_or("no primary mail account in session")?;

    // Get the From address from profile config
    let profile = config.profiles.get(profile_name);
    let from_email = profile
        .and_then(|p| p.from_email.as_deref())
        .unwrap_or("user@example.com");
    let from_name = profile.and_then(|p| p.from_name.as_deref()).unwrap_or("");

    let from_header = if from_name.is_empty() {
        from_email.to_string()
    } else {
        format!("{from_name} <{from_email}>")
    };

    // Build a minimal RFC 5322 draft message
    let rfc5322 = format!(
        "From: {from_header}\r\n\
         To: \r\n\
         Subject: (New Draft)\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Transfer-Encoding: 8bit\r\n\
         \r\n\
         \r\n"
    );

    // Upload blob
    let blob_resp = client
        .upload_blob(UploadBlobParams {
            upload_url_template: &session.upload_url,
            account_id,
            content_type: "message/rfc822",
            data: bytes::Bytes::from(rfc5322.into_bytes()),
        })
        .await?;

    // Find the Drafts mailbox
    let drafts_id = folders
        .iter()
        .find(|f| f.role.as_deref() == Some("drafts"))
        .or_else(|| {
            folders
                .iter()
                .find(|f| f.name.to_lowercase().contains("draft"))
        })
        .map(|f| f.id.clone());

    let drafts_id = match drafts_id {
        Some(id) => id,
        None => {
            // Fall back: fetch mailboxes to find drafts
            let mailboxes = sc.mailbox_get(None, None).await?;
            mailboxes
                .list
                .iter()
                .find(|m| m.role.as_ref().is_some_and(|r| r.to_wire_str() == "drafts"))
                .map(|m| m.id.as_ref().to_string())
                .ok_or("no Drafts mailbox found")?
        }
    };

    let mailbox_ids: Vec<jmap_types::Id> = vec![jmap_types::Id::from(drafts_id.as_str())];

    // Import as draft
    let mut emails_map: std::collections::HashMap<String, EmailImportInput<'_>> =
        std::collections::HashMap::new();
    emails_map.insert(
        "draft1".to_string(),
        EmailImportInput {
            blob_id: &blob_resp.blob_id,
            mailbox_ids: &mailbox_ids,
            keywords: Some(&["$draft", "$seen"]),
            received_at: None,
            extra: serde_json::Map::new(),
        },
    );

    let import_resp = sc.email_import(&emails_map, None).await?;

    // Check for errors
    if let Some(not_created) = &import_resp.not_created {
        if let Some((_key, err)) = not_created.iter().next() {
            return Err(format!("Draft creation failed: {:?}", err).into());
        }
    }

    Ok(())
}

/// Create a new contact via JMAP (ContactCard/set).
async fn execute_contact_create(
    client: &Option<JmapClient>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use jmap_contacts_client::JmapContactsExt;

    let client = client.as_ref().ok_or("no JMAP client")?;
    let session = client.fetch_session().await?;
    let sc = client.with_contacts_session(session);

    // Create a blank contact card
    let create = json!({
        "new1": {
            "name": { "full": "(New Contact)" }
        }
    });

    let resp = sc.contact_card_set(Some(create), None, None).await?;

    if let Some(not_created) = &resp.not_created {
        if let Some((_key, err)) = not_created.iter().next() {
            return Err(format!("Contact creation failed: {:?}", err).into());
        }
    }

    Ok(())
}

/// Delete a contact via JMAP (ContactCard/set destroy).
async fn execute_contact_delete(
    client: &Option<JmapClient>,
    contact_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use jmap_contacts_client::JmapContactsExt;

    let client = client.as_ref().ok_or("no JMAP client")?;
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

/// Create a new calendar event via JMAP (CalendarEvent/set).
async fn execute_event_create(
    client: &Option<JmapClient>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = client.as_ref().ok_or("no JMAP client")?;
    let session = client.fetch_session().await?;

    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:calendars")
        .ok_or("no primary calendars account in session")?;

    // Use raw request to create a simple event
    let request_args = json!({
        "accountId": account_id,
        "create": {
            "new1": {
                "@type": "Event",
                "title": "(New Event)",
                "start": "2026-01-01T09:00:00",
                "duration": "PT1H"
            }
        }
    });

    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:calendars".to_string(),
        ],
        vec![(
            "CalendarEvent/set".to_string(),
            request_args,
            "create1".to_string(),
        )],
        None,
    );

    let resp = client.call(session.api_url.as_str(), &request).await?;

    for (method_name, result, _call_id) in &resp.method_responses {
        if method_name == "error" {
            let err = result["description"].as_str().unwrap_or("unknown");
            return Err(format!("JMAP error: {err}").into());
        }
        if method_name == "CalendarEvent/set" {
            if let Some(not_created) = result["notCreated"].as_object() {
                if let Some((_key, err)) = not_created.iter().next() {
                    return Err(format!("Event creation failed: {err}").into());
                }
            }
        }
    }

    Ok(())
}

/// Delete a calendar event via JMAP (CalendarEvent/set destroy).
async fn execute_event_delete(
    client: &Option<JmapClient>,
    event_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = client.as_ref().ok_or("no JMAP client")?;
    let session = client.fetch_session().await?;

    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:calendars")
        .ok_or("no primary calendars account in session")?;

    let request_args = json!({
        "accountId": account_id,
        "destroy": [event_id]
    });

    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:calendars".to_string(),
        ],
        vec![(
            "CalendarEvent/set".to_string(),
            request_args,
            "del1".to_string(),
        )],
        None,
    );

    let resp = client.call(session.api_url.as_str(), &request).await?;

    for (method_name, result, _call_id) in &resp.method_responses {
        if method_name == "error" {
            let err = result["description"].as_str().unwrap_or("unknown");
            return Err(format!("JMAP error: {err}").into());
        }
        if method_name == "CalendarEvent/set" {
            if let Some(not_destroyed) = result["notDestroyed"].as_object() {
                if let Some((_key, err)) = not_destroyed.iter().next() {
                    return Err(format!("Event deletion failed: {err}").into());
                }
            }
        }
    }

    Ok(())
}
