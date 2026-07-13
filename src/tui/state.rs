//! Application state: modes, panels, pending server actions, and the `App` struct.

use jmap_base_client::JmapClient;
use ratatui::widgets::ListState;

use crate::config::Config;

use super::entries::{
    format_contact, format_event, format_folder, format_mail, CalendarEventEntry, ContactEntry,
    FolderEntry, MailEntry, ProfileEntry,
};
use super::event::InputMode;
use super::popout::PopoutManager;
use super::search::{MatchedItem, Matcher};
use super::ui::preview::PreviewContent;

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

/// A server-side operation queued by an action, executed by the event loop.
#[derive(Debug, Clone)]
pub enum PendingAction {
    /// Move an email to another mailbox (archive/delete/spam).
    Move {
        email_id: String,
        source_mailbox_id: String,
        target_mailbox_id: String,
        action_name: String,
    },
    /// Send an email composed in a popout.
    SendMail {
        to: String,
        cc: String,
        bcc: String,
        subject: String,
        body: String,
    },
    CreateContact {
        name: String,
        email: String,
        phone: String,
    },
    DeleteContact(String),
    CreateEvent {
        title: String,
        start: String,
        duration: String,
    },
    DeleteEvent(String),
}

/// The main application state.
pub struct App {
    /// Current search input string.
    pub input: String,
    /// Current input mode.
    pub input_mode: InputMode,
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

    pub(crate) should_quit: bool,
    pub(crate) matcher: Matcher<String>,

    // Data stores
    pub(crate) profiles: Vec<ProfileEntry>,
    pub(crate) folders: Vec<FolderEntry>,
    pub(crate) mails: Vec<MailEntry>,
    pub(crate) contacts: Vec<ContactEntry>,
    pub(crate) events: Vec<CalendarEventEntry>,

    // Context
    pub(crate) active_profile_name: String,
    pub(crate) active_folder_id: Option<String>,
    pub(crate) active_folder_name: String,

    // JMAP client (created on profile selection)
    pub(crate) client: Option<JmapClient>,
    pub(crate) config: Config,

    // Async state
    pub(crate) loading: bool,
    pub(crate) status_message: Option<String>,

    /// Queued server-side operations, drained by the event loop.
    pub(crate) pending: Vec<PendingAction>,
    /// Set when the current panel's data must be re-fetched.
    pub needs_reload: bool,
    /// Set when search triggered an all-folder fetch and needs async load.
    pub needs_search_reload: bool,
    /// Whether the mail list currently shows results from all folders.
    pub(crate) search_all_folders: bool,
    /// Popout overlays.
    pub popouts: PopoutManager,
    /// Quit confirmation dialog.
    pub show_quit_confirm: bool,
    /// Mode to restore when cancelling a destructive-action confirmation.
    pub(crate) pre_confirm_mode: Option<InputMode>,
}

