//! The application: owns the screens, popups, and dialogs; routes every
//! message; composes frames. All JMAP work happens in the worker — the app
//! only sends commands and consumes result events.

use std::collections::HashMap;

use crossterm::event::KeyEventKind;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{Config, FolderMappings, Profile};
use crate::jmap::mail::FullEmail;

use super::keymap::{map_key, KeyMode, Screen};
use super::messages::{Command, Event, Message};
use super::model::form::Form;
use super::overlay::{Popup, PopupKind, Popups};
use super::screens::calendar::CalendarScreen;
use super::screens::contacts::ContactsScreen;
use super::screens::mail::{MailFocus, MailScreen};
use super::statusbar::{self, Tooltip};

/// A destructive action awaiting y/n confirmation.
#[derive(Debug, Clone, PartialEq)]
pub enum PendingConfirm {
    DeleteMail(String),
    ArchiveMail(String),
    SpamMail(String),
    DeleteContact(String),
    DeleteEvent(String),
}

impl PendingConfirm {
    fn prompt(&self) -> &'static str {
        match self {
            PendingConfirm::DeleteMail(_) => "Delete this email? [y/n]",
            PendingConfirm::ArchiveMail(_) => "Archive this email? [y/n]",
            PendingConfirm::SpamMail(_) => "Mark as spam? [y/n]",
            PendingConfirm::DeleteContact(_) => "Delete this contact? [y/n]",
            PendingConfirm::DeleteEvent(_) => "Delete this event? [y/n]",
        }
    }
}

pub struct App {
    sender: UnboundedSender<Message>,

    pub screen: Screen,
    pub mail: MailScreen,
    pub contacts: ContactsScreen,
    pub calendar: CalendarScreen,
    pub popups: Popups,

    pub search_active: bool,
    pub editing: bool,
    pub confirm: Option<PendingConfirm>,
    pub quit_dialog: bool,
    pub running: bool,

    pub tooltip: Option<Tooltip>,
    pub busy: usize,
    pub spinner_step: usize,

    /// Cache of fully fetched email bodies (for reply/forward quoting).
    bodies: HashMap<String, FullEmail>,

    profile: Profile,
    contacts_loaded: bool,
    events_loaded: bool,
}

impl App {
    pub fn new(config: &Config, profile_name: &str, sender: UnboundedSender<Message>) -> Self {
        let profile = config
            .profiles
            .get(profile_name)
            .cloned()
            .unwrap_or_else(|| Profile {
                server_url: String::new(),
                auth: crate::config::AuthMethod::OAuthBrowser {
                    client_id: "herald".into(),
                },
                from_email: None,
                from_name: None,
                folders: FolderMappings::default(),
                compose_format: None,
                signature: None,
                allow_insecure: false,
                confirm_actions: true,
            });

        Self {
            sender,
            screen: Screen::Mail,
            mail: MailScreen::new(),
            contacts: ContactsScreen::new(),
            calendar: CalendarScreen::new(),
            popups: Popups::default(),
            search_active: false,
            editing: false,
            confirm: None,
            quit_dialog: false,
            running: true,
            tooltip: None,
            busy: 0,
            spinner_step: 0,
            bodies: HashMap::new(),
            profile,
            contacts_loaded: false,
            events_loaded: false,
        }
    }

    fn send(&self, cmd: Command) {
        let _ = self.sender.send(Message::Command(cmd));
    }

    fn mappings(&self) -> &FolderMappings {
        &self.profile.folders
    }

    /// Which context receives keys right now.
    pub fn key_mode(&self) -> KeyMode {
        if self.quit_dialog {
            KeyMode::QuitDialog
        } else if self.confirm.is_some() {
            KeyMode::Confirm
        } else if self.editing {
            KeyMode::Editing
        } else if self.popups.has_active() {
            KeyMode::Popup
        } else if self.search_active {
            KeyMode::Search
        } else {
            KeyMode::Normal(self.screen)
        }
    }

    // ------------------------------------------------------------------
    // Message processing
    // ------------------------------------------------------------------

    pub fn process(&mut self, message: &Message) {
        match message {
            Message::Event(event) => self.process_event(event),
            Message::Command(cmd) => {
                if cmd.is_data() {
                    self.busy += 1;
                } else {
                    self.handle_command(cmd.clone());
                }
            }
        }
    }

