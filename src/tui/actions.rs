//! Action handling: keyboard actions → state changes and queued server ops.

use ratatui::text::Line;

use super::editor;
use super::event::{Action, ConfirmAction, InputMode};
use super::popout::{Popout, PopoutKind};
use super::state::{App, Mode, Panel, PendingAction};

impl App {
    /// Process a user action.
    pub(crate) fn handle_action(&mut self, action: Action) {
        // Quit confirm dialog swallows everything
        if self.show_quit_confirm {
            match action {
                Action::NavigateRight | Action::OpenItem => self.should_quit = true,
                _ => self.show_quit_confirm = false,
            }
            return;
        }

        // Confirm state: y executes, n/Esc cancels, everything else is ignored
        if let InputMode::Confirm(ref confirm_action) = self.input_mode {
            let confirm_action = confirm_action.clone();
            match action {
                Action::ConfirmYes => {
                    self.input_mode = self.pre_confirm_mode.take().unwrap_or(InputMode::Normal);
                    self.execute_confirmed(confirm_action);
                }
                Action::ConfirmNo => {
                    self.input_mode = self.pre_confirm_mode.take().unwrap_or(InputMode::Normal);
                    self.status_message = Some("Cancelled".to_string());
                }
                _ => {}
            }
            return;
        }

        match action {
            Action::Quit => self.show_quit_confirm = true,
            Action::SelectPrev => self.select_prev(),
            Action::SelectNext => self.select_next(),
            Action::NavigateRight => self.navigate_right(),
            Action::NavigateLeft => self.navigate_left(),
            Action::SwitchModeNext => self.switch_mode(self.mode.next()),
            Action::SwitchModePrev => self.switch_mode(self.mode.prev()),
            Action::EnterSearch => self.input_mode = InputMode::Search,
            Action::ExitToNormal => self.input_mode = InputMode::Normal,
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

            // --- Context actions ---
            Action::Create => self.create_for_context(),
            Action::Reply => self.reply_selected(),
            Action::Forward => self.forward_selected(),
            Action::Archive => self.request_mail_confirm("archive"),
            Action::Delete => self.delete_for_context(),
            Action::Spam => self.request_mail_confirm("spam"),
            Action::Send => self.send_or_save_focused(),

            // --- Popout management ---
            Action::TogglePopout(idx) => {
                self.popouts.toggle(idx);
                self.sync_overlay_mode();
            }
            Action::MinimizeOverlay => {
                self.popouts.minimize_focused();
                self.sync_overlay_mode();
            }
            Action::CloseOverlay => {
                let discarded = self.popouts.focused_popout().is_some_and(Popout::is_editor);
                self.popouts.close_focused();
                self.sync_overlay_mode();
                if discarded {
                    self.status_message = Some("Draft discarded".to_string());
                }
            }
            Action::ToggleMaximize => self.popouts.toggle_maximize(),
            Action::FocusNextPopout => self.popouts.focus_next(),
            Action::EditPopout => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    if p.is_editor() {
                        self.input_mode = InputMode::Editing;
                    }
                }
            }