impl App {
    /// Create a new app with config; the active profile defaults to
    /// `--profile`, then `default_profile`, then the first configured profile.
    pub fn new(config: Config, profile_name: Option<&str>) -> Self {
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

        Self {
            input: String::new(),
            input_mode: InputMode::Normal,
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
            matcher: Matcher::new(),
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
            status_message: None,
            pending: Vec::new(),
            needs_reload: false,
            needs_search_reload: false,
            search_all_folders: false,
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

    /// Title for the results list.
    pub fn results_title(&self) -> &str {
        match self.panel {
            Panel::Profiles => "Profiles",
            Panel::Folders => "Folders",
            Panel::Mails => {
                if self.search_all_folders {
                    "All Mail"
                } else {
                    &self.active_folder_name
                }
            }
            Panel::Contacts => "Contacts",
            Panel::Calendar => "Calendar",
        }
    }

    /// Sync the input mode with the popout state: overlays get focus while
    /// any popout is active; when all are minimized the app has focus again.
    pub(crate) fn sync_overlay_mode(&mut self) {
        if self.popouts.has_active() {
            if self.input_mode == InputMode::Normal {
                self.input_mode = InputMode::Overlay;
            }
        } else if matches!(self.input_mode, InputMode::Overlay | InputMode::Editing) {
            self.input_mode = InputMode::Normal;
        }
    }

    /// Reset list state when switching panels.
    pub(crate) fn reset_for_panel(&mut self) {
        self.input.clear();
        self.list_state.select(Some(0));
        self.results.clear();
        self.preview_content = None;
        self.search_all_folders = false;
        // Recreate matcher (flush all items)
        self.matcher = Matcher::new();
    }

    pub(crate) fn update_search(&mut self) {
        // On the Mails panel, switch to all-folder search when input is non-empty
        if self.panel == Panel::Mails {
            let want_all = !self.input.is_empty();
            if want_all && !self.search_all_folders {
                // Switch INTO all-folder mode
                self.search_all_folders = true;
                self.needs_search_reload = true;
            } else if !want_all && self.search_all_folders {
                // Switch BACK to single-folder mode
                self.search_all_folders = false;
                self.needs_search_reload = true;
            }
        }
        self.matcher.find(&self.input);
    }

    /// Signature from the active profile config.
    pub(crate) fn signature(&self) -> Option<String> {
        self.config
            .profiles
            .get(&self.active_profile_name)
            .and_then(|p| p.signature.clone())
    }

    pub(crate) fn select_prev(&mut self) {
        let selected = self.list_state.selected().unwrap_or(0);
        if selected > 0 {
            self.list_state.select(Some(selected - 1));
            self.update_preview();
        }
    }

    pub(crate) fn select_next(&mut self) {
        let selected = self.list_state.selected().unwrap_or(0);
        let max = self.results.len().saturating_sub(1);
        if selected < max {
            self.list_state.select(Some(selected + 1));
            self.update_preview();
        }
    }

    /// The JMAP id of the currently selected list item (via `MatchedItem.inner`).
    pub(crate) fn selected_id(&self) -> Option<&str> {
        let selected = self.list_state.selected().unwrap_or(0);
        self.results.get(selected).map(|item| item.inner.as_str())
    }

    /// The currently selected mail entry, resolved by JMAP id.
    pub(crate) fn selected_mail(&self) -> Option<&MailEntry> {
        let id = self.selected_id()?;
        self.mails.iter().find(|m| m.id == id)
    }

    /// Tick the matcher and refresh results.
    pub(crate) fn tick(&mut self) {
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
        // Ensure something is always selected when results exist
        if !self.results.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }

        self.update_preview();
    }

    /// Update the preview based on current panel and selection.
    pub(crate) fn update_preview(&mut self) {
        use ratatui::text::Line;

        let Some(id) = self.selected_id().map(str::to_string) else {
            self.preview_content = None;
            return;
        };

        self.preview_content = match self.panel {
            Panel::Profiles => {
                self.profiles
                    .iter()
                    .find(|p| p.name == id)
                    .map(|profile| PreviewContent {
                        title: profile.name.clone(),
                        body: vec![Line::from(format!("Server: {}", profile.server_url))],
                    })
            }
            Panel::Folders => {
                self.folders
                    .iter()
                    .find(|f| f.id == id)
                    .map(|folder| PreviewContent {
                        title: folder.name.clone(),
                        body: vec![
                            Line::from(format!("ID: {}", folder.id)),
                            Line::from(format!("Role: {}", folder.role.as_deref().unwrap_or("-"))),
                            Line::from(format!("Total: {}", folder.total_emails)),
                            Line::from(format!("Unread: {}", folder.unread_emails)),
                        ],
                    })
            }
            Panel::Mails => self.mails.iter().find(|m| m.id == id).map(|mail| {
                let mut body = vec![
                    Line::from(format!("From: {}", mail.from)),
                    Line::from(format!("Date: {}", mail.date)),
                    Line::from(format!("Subject: {}", mail.subject)),
                    Line::from(""),
                    Line::from("─".repeat(40)),
                    Line::from(""),
                ];
                for line in mail.preview.lines() {
                    body.push(Line::from(line.to_string()));
                }
                PreviewContent {
                    title: mail.subject.clone(),
                    body,
                }
            }),
            Panel::Contacts => {
                self.contacts
                    .iter()
                    .find(|c| c.id == id)
                    .map(|contact| PreviewContent {
                        title: contact.name.clone(),
                        body: vec![
                            Line::from(format!("Name: {}", contact.name)),
                            Line::from(format!("Email: {}", contact.email)),
                            Line::from(format!("Phone: {}", contact.phone)),
                        ],
                    })
            }
            Panel::Calendar => {
                self.events
                    .iter()
                    .find(|e| e.id == id)
                    .map(|event| PreviewContent {
                        title: event.title.clone(),
                        body: vec![
                            Line::from(format!("Title: {}", event.title)),
                            Line::from(format!("Start: {}", event.start)),
                            Line::from(format!("Duration: {}", event.duration)),
                            Line::from(format!("Status: {}", event.status)),
                        ],
                    })
            }
        };
    }

    /// Inject items into the matcher based on the current panel.
    ///
    /// The stable JMAP identifier is stored as `inner` so that resolution
    /// uses the id rather than the display string.
    pub(crate) fn inject_items(&self) {
        let injector = self.matcher.injector();
        match self.panel {
            Panel::Profiles => {
                for profile in &self.profiles {
                    // Profile name IS the unique identifier
                    injector.push(profile.name.clone(), |item, cols| {
                        cols[0] = item.as_str().into();
                    });
                }
            }
            Panel::Folders => {
                for folder in &self.folders {
                    let display = format_folder(folder);
                    injector.push(folder.id.clone(), |_item, cols| {
                        cols[0] = display.as_str().into();
                    });
                }
            }
            Panel::Mails => {
                for mail in &self.mails {
                    let display = format_mail(mail);
                    injector.push(mail.id.clone(), |_item, cols| {
                        cols[0] = display.as_str().into();
                    });
                }
            }
            Panel::Contacts => {
                for contact in &self.contacts {
                    let display = format_contact(contact);
                    injector.push(contact.id.clone(), |_item, cols| {
                        cols[0] = display.as_str().into();
                    });
                }
            }
            Panel::Calendar => {
                for event in &self.events {
                    let display = format_event(event);
                    injector.push(event.id.clone(), |_item, cols| {
                        cols[0] = display.as_str().into();
                    });
                }
            }
        }
    }

    /// Re-inject items after data changed and kick the matcher.
    pub(crate) fn refresh_matcher(&mut self) {
        self.matcher = Matcher::new();
        self.inject_items();
        self.matcher.find(&self.input);
        self.matcher.tick();
    }
}