    fn process_event(&mut self, event: &Event) {
        match event {
            Event::Key(key) => {
                if key.kind == KeyEventKind::Press {
                    let cmd = map_key(*key, self.key_mode());
                    self.handle_command(cmd);
                }
            }
            Event::Tick => {
                self.spinner_step = self.spinner_step.wrapping_add(1);
                self.mail.tick();
            }
            Event::Resized => {}

            Event::ClientReady(_) => {
                self.tooltip = Some(Tooltip::info("Connected"));
                self.send(Command::LoadFolders);
            }
            Event::ConnectFailed(e) => {
                self.busy = self.busy.saturating_sub(1);
                self.tooltip = Some(Tooltip::error(format!("Auth error: {e}")));
            }

            Event::FoldersLoaded(folders) => {
                self.busy = self.busy.saturating_sub(1);
                self.mail.on_folders_loaded(folders.clone());
                if self.mail.active_folder_id.is_none() {
                    self.open_inbox();
                }
            }
            Event::MailPageLoaded {
                mails,
                position,
                all_folders,
            } => {
                self.busy = self.busy.saturating_sub(1);
                self.mail.on_mails_loaded(mails.clone(), *position, *all_folders);
            }
            Event::MailBodyLoaded(full) => {
                self.busy = self.busy.saturating_sub(1);
                self.bodies.insert(full.id.clone(), (**full).clone());
                let kind = PopupKind::MailView {
                    email_id: full.id.clone(),
                };
                if let Some(idx) = self.popups.find(&kind) {
                    self.popups.items[idx].body = mail_view_body(full);
                }
            }
            Event::ContactsLoaded(contacts) => {
                self.busy = self.busy.saturating_sub(1);
                self.contacts_loaded = true;
                self.contacts.on_loaded(contacts.clone());
            }
            Event::EventsLoaded(events) => {
                self.busy = self.busy.saturating_sub(1);
                self.events_loaded = true;
                self.calendar.on_loaded(events.clone());
            }

            Event::ActionCompleted(msg) => {
                self.busy = self.busy.saturating_sub(1);
                self.tooltip = Some(Tooltip::info(msg.clone()));
                self.reload_current();
            }
            Event::ActionFailed(msg) => {
                self.busy = self.busy.saturating_sub(1);
                self.mail.loading_more = false;
                self.tooltip = Some(Tooltip::error(msg.clone()));
            }
        }
    }

    /// Refresh the data behind the current screen (after a mutation).
    fn reload_current(&mut self) {
        self.send(Command::LoadFolders);
        match self.screen {
            Screen::Mail => self.reload_mail_page(),
            Screen::Contacts => self.send(Command::LoadContacts),
            Screen::Calendar => self.send(Command::LoadEvents),
        }
    }

    fn reload_mail_page(&mut self) {
        let folder_id = if self.mail.all_folders {
            None
        } else {
            self.mail.active_folder_id.clone()
        };
        if folder_id.is_some() || self.mail.all_folders {
            self.send(Command::LoadMailPage {
                folder_id,
                position: 0,
            });
        }
    }

    // ------------------------------------------------------------------
    // Command handling
    // ------------------------------------------------------------------

    pub fn handle_command(&mut self, cmd: Command) {
        use Command::*;
        match cmd {
            NoOp => {}
            Quit => self.quit_dialog = true,
            Help => self.open_help(),

            ConfirmYes => {
                if self.quit_dialog {
                    self.running = false;
                } else if let Some(action) = self.confirm.take() {
                    self.tooltip = None;
                    self.execute_confirmed(action);
                }
            }
            ConfirmNo => {
                if self.quit_dialog {
                    self.quit_dialog = false;
                } else if self.confirm.take().is_some() {
                    self.tooltip = Some(Tooltip::info("Cancelled"));
                }
            }

            ScreenNext => self.switch_screen(self.screen.next()),
            ScreenPrev => self.switch_screen(self.screen.prev()),

            SelectNext => match self.screen {
                Screen::Mail => {
                    if self.mail.select_next() {
                        self.mail.loading_more = true;
                        let folder_id = if self.mail.all_folders {
                            None
                        } else {
                            self.mail.active_folder_id.clone()
                        };
                        let position = self.mail.mails.len();
                        self.send(Command::LoadMailPage {
                            folder_id,
                            position,
                        });
                    }
                }
                Screen::Contacts => self.contacts.select_next(),
                Screen::Calendar => self.calendar.select_next(),
            },
            SelectPrev => match self.screen {
                Screen::Mail => self.mail.select_prev(),
                Screen::Contacts => self.contacts.select_prev(),
                Screen::Calendar => self.calendar.select_prev(),
            },

            FocusLeft => {
                if self.screen == Screen::Mail {
                    self.mail.focus = match self.mail.focus {
                        MailFocus::List => MailFocus::Folders,
                        MailFocus::Folders => MailFocus::Account,
                        MailFocus::Account => MailFocus::Folders,
                    };
                }
            }
            FocusRight => {
                if self.screen == Screen::Mail {
                    self.mail.focus = match self.mail.focus {
                        MailFocus::List => MailFocus::Folders,
                        MailFocus::Folders => MailFocus::List,
                        MailFocus::Account => MailFocus::Folders,
                    };
                }
            }

            OpenItem => self.open_item(),
            Escape => self.escape(),

            EnterSearch => {
                if self.screen == Screen::Calendar {
                    self.tooltip = Some(Tooltip::warn("no search on the calendar"));
                } else {
                    if self.screen == Screen::Mail {
                        self.mail.focus = MailFocus::List;
                    }
                    self.search_active = true;
                }
            }
            InsertChar(c) => {
                let mut q = self.current_query();
                q.push(c);
                self.apply_query(&q);
            }
            Backspace => {
                let mut q = self.current_query();
                q.pop();
                self.apply_query(&q);
            }
            ClearInput => self.apply_query(""),
            SubmitInput => {
                // commit: back to Normal with the first result selected
                self.search_active = false;
                if self.screen == Screen::Mail {
                    self.mail.win.reset();
                    self.mail.tick();
                } else {
                    self.contacts.win.reset();
                }
            }
            CancelSearch => {
                self.search_active = false;
                self.apply_query("");
            }

            CreateItem => self.create_item(),
            EditItem => self.edit_item(),
            DeleteItem => self.delete_item(),
            Reply => self.reply(false),
            Forward => self.reply(true),
            Archive => self.mail_action("archive"),
            Spam => self.mail_action("spam"),
            Submit => self.submit_popup(),

            TogglePopup(idx) => {
                self.popups.toggle(idx);
                self.editing = false;
            }
            MinimizePopup => {
                self.popups.minimize_focused();
                self.editing = false;
            }
            ClosePopup => {
                let was_editor = self
                    .popups
                    .focused_popup()
                    .is_some_and(|p| p.kind.is_editor());
                self.popups.close_focused();
                self.editing = false;
                if was_editor {
                    self.tooltip = Some(Tooltip::info("Draft discarded"));
                }
            }
            ToggleMaximize => self.popups.toggle_maximize(),
            FocusNextPopup => {
                self.popups.focus_next();
                self.editing = false;
            }
            EditPopup => {
                if self
                    .popups
                    .focused_popup()
                    .is_some_and(|p| p.kind.is_editor())
                {
                    self.editing = true;
                }
            }

            EditorChar(c) => self.with_form(|f| f.insert_char(c)),
            EditorBackspace => self.with_form(|f| f.backspace()),
            EditorEnter => self.with_form(|f| f.enter()),
            EditorNextField => self.with_form(|f| f.next_focus()),
            EditorEscape => self.editing = false,

            DayPrev => self.calendar.move_days(-1),
            DayNext => self.calendar.move_days(1),
            MonthPrev => self.calendar.move_months(-1),
            MonthNext => self.calendar.move_months(1),
            Today => self.calendar.today(),

            // data commands are consumed by the worker
            _ => {}
        }
    }