            // --- Editor actions ---
            Action::EditorChar(c) => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    editor::insert_char(p, c);
                }
            }
            Action::EditorBackspace => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    editor::backspace(p);
                }
            }
            Action::EditorEnter => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    editor::enter(p);
                }
            }
            Action::EditorNextField => {
                if let Some(p) = self.popouts.focused_popout_mut() {
                    p.next_field();
                    editor::refresh_body(p);
                }
            }
            Action::EditorEscape => {
                self.input_mode = InputMode::Overlay;
                self.status_message =
                    Some("'s' send/save · 'i' edit · Esc minimize · 'x' discard".to_string());
            }

            Action::ConfirmYes | Action::ConfirmNo | Action::None => {}
        }
    }

    // -----------------------------------------------------------------------
    // Navigation
    // -----------------------------------------------------------------------

    fn navigate_right(&mut self) {
        match self.panel {
            Panel::Profiles => {
                if let Some(name) = self.selected_id().map(str::to_string) {
                    self.active_profile_name = name;
                    self.client = None; // Will be re-created
                    self.panel = Panel::Folders;
                    self.reset_for_panel();
                }
            }
            Panel::Folders => {
                let folder = self
                    .selected_id()
                    .and_then(|id| self.folders.iter().find(|f| f.id == id))
                    .map(|f| (f.id.clone(), f.name.clone()));
                if let Some((id, name)) = folder {
                    self.active_folder_id = Some(id);
                    self.active_folder_name = name;
                    self.panel = Panel::Mails;
                    self.reset_for_panel();
                }
            }
            // No deeper level; preview is already shown on selection
            Panel::Mails | Panel::Contacts | Panel::Calendar => {}
        }
    }

    fn navigate_left(&mut self) {
        match self.panel {
            Panel::Mails => {
                self.panel = Panel::Folders;
                self.reset_for_panel();
            }
            Panel::Folders | Panel::Contacts | Panel::Calendar => {
                self.panel = Panel::Profiles;
                self.reset_for_panel();
            }
            Panel::Profiles => {}
        }
    }

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

    /// Open/select the current item. Mails open in a popout overlay.
    fn open_item(&mut self) {
        match self.panel {
            Panel::Mails => {
                if let Some(mail) = self.selected_mail() {
                    let mut body = vec![
                        Line::from(format!("From:    {}", mail.from)),
                        Line::from(format!("Date:    {}", mail.date)),
                        Line::from(format!("Subject: {}", mail.subject)),
                        Line::from(""),
                        Line::from("─".repeat(40)),
                        Line::from(""),
                    ];
                    for line in mail.preview.lines() {
                        body.push(Line::from(line.to_string()));
                    }
                    let popout = Popout::email_view(mail.subject.clone(), body, mail.id.clone());
                    self.popouts.open(popout);
                    self.input_mode = InputMode::Overlay;
                }
            }
            Panel::Folders | Panel::Profiles => self.navigate_right(),
            Panel::Contacts | Panel::Calendar => {}
        }
    }

    // -----------------------------------------------------------------------
    // Create / edit popouts
    // -----------------------------------------------------------------------

    /// `c` — create something new, depending on the current view.
    fn create_for_context(&mut self) {
        match self.panel {
            Panel::Mails | Panel::Folders | Panel::Profiles => self.open_compose(),
            Panel::Contacts => self.open_editor_popout(Popout::contact_form()),
            Panel::Calendar => {
                let now = crate::jmap::calendar::utc_now_iso8601();
                self.open_editor_popout(Popout::event_form(&now));
            }
        }
    }

    fn open_compose(&mut self) {
        let sig = self.signature();
        self.open_editor_popout(Popout::compose(sig.as_deref()));
    }

    fn open_editor_popout(&mut self, popout: Popout) {
        self.popouts.open(popout);
        if let Some(p) = self.popouts.focused_popout_mut() {
            editor::refresh_body(p);
        }
        self.input_mode = InputMode::Editing;
    }

    /// Reply to the selected/open email.
    fn reply_selected(&mut self) {
        if self.focused_is_mail_editor() {
            self.status_message = Some("Already composing".to_string());
            return;
        }
        let mail = self.open_or_selected_mail();
        if let Some((id, from, subject, preview)) = mail {
            let sig = self.signature();
            self.open_editor_popout(Popout::reply(id, &from, &subject, &preview, sig.as_deref()));
        } else {
            self.status_message = Some("No email selected".to_string());
        }
    }

    /// Forward the selected/open email.
    fn forward_selected(&mut self) {
        if self.focused_is_mail_editor() {
            self.status_message = Some("Already composing".to_string());
            return;
        }
        let mail = self.open_or_selected_mail();
        if let Some((id, _from, subject, preview)) = mail {
            let sig = self.signature();
            self.open_editor_popout(Popout::forward(id, &subject, &preview, sig.as_deref()));
        } else {
            self.status_message = Some("No email selected".to_string());
        }
    }

    fn focused_is_mail_editor(&self) -> bool {
        matches!(self.input_mode, InputMode::Overlay | InputMode::Editing)
            && self
                .popouts
                .focused_popout()
                .is_some_and(Popout::has_body_editor)
    }

    /// The email targeted by a context action: the focused email-view popout,
    /// or the selected list row.
    fn open_or_selected_mail(&self) -> Option<(String, String, String, String)> {
        let id = if self.input_mode == InputMode::Overlay {
            match self.popouts.focused_popout().map(|p| &p.kind) {
                Some(PopoutKind::EmailView { email_id }) => Some(email_id.clone()),
                _ => None,
            }
        } else {
            None
        };
        let id = id.or_else(|| self.selected_id().map(str::to_string))?;
        self.mails.iter().find(|m| m.id == id).map(|m| {
            (
                m.id.clone(),
                m.from.clone(),
                m.subject.clone(),
                m.preview.clone(),
            )
        })
    }

    // -----------------------------------------------------------------------
    // Send / save
    // -----------------------------------------------------------------------

    /// `s` in Overlay mode: send a draft or save a contact/event form.
    fn send_or_save_focused(&mut self) {
        let Some(p) = self.popouts.focused_popout() else {
            return;
        };
        let pending = match &p.kind {
            PopoutKind::Compose | PopoutKind::Reply { .. } | PopoutKind::Forward { .. } => {
                if p.field("To").trim().is_empty() {
                    self.status_message = Some("No recipient — press 'i' and fill To".to_string());
                    return;
                }
                PendingAction::SendMail {
                    to: p.field("To").to_string(),
                    cc: p.field("Cc").to_string(),
                    bcc: p.field("Bcc").to_string(),
                    subject: p.field("Subject").to_string(),
                    body: p.editor_buffer.clone(),
                }
            }
            PopoutKind::ContactForm => {
                if p.field("Name").trim().is_empty() {
                    self.status_message = Some("Name is required".to_string());
                    return;
                }
                PendingAction::CreateContact {
                    name: p.field("Name").to_string(),
                    email: p.field("Email").to_string(),
                    phone: p.field("Phone").to_string(),
                }
            }
            PopoutKind::EventForm => {
                if p.field("Title").trim().is_empty() {
                    self.status_message = Some("Title is required".to_string());
                    return;
                }
                PendingAction::CreateEvent {
                    title: p.field("Title").to_string(),
                    start: p.field("Start").to_string(),
                    duration: p.field("Duration").to_string(),
                }
            }
            PopoutKind::EmailView { .. } => return,
        };
        self.pending.push(pending);
        self.needs_reload = true;
        self.popouts.close_focused();
        self.sync_overlay_mode();
    }

    // -----------------------------------------------------------------------
    // Destructive actions (with confirmation)
    // -----------------------------------------------------------------------

    /// `d` — delete the selected item, depending on the current view.
    fn delete_for_context(&mut self) {
        // Discard a focused editor popout instead of deleting list items
        if matches!(self.input_mode, InputMode::Overlay)
            && self.popouts.focused_popout().is_some_and(Popout::is_editor)
        {
            self.handle_action(Action::CloseOverlay);
            return;
        }
        match self.panel {
            Panel::Mails => self.request_mail_confirm("delete"),
            Panel::Contacts => {
                if let Some(id) = self.selected_id().map(str::to_string) {
                    self.request_confirm(ConfirmAction::DeleteContact(id));
                } else {
                    self.status_message = Some("No contact selected".to_string());
                }
            }
            Panel::Calendar => {
                if let Some(id) = self.selected_id().map(str::to_string) {
                    self.request_confirm(ConfirmAction::DeleteEvent(id));
                } else {
                    self.status_message = Some("No event selected".to_string());
                }
            }
            _ => self.status_message = Some("Nothing to delete here".to_string()),
        }
    }

    /// Ask for confirmation of a mail move (archive/delete/spam).
    pub(crate) fn request_mail_confirm(&mut self, target: &str) {
        if self.panel != Panel::Mails {
            self.status_message = Some("Mail actions only available in Mail view".to_string());
            return;
        }
        let mail_id = self.open_or_selected_mail().map(|(id, _, _, _)| id);
        let Some(mail_id) = mail_id else {
            self.status_message = Some("No email selected".to_string());
            return;
        };
        let confirm = match target {
            "delete" => ConfirmAction::DeleteMail(mail_id),
            "archive" => ConfirmAction::ArchiveMail(mail_id),
            "spam" => ConfirmAction::SpamMail(mail_id),
            _ => return,
        };
        self.request_confirm(confirm);
    }

    fn request_confirm(&mut self, action: ConfirmAction) {
        // Skip confirmation if disabled in the profile config
        let confirm_enabled = self
            .config
            .profiles
            .get(&self.active_profile_name)
            .map(|p| p.confirm_actions)
            .unwrap_or(true);

        if confirm_enabled {
            self.pre_confirm_mode = Some(self.input_mode.clone());
            self.input_mode = InputMode::Confirm(action);
        } else {
            self.execute_confirmed(action);
        }
    }

    /// Execute a confirmed destructive action (after 'y').
    fn execute_confirmed(&mut self, action: ConfirmAction) {
        let pending = match action {
            ConfirmAction::DeleteMail(id) => self.mail_move_action(&id, "delete"),
            ConfirmAction::ArchiveMail(id) => self.mail_move_action(&id, "archive"),
            ConfirmAction::SpamMail(id) => self.mail_move_action(&id, "spam"),
            ConfirmAction::DeleteContact(id) => Some(PendingAction::DeleteContact(id)),
            ConfirmAction::DeleteEvent(id) => Some(PendingAction::DeleteEvent(id)),
        };
        if let Some(pending) = pending {
            self.pending.push(pending);
            self.needs_reload = true;
        }
    }

    /// Build a mail move for a named target (folder resolved from config).
    fn mail_move_action(&mut self, email_id: &str, target: &str) -> Option<PendingAction> {
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
            _ => return None,
        };

        let target_folder_id = self
            .folders
            .iter()
            .find(|f| f.name == target_folder_name)
            .map(|f| f.id.clone());
        let Some(target_folder_id) = target_folder_id else {
            self.status_message = Some(format!("Folder not found: {target_folder_name}"));
            return None;
        };

        Some(PendingAction::Move {
            email_id: email_id.to_string(),
            source_mailbox_id: self.active_folder_id.clone().unwrap_or_default(),
            target_mailbox_id: target_folder_id,
            action_name: target.to_string(),
        })
    }

    // -----------------------------------------------------------------------
    // Slash commands
    // -----------------------------------------------------------------------

    /// Execute the current input as a slash command or open the selection.
    fn execute_input_command(&mut self) {
        if self.input.starts_with('/') {
            let cmd = self.input.trim_start_matches('/').trim().to_lowercase();
            self.execute_slash_command(&cmd);
            self.input.clear();
            self.update_search();
            if self.input_mode == InputMode::Search {
                self.input_mode = InputMode::Normal;
            }
        } else {
            self.open_item();
            if self.input_mode == InputMode::Search {
                self.input_mode = InputMode::Normal;
            }
        }
    }

    fn execute_slash_command(&mut self, cmd: &str) {
        match cmd {
            // --- Mail commands ---
            "reply" | "r" => self.reply_selected(),
            "forward" | "f" => self.forward_selected(),
            "archive" | "a" => self.request_mail_confirm("archive"),
            "delete" | "del" | "d" => self.request_mail_confirm("delete"),
            "spam" | "s" => self.request_mail_confirm("spam"),
            "compose" | "new" | "c" => self.open_compose(),

            // --- Contact commands ---
            "add-contact" | "new-contact" => self.open_editor_popout(Popout::contact_form()),
            "delete-contact" => {
                if let Some(id) = self.selected_id().map(str::to_string) {
                    self.request_confirm(ConfirmAction::DeleteContact(id));
                } else {
                    self.status_message = Some("No contact selected".to_string());
                }
            }

            // --- Calendar commands ---
            "add-event" | "new-event" => {
                let now = crate::jmap::calendar::utc_now_iso8601();
                self.open_editor_popout(Popout::event_form(&now));
            }
            "delete-event" => {
                if let Some(id) = self.selected_id().map(str::to_string) {
                    self.request_confirm(ConfirmAction::DeleteEvent(id));
                } else {
                    self.status_message = Some("No event selected".to_string());
                }
            }

            _ => {
                self.status_message = Some(format!("Unknown command: /{cmd}"));
            }
        }
    }
}