    fn with_form(&mut self, f: impl FnOnce(&mut Form)) {
        if let Some(p) = self.popups.focused_popup_mut() {
            if let Some(form) = &mut p.form {
                f(form);
            }
        }
    }

    fn switch_screen(&mut self, screen: Screen) {
        self.screen = screen;
        self.search_active = false;
        match screen {
            Screen::Contacts if !self.contacts_loaded => self.send(Command::LoadContacts),
            Screen::Calendar if !self.events_loaded => self.send(Command::LoadEvents),
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------

    fn current_query(&self) -> String {
        match self.screen {
            Screen::Mail => self.mail.query.clone(),
            Screen::Contacts => self.contacts.query.clone(),
            Screen::Calendar => String::new(),
        }
    }

    fn apply_query(&mut self, query: &str) {
        match self.screen {
            Screen::Mail => {
                if self.mail.set_query(query) {
                    // scope flipped: all-folder search ↔ single folder
                    let folder_id = if query.is_empty() {
                        self.mail.active_folder_id.clone()
                    } else {
                        None
                    };
                    self.mail.all_folders = !query.is_empty();
                    self.send(Command::LoadMailPage {
                        folder_id,
                        position: 0,
                    });
                }
                // Tick matcher immediately to process the query on the current data
                self.mail.tick();
            }
            Screen::Contacts => {
                self.contacts.set_query(query);
                // Tick matcher immediately to process the query on the current data
                // (contacts don't use the matcher yet, but this keeps consistency)
            }
            Screen::Calendar => {}
        }
    }

    // ------------------------------------------------------------------
    // Esc ladder
    // ------------------------------------------------------------------

    /// Esc in Normal mode: search → previous view → folders → inbox → quit.
    fn escape(&mut self) {
        if !self.current_query().is_empty() {
            self.apply_query("");
            return;
        }
        match self.screen {
            Screen::Contacts | Screen::Calendar => {
                self.switch_screen(Screen::Mail);
            }
            Screen::Mail => match self.mail.focus {
                MailFocus::List => {
                    if self.mail.is_inbox_active() {
                        self.quit_dialog = true;
                    } else {
                        self.mail.focus = MailFocus::Folders;
                        // park the folder cursor on the active folder
                        if let Some(id) = &self.mail.active_folder_id {
                            if let Some(pos) =
                                self.mail.folders.iter().position(|f| &f.id == id)
                            {
                                let total = self.mail.folders.len();
                                self.mail.folder_win.select(pos, total);
                            }
                        }
                    }
                }
                MailFocus::Folders => {
                    self.mail.focus = MailFocus::Account;
                }
                MailFocus::Account => self.open_inbox(),
            },
        }
    }

    /// Activate the inbox and load its mail.
    fn open_inbox(&mut self) {
        let inbox = self.mail.inbox().map(|f| (f.id.clone(), f.name.clone()));
        self.mail.focus = MailFocus::List;
        if let Some((id, name)) = inbox {
            let changed = self.mail.active_folder_id.as_deref() != Some(id.as_str());
            self.mail.active_folder_id = Some(id.clone());
            self.mail.active_folder_name = name;
            if changed || self.mail.mails.is_empty() {
                self.send(Command::LoadMailPage {
                    folder_id: Some(id),
                    position: 0,
                });
            }
        }
    }

    // ------------------------------------------------------------------
    // Open / create / edit / delete
    // ------------------------------------------------------------------

    fn open_item(&mut self) {
        match self.screen {
            Screen::Mail => match self.mail.focus {
                MailFocus::Folders => {
                    if let Some(f) = self.mail.selected_folder() {
                        let (id, name) = (f.id.clone(), f.name.clone());
                        self.mail.active_folder_id = Some(id.clone());
                        self.mail.active_folder_name = name;
                        self.mail.focus = MailFocus::List;
                        self.mail.query.clear();
                        self.mail.win.reset();
                        self.send(Command::LoadMailPage {
                            folder_id: Some(id),
                            position: 0,
                        });
                    }
                }
                MailFocus::List => self.open_mail_popup(),
                MailFocus::Account => {}, // Account view has no items to open
            },
            Screen::Contacts => {
                if let Some(c) = self.contacts.selected() {
                    let body = vec![
                        Line::from(format!("Name:  {}", c.name)),
                        Line::from(format!("Email: {}", c.email)),
                        Line::from(format!("Phone: {}", c.phone)),
                    ];
                    self.popups
                        .open(Popup::view(PopupKind::ContactView, c.name.clone(), body));
                }
            }
            Screen::Calendar => {
                if let Some(e) = self.calendar.selected_event() {
                    let kind = PopupKind::EventView {
                        event_id: e.id.clone(),
                    };
                    if let Some(idx) = self.popups.find(&kind) {
                        self.popups.toggle(idx);
                        return;
                    }
                    let body = vec![
                        Line::from(format!("Title:    {}", e.title)),
                        Line::from(format!("Start:    {}", e.start)),
                        Line::from(format!("Duration: {}", e.duration)),
                        Line::from(format!("Status:   {}", e.status)),
                    ];
                    self.popups.open(Popup::view(kind, e.title.clone(), body));
                }
            }
        }
    }

    fn open_mail_popup(&mut self) {
        let Some(mail) = self.mail.selected_mail() else {
            return;
        };
        let (id, subject) = (mail.id.clone(), mail.subject.clone());
        let kind = PopupKind::MailView {
            email_id: id.clone(),
        };
        if let Some(idx) = self.popups.find(&kind) {
            self.popups.toggle(idx);
            return;
        }
        let body = if let Some(full) = self.bodies.get(&id) {
            mail_view_body(full)
        } else {
            self.send(Command::LoadMailBody(id.clone()));
            vec![
                Line::from(format!("From: {}", mail.from)),
                Line::from(format!("Date: {}", mail.date)),
                Line::from(""),
                Line::from(Span::styled(
                    "loading full message…",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        };
        self.popups.open(Popup::view(kind, subject, body));
    }

    fn create_item(&mut self) {
        match self.screen {
            Screen::Mail => {
                let mut form = Form::new(&["To", "Cc", "Bcc", "Subject"], true);
                if let Some(sig) = &self.profile.signature {
                    form.set_body(&format!("\n\n{sig}"));
                }
                self.open_editor(Popup::editor(PopupKind::Compose, "New Email".into(), form));
            }
            Screen::Contacts => {
                let form = Form::new(&["Name", "Email", "Phone"], false);
                self.open_editor(Popup::editor(
                    PopupKind::ContactForm { contact_id: None },
                    "New Contact".into(),
                    form,
                ));
            }
            Screen::Calendar => {
                let mut form = Form::new(&["Title", "Start", "Duration"], false);
                form.set_field("Start", &format!("{}T09:00:00", self.calendar.selected.iso()));
                form.set_field("Duration", "PT1H");
                self.open_editor(Popup::editor(
                    PopupKind::EventForm { event_id: None },
                    "New Event".into(),
                    form,
                ));
            }
        }
    }

    fn edit_item(&mut self) {
        match self.screen {
            Screen::Contacts => {
                if let Some(c) = self.contacts.selected() {
                    let (id, name, email, phone) =
                        (c.id.clone(), c.name.clone(), c.email.clone(), c.phone.clone());
                    let mut form = Form::new(&["Name", "Email", "Phone"], false);
                    form.set_field("Name", &name);
                    form.set_field("Email", &email);
                    form.set_field("Phone", &phone);
                    self.open_editor(Popup::editor(
                        PopupKind::ContactForm {
                            contact_id: Some(id),
                        },
                        format!("Edit: {name}"),
                        form,
                    ));
                } else {
                    self.tooltip = Some(Tooltip::warn("no contact selected"));
                }
            }
            Screen::Calendar => {
                if let Some(e) = self.calendar.selected_event() {
                    let (id, title, start, duration) =
                        (e.id.clone(), e.title.clone(), e.start.clone(), e.duration.clone());
                    let mut form = Form::new(&["Title", "Start", "Duration"], false);
                    form.set_field("Title", &title);
                    form.set_field("Start", &start);
                    form.set_field("Duration", &duration);
                    self.open_editor(Popup::editor(
                        PopupKind::EventForm { event_id: Some(id) },
                        format!("Edit: {title}"),
                        form,
                    ));
                } else {
                    self.tooltip = Some(Tooltip::warn("no event selected"));
                }
            }
            Screen::Mail => {
                // 'e' has no meaning on the mail list; in Popup mode it edits
            }
        }
    }

    fn open_editor(&mut self, popup: Popup) {
        self.popups.open(popup);
        self.editing = true;
    }

    /// `d`: delete the focused popup's entry or the selected list item.
    fn delete_item(&mut self) {
        // A focused editor popup: discard it instead
        if self
            .popups
            .focused_popup()
            .is_some_and(|p| p.kind.is_editor())
        {
            self.handle_command(Command::ClosePopup);
            return;
        }
        // A focused view popup targets its own entry
        if let Some(p) = self.popups.focused_popup() {
            match &p.kind {
                PopupKind::MailView { email_id } => {
                    let id = email_id.clone();
                    self.request_confirm(PendingConfirm::DeleteMail(id));
                    return;
                }
                PopupKind::EventView { event_id } => {
                    let id = event_id.clone();
                    self.request_confirm(PendingConfirm::DeleteEvent(id));
                    return;
                }
                _ => {}
            }
        }
        match self.screen {
            Screen::Mail => {
                if let Some(m) = self.mail.selected_mail() {
                    let id = m.id.clone();
                    self.request_confirm(PendingConfirm::DeleteMail(id));
                } else {
                    self.tooltip = Some(Tooltip::warn("no email selected"));
                }
            }
            Screen::Contacts => {
                if let Some(c) = self.contacts.selected() {
                    let id = c.id.clone();
                    self.request_confirm(PendingConfirm::DeleteContact(id));
                } else {
                    self.tooltip = Some(Tooltip::warn("no contact selected"));
                }
            }
            Screen::Calendar => {
                if let Some(e) = self.calendar.selected_event() {
                    let id = e.id.clone();
                    self.request_confirm(PendingConfirm::DeleteEvent(id));
                } else {
                    self.tooltip = Some(Tooltip::warn("no event selected"));
                }
            }
        }
    }

    fn mail_action(&mut self, action: &str) {
        let Some(id) = self.context_mail_id() else {
            self.tooltip = Some(Tooltip::warn("no email selected"));
            return;
        };
        let confirm = match action {
            "archive" => PendingConfirm::ArchiveMail(id),
            "spam" => PendingConfirm::SpamMail(id),
            _ => return,
        };
        self.request_confirm(confirm);
    }

    /// The email targeted by a context action: the focused mail popup or the
    /// selected list row.
    fn context_mail_id(&self) -> Option<String> {
        if let Some(p) = self.popups.focused_popup() {
            if let PopupKind::MailView { email_id } = &p.kind {
                return Some(email_id.clone());
            }
        }
        self.mail.selected_mail().map(|m| m.id.clone())
    }

    fn request_confirm(&mut self, action: PendingConfirm) {
        if self.profile.confirm_actions {
            self.tooltip = Some(Tooltip::warn(action.prompt()));
            self.confirm = Some(action);
        } else {
            self.execute_confirmed(action);
        }
    }

    fn execute_confirmed(&mut self, action: PendingConfirm) {
        match action {
            PendingConfirm::DeleteMail(id) => self.move_mail(id, "trash", "delete"),
            PendingConfirm::ArchiveMail(id) => self.move_mail(id, "archive", "archive"),
            PendingConfirm::SpamMail(id) => self.move_mail(id, "spam", "spam"),
            PendingConfirm::DeleteContact(id) => self.send(Command::DeleteContact(id)),
            PendingConfirm::DeleteEvent(id) => self.send(Command::DeleteEvent(id)),
        }
    }

    fn move_mail(&mut self, email_id: String, action_key: &str, action_name: &str) {
        let target = super::model::folders::resolve_action_folder(
            &self.mail.folders,
            self.mappings(),
            action_key,
        );
        let Some(target) = target else {
            self.tooltip = Some(Tooltip::error(format!("no {action_key} folder found")));
            return;
        };
        // Source: the mail's own folder (all-folder view) or the active one
        let source = self
            .mail
            .mails
            .iter()
            .find(|m| m.id == email_id)
            .and_then(|m| m.folder_id.clone())
            .or_else(|| self.mail.active_folder_id.clone())
            .unwrap_or_default();
        self.send(Command::MoveMail {
            email_id,
            source_mailbox_id: source,
            target_mailbox_id: target,
            action: action_name.to_string(),
        });
    }

    // ------------------------------------------------------------------
    // Reply / forward / submit
    // ------------------------------------------------------------------

    fn reply(&mut self, forward: bool) {
        let Some(id) = self.context_mail_id() else {
            self.tooltip = Some(Tooltip::warn("no email selected"));
            return;
        };
        let Some(mail) = self.mail.mails.iter().find(|m| m.id == id) else {
            return;
        };
        let (from, subject) = (mail.from.clone(), mail.subject.clone());
        // Quote the full body when we have it, the preview otherwise
        let quoted_src = self
            .bodies
            .get(&id)
            .map(|f| f.body.clone())
            .unwrap_or_else(|| mail.preview.clone());
        let quoted: String = quoted_src.lines().map(|l| format!("> {l}\n")).collect();
        let sig = self
            .profile
            .signature
            .as_ref()
            .map(|s| format!("\n\n{s}"))
            .unwrap_or_default();

        let mut form = Form::new(&["To", "Cc", "Bcc", "Subject"], true);
        let (kind, title) = if forward {
            form.set_field(
                "Subject",
                &if subject.starts_with("Fwd:") {
                    subject.clone()
                } else {
                    format!("Fwd: {subject}")
                },
            );
            form.set_body(&format!("{sig}\n\n---------- Forwarded message ----------\n{quoted}"));
            (
                PopupKind::Forward {
                    email_id: id.clone(),
                },
                format!("Fwd: {subject}"),
            )
        } else {
            form.set_field("To", &extract_address(&from));
            form.set_field(
                "Subject",
                &if subject.starts_with("Re:") {
                    subject.clone()
                } else {
                    format!("Re: {subject}")
                },
            );
            form.set_body(&format!("{sig}\n\n{quoted}"));
            (
                PopupKind::Reply {
                    email_id: id.clone(),
                },
                format!("Re: {subject}"),
            )
        };
        self.open_editor(Popup::editor(kind, title, form));
    }

    /// `s` on a focused editor popup: send the mail / save the form.
    fn submit_popup(&mut self) {
        let Some(p) = self.popups.focused_popup() else {
            return;
        };
        let Some(form) = &p.form else { return };

        let cmd = match &p.kind {
            PopupKind::Compose | PopupKind::Reply { .. } | PopupKind::Forward { .. } => {
                if form.field("To").trim().is_empty() {
                    self.tooltip = Some(Tooltip::warn("no recipient — press 'i' and fill To"));
                    return;
                }
                let sent_mailbox_id = super::model::folders::resolve_action_folder(
                    &self.mail.folders,
                    self.mappings(),
                    "sent",
                );
                Command::SendMail {
                    to: form.field("To").to_string(),
                    cc: form.field("Cc").to_string(),
                    bcc: form.field("Bcc").to_string(),
                    subject: form.field("Subject").to_string(),
                    body: form.body.clone().unwrap_or_default(),
                    sent_mailbox_id,
                }
            }
            PopupKind::ContactForm { contact_id } => {
                if form.field("Name").trim().is_empty() {
                    self.tooltip = Some(Tooltip::warn("Name is required"));
                    return;
                }
                let (name, email, phone) = (
                    form.field("Name").to_string(),
                    form.field("Email").to_string(),
                    form.field("Phone").to_string(),
                );
                match contact_id {
                    Some(id) => Command::UpdateContact {
                        id: id.clone(),
                        name,
                        email,
                        phone,
                    },
                    None => Command::CreateContact { name, email, phone },
                }
            }
            PopupKind::EventForm { event_id } => {
                if form.field("Title").trim().is_empty() {
                    self.tooltip = Some(Tooltip::warn("Title is required"));
                    return;
                }
                let (title, start, duration) = (
                    form.field("Title").to_string(),
                    form.field("Start").to_string(),
                    form.field("Duration").to_string(),
                );
                match event_id {
                    Some(id) => Command::UpdateEvent {
                        id: id.clone(),
                        title,
                        start,
                        duration,
                    },
                    None => Command::CreateEvent {
                        title,
                        start,
                        duration,
                    },
                }
            }
            _ => return,
        };
        self.send(cmd);
        self.popups.close_focused();
        self.editing = false;
    }

    fn open_help(&mut self) {
        if let Some(idx) = self.popups.find(&PopupKind::Help) {
            self.popups.toggle(idx);
            return;
        }
        let mut body = Vec::new();
        for (key, desc) in [
            ("Tab / Shift-Tab", "next / previous screen"),
            ("j k / ↓ ↑", "move selection"),
            ("h l", "mail: panel focus · calendar: day ±1"),
            ("H L / t", "calendar: month ±1 / today"),
            ("Enter", "open entry as popup"),
            ("/", "search (Enter select, Esc cancel)"),
            ("c e d", "create / edit / delete"),
            ("r f a s", "mail: reply, forward, archive, spam"),
            ("s", "popup: send / save"),
            ("i", "popup: edit fields"),
            ("x m Tab", "popup: close, maximize, next"),
            ("1-9 0", "toggle popup N"),
            ("Esc", "back towards the inbox / minimize"),
            ("q", "quit (Enter confirms)"),
        ] {
            body.push(Line::from(vec![
                Span::styled(
                    format!("{key:<16}"),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::raw(desc.to_string()),
            ]));
        }
        self.popups
            .open(Popup::view(PopupKind::Help, "Help".into(), body));
    }

    // ------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let bar = u16::from(!self.popups.items.is_empty());
        let input = u16::from(self.search_active);
        let main_h = area.height.saturating_sub(1 + bar + input);

        let main = Rect::new(area.x, area.y, area.width, main_h);
        let bar_area = Rect::new(area.x, area.y + main_h, area.width, bar);
        let input_area = Rect::new(area.x, area.y + main_h + bar, area.width, input);
        let status = Rect::new(area.x, area.y + main_h + bar + input, area.width, 1);

        match self.screen {
            Screen::Mail => {
                let chrome_focused = !self.popups.has_active();
                self.mail.render(frame, main, chrome_focused);
            }
            Screen::Contacts => {
                let focused = !self.popups.has_active();
                self.contacts.render(frame, main, focused);
            }
            Screen::Calendar => {
                let focused = !self.popups.has_active();
                self.calendar.render(frame, main, focused);
            }
        }

        let cursor = self.popups.render(frame, main, self.editing);
        if let Some(pos) = cursor {
            frame.set_cursor_position(pos);
        }

        if bar == 1 {
            self.popups.render_bar(frame, bar_area);
        }
        if input == 1 {
            self.render_input_bar(frame, input_area);
        }

        statusbar::render(
            frame,
            status,
            self.screen,
            self.key_mode(),
            self.mail.focus,
            self.tooltip.as_ref(),
            self.busy > 0 || self.mail.loading_more,
            self.spinner_step,
            !self.popups.items.is_empty(),
        );

        if self.quit_dialog {
            draw_quit_dialog(frame, area);
        }
    }

    fn render_input_bar(&self, frame: &mut Frame, area: Rect) {
        let query = self.current_query();
        let count = match self.screen {
            Screen::Mail => format!("{}/{}", self.mail.matched_count, self.mail.mails.len()),
            Screen::Contacts => format!(
                "{}/{}",
                self.contacts.filtered().len(),
                self.contacts.contacts.len()
            ),
            Screen::Calendar => String::new(),
        };
        let line = Line::from(vec![
            Span::styled("❯ ", Style::default().fg(Color::Cyan)),
            Span::styled(query.clone(), Style::default().fg(Color::White)),
            Span::styled(
                format!("   {count}"),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        frame.set_cursor_position((area.x + 2 + query.chars().count() as u16, area.y));
    }
}

/// Body lines for a fully fetched email.
fn mail_view_body(full: &FullEmail) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("From:    {}", full.from)),
        Line::from(format!("To:      {}", full.to)),
        Line::from(format!("Date:    {}", full.date)),
        Line::from(format!("Subject: {}", full.subject)),
        Line::from(Span::styled(
            "─".repeat(50),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    for l in full.body.lines() {
        lines.push(Line::from(l.to_string()));
    }
    lines
}

/// Extract the address part from "Name <addr>" (or return the input).
fn extract_address(from: &str) -> String {
    match (from.find('<'), from.rfind('>')) {
        (Some(a), Some(b)) if a < b => from[a + 1..b].to_string(),
        _ => from.trim().to_string(),
    }
}

/// Centered quit dialog with an explicit background; only Enter confirms.
fn draw_quit_dialog(frame: &mut Frame, area: Rect) {
    let w = 40u16.min(area.width.saturating_sub(4));
    let h = 5u16.min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red).bg(Color::Black))
        .title(Span::styled(
            " Quit Herald? ",
            Style::default()
                .fg(Color::White)
                .bg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ));
    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Enter",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" quit    "),
            Span::styled("any key", Style::default().fg(Color::Gray)),
            Span::raw(" stay"),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(block)
            .style(Style::default().fg(Color::White).bg(Color::Black)),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::types::{ContactEntry, EventEntry, FolderEntry};
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    fn test_app(confirm_actions: bool) -> (App, UnboundedReceiver<Message>) {
        let (tx, rx) = unbounded_channel();
        let mut config = Config::default();
        config.profiles.insert(
            "t".into(),
            Profile {
                server_url: "https://x".into(),
                auth: crate::config::AuthMethod::OAuthBrowser {
                    client_id: "herald".into(),
                },
                from_email: Some("me@x".into()),
                from_name: None,
                folders: FolderMappings::default(),
                compose_format: None,
                signature: Some("-- sig".into()),
                allow_insecure: false,
                confirm_actions,
            },
        );
        (App::new(&config, "t", tx), rx)
    }

    fn folder(id: &str, name: &str, tag: Option<&str>) -> FolderEntry {
        FolderEntry {
            id: id.into(),
            name: name.into(),
            parent_id: None,
            role: None,
            sort_order: 0,
            total_emails: 0,
            unread_emails: 0,
            display_name: name.into(),
            depth: 0,
            action_tag: tag.map(String::from),
        }
    }

    fn drain(rx: &mut UnboundedReceiver<Message>) -> Vec<Command> {
        let mut cmds = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Message::Command(c) = msg {
                cmds.push(c);
            }
        }
        cmds
    }

    #[test]
    fn esc_ladder_walks_back_to_inbox_then_quit() {
        let (mut app, mut rx) = test_app(true);
        app.process(&Message::Event(Event::FoldersLoaded(vec![
            folder("in", "Inbox", Some("inbox")),
            folder("ar", "Archive", Some("archive")),
        ])));
        drain(&mut rx);

        // contacts screen → Esc → mail
        app.screen = Screen::Contacts;
        app.handle_command(Command::Escape);
        assert_eq!(app.screen, Screen::Mail);

        // non-inbox mail list → Esc → folder focus
        app.mail.active_folder_id = Some("ar".into());
        app.mail.focus = MailFocus::List;
        app.handle_command(Command::Escape);
        assert_eq!(app.mail.focus, MailFocus::Folders);

        // folder focus → Esc → account focus
        app.handle_command(Command::Escape);
        assert_eq!(app.mail.focus, MailFocus::Account);

        // account focus → Esc → inbox active + list focus
        app.handle_command(Command::Escape);
        assert_eq!(app.mail.active_folder_id.as_deref(), Some("in"));
        assert_eq!(app.mail.focus, MailFocus::List);
        assert!(!app.quit_dialog);

        // inbox → Esc → quit dialog
        app.handle_command(Command::Escape);
        assert!(app.quit_dialog);

        // only Enter (ConfirmYes) quits
        app.handle_command(Command::ConfirmNo);
        assert!(!app.quit_dialog);
        assert!(app.running);
        app.handle_command(Command::Quit);
        app.handle_command(Command::ConfirmYes);
        assert!(!app.running);
    }

    #[test]
    fn delete_contact_asks_then_sends() {
        let (mut app, mut rx) = test_app(true);
        app.screen = Screen::Contacts;
        app.contacts.on_loaded(vec![ContactEntry {
            id: "c1".into(),
            name: "Alice".into(),
            email: String::new(),
            phone: String::new(),
        }]);
        app.handle_command(Command::DeleteItem);
        assert_eq!(app.confirm, Some(PendingConfirm::DeleteContact("c1".into())));
        assert!(drain(&mut rx).is_empty(), "nothing sent before confirmation");
        app.handle_command(Command::ConfirmYes);
        assert_eq!(drain(&mut rx), vec![Command::DeleteContact("c1".into())]);
    }

    #[test]
    fn confirm_disabled_deletes_immediately() {
        let (mut app, mut rx) = test_app(false);
        app.screen = Screen::Contacts;
        app.contacts.on_loaded(vec![ContactEntry {
            id: "c1".into(),
            name: "Alice".into(),
            email: String::new(),
            phone: String::new(),
        }]);
        app.handle_command(Command::DeleteItem);
        assert_eq!(drain(&mut rx), vec![Command::DeleteContact("c1".into())]);
    }

    #[test]
    fn contact_edit_prefills_and_updates() {
        let (mut app, mut rx) = test_app(false);
        app.screen = Screen::Contacts;
        app.contacts.on_loaded(vec![ContactEntry {
            id: "c1".into(),
            name: "Alice".into(),
            email: "a@x".into(),
            phone: "1".into(),
        }]);
        app.handle_command(Command::EditItem);
        assert!(app.editing);
        let form = app.popups.focused_popup().unwrap().form.as_ref().unwrap();
        assert_eq!(form.field("Name"), "Alice");
        assert_eq!(form.field("Email"), "a@x");

        // change the name and save
        app.handle_command(Command::EditorBackspace); // "Alic"
        app.handle_command(Command::EditorChar('!'));
        app.handle_command(Command::EditorEscape);
        app.handle_command(Command::Submit);
        let cmds = drain(&mut rx);
        assert_eq!(
            cmds,
            vec![Command::UpdateContact {
                id: "c1".into(),
                name: "Alic!".into(),
                email: "a@x".into(),
                phone: "1".into(),
            }]
        );
        assert!(app.popups.items.is_empty(), "editor closes after save");
    }

    #[test]
    fn event_create_and_update_flow() {
        let (mut app, mut rx) = test_app(false);
        app.screen = Screen::Calendar;
        app.handle_command(Command::CreateItem);
        let form = app.popups.focused_popup().unwrap().form.as_ref().unwrap();
        assert!(form.field("Start").contains('T'), "start prefilled from grid");
        assert_eq!(form.field("Duration"), "PT1H");
        // title required
        app.handle_command(Command::Submit);
        assert!(drain(&mut rx).is_empty());
        app.handle_command(Command::EditPopup);
        app.handle_command(Command::EditorChar('X'));
        app.handle_command(Command::Submit);
        let cmds = drain(&mut rx);
        assert!(matches!(cmds[0], Command::CreateEvent { ref title, .. } if title == "X"));

        // edit an existing event
        app.calendar.on_loaded(vec![EventEntry {
            id: "e1".into(),
            title: "Standup".into(),
            start: format!("{}T09:00:00", app.calendar.selected.iso()),
            duration: "PT30M".into(),
            status: "confirmed".into(),
        }]);
        app.handle_command(Command::EditItem);
        app.handle_command(Command::Submit);
        let cmds = drain(&mut rx);
        assert!(matches!(
            cmds[0],
            Command::UpdateEvent { ref id, ref title, .. } if id == "e1" && title == "Standup"
        ));
    }

    #[test]
    fn reply_prefills_recipient_subject_and_quote() {
        let (mut app, mut rx) = test_app(false);
        app.process(&Message::Event(Event::MailPageLoaded {
            mails: vec![crate::tui::types::MailEntry {
                id: "m1".into(),
                subject: "Hello".into(),
                from: "Alice <alice@x.io>".into(),
                date: "2026-07-13".into(),
                preview: "line one".into(),
                folder_id: Some("in".into()),
            }],
            position: 0,
            all_folders: false,
        }));
        for _ in 0..100 {
            app.mail.tick();
            if app.mail.matched_count == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        app.handle_command(Command::Reply);
        let form = app.popups.focused_popup().unwrap().form.as_ref().unwrap();
        assert_eq!(form.field("To"), "alice@x.io");
        assert_eq!(form.field("Subject"), "Re: Hello");
        let body = form.body.clone().unwrap();
        assert!(body.contains("> line one"));
        assert!(body.contains("-- sig"));

        // send it
        app.handle_command(Command::Submit);
        let cmds = drain(&mut rx);
        assert!(matches!(
            cmds[0],
            Command::SendMail { ref to, ref subject, .. }
                if to == "alice@x.io" && subject == "Re: Hello"
        ));
    }

    #[test]
    fn send_requires_recipient() {
        let (mut app, mut rx) = test_app(false);
        app.handle_command(Command::CreateItem); // compose
        app.handle_command(Command::Submit);
        assert!(drain(&mut rx).is_empty());
        assert!(app.popups.has_active(), "popup stays open");
    }

    #[test]
    fn action_completed_reloads_current_screen() {
        let (mut app, mut rx) = test_app(false);
        app.screen = Screen::Contacts;
        app.process(&Message::Event(Event::ActionCompleted("✓ done".into())));
        let cmds = drain(&mut rx);
        assert!(cmds.contains(&Command::LoadFolders));
        assert!(cmds.contains(&Command::LoadContacts));
        assert_eq!(app.tooltip.as_ref().unwrap().text, "✓ done");
    }

    #[test]
    fn extract_address_variants() {
        assert_eq!(extract_address("Alice <a@x.io>"), "a@x.io");
        assert_eq!(extract_address("a@x.io"), "a@x.io");
        assert_eq!(extract_address("  b@y.z  "), "b@y.z");
    }

    #[test]
    fn key_mode_priority() {
        let (mut app, _rx) = test_app(true);
        assert_eq!(app.key_mode(), KeyMode::Normal(Screen::Mail));
        app.search_active = true;
        assert_eq!(app.key_mode(), KeyMode::Search);
        app.handle_command(Command::CreateItem); // opens editor popup + editing
        assert_eq!(app.key_mode(), KeyMode::Editing);
        app.editing = false;
        assert_eq!(app.key_mode(), KeyMode::Popup);
        app.confirm = Some(PendingConfirm::DeleteMail("x".into()));
        assert_eq!(app.key_mode(), KeyMode::Confirm);
        app.quit_dialog = true;
        assert_eq!(app.key_mode(), KeyMode::QuitDialog);
    }
}
