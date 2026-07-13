//! Popout overlay system for Herald TUI.
//!
//! Opened emails, drafts, and contact/event forms appear as overlays on top of
//! the main app. Up to [`MAX_POPOUTS`] popouts can be open at once; each is
//! addressable by a number key (1–9, 0 = tenth). A popout is either *active*
//! (shown as an overlay) or *minimized* (listed only in the popout bar). At
//! most two popouts are shown side by side; activating a third minimizes the
//! oldest active one. When every popout is minimized, the main app has focus.

use ratatui::text::Line;

/// Maximum number of open popouts (addressable via keys 1–9 and 0).
pub const MAX_POPOUTS: usize = 10;

/// Maximum number of popouts shown as overlays at the same time.
pub const MAX_ACTIVE: usize = 2;

/// Visual state of a popout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopoutState {
    /// Only listed in the popout bar.
    Minimized,
    /// Shown as an overlay.
    Normal,
    /// Shown as a full-screen overlay.
    Maximized,
}

/// The kind of content in a popout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopoutKind {
    /// Viewing an email (read-only).
    EmailView { email_id: String },
    /// Composing a new draft.
    Compose,
    /// Replying to an email.
    Reply { original_id: String },
    /// Forwarding an email.
    Forward { original_id: String },
    /// Creating a new contact.
    ContactForm,
    /// Creating a new calendar event.
    EventForm,
}

impl PopoutKind {
    pub fn icon(&self) -> &'static str {
        match self {
            PopoutKind::EmailView { .. } => "📧",
            PopoutKind::Compose => "✏",
            PopoutKind::Reply { .. } => "↩",
            PopoutKind::Forward { .. } => "➡",
            PopoutKind::ContactForm => "📇",
            PopoutKind::EventForm => "📅",
        }
    }
}

/// One editable line in a popout form (To/Subject for mail, Name for contacts…).
#[derive(Debug, Clone)]
pub struct FormField {
    pub label: &'static str,
    pub value: String,
}

impl FormField {
    fn new(label: &'static str) -> Self {
        Self {
            label,
            value: String::new(),
        }
    }

    fn with_value(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
        }
    }
}

/// A single popout panel.
#[derive(Debug, Clone)]
pub struct Popout {
    /// Display title for the panel border and popout bar.
    pub title: String,
    /// Body content lines (rebuilt from fields + editor buffer for editors).
    pub body: Vec<Line<'static>>,
    /// Current visual state.
    pub state: PopoutState,
    /// What kind of content this popout holds.
    pub kind: PopoutKind,
    /// Editable body text (compose/reply/forward).
    pub editor_buffer: String,
    /// Cursor position within editor_buffer (byte offset).
    pub editor_cursor: usize,
    /// Editable header/form fields.
    pub fields: Vec<FormField>,
    /// Index of the field being edited (None = body editor, for mail editors).
    pub active_field: Option<usize>,
}

impl Popout {
    fn new(title: String, kind: PopoutKind, fields: Vec<FormField>) -> Self {
        Self {
            title,
            body: Vec::new(),
            state: PopoutState::Normal,
            kind,
            editor_buffer: String::new(),
            editor_cursor: 0,
            fields,
            active_field: None,
        }
    }

    /// A read-only popout showing an opened email.
    pub fn email_view(title: String, body: Vec<Line<'static>>, email_id: String) -> Self {
        let mut p = Self::new(title, PopoutKind::EmailView { email_id }, Vec::new());
        p.body = body;
        p
    }

    /// A new draft. Editing starts on the To field.
    pub fn compose(signature: Option<&str>) -> Self {
        let mut p = Self::new(
            "New Draft".to_string(),
            PopoutKind::Compose,
            mail_fields("", ""),
        );
        p.editor_buffer = signature_block(signature);
        p.active_field = Some(0);
        p
    }

    /// A reply. To/Subject are prefilled; editing starts in the body.
    pub fn reply(
        original_id: String,
        from: &str,
        subject: &str,
        quoted: &str,
        signature: Option<&str>,
    ) -> Self {
        let mut p = Self::new(
            format!("Re: {subject}"),
            PopoutKind::Reply { original_id },
            mail_fields(from, &format!("Re: {subject}")),
        );
        let mut buffer = signature_block(signature);
        buffer.push_str("\n\n");
        for line in quoted.lines() {
            buffer.push_str("> ");
            buffer.push_str(line);
            buffer.push('\n');
        }
        p.editor_buffer = buffer;
        p
    }

    /// A forward. Editing starts on the (empty) To field.
    pub fn forward(
        original_id: String,
        subject: &str,
        content: &str,
        signature: Option<&str>,
    ) -> Self {
        let mut p = Self::new(
            format!("Fwd: {subject}"),
            PopoutKind::Forward { original_id },
            mail_fields("", &format!("Fwd: {subject}")),
        );
        let mut buffer = signature_block(signature);
        buffer.push_str("\n\n--- Forwarded message ---\n\n");
        buffer.push_str(content);
        p.editor_buffer = buffer;
        p.active_field = Some(0);
        p
    }

    /// A form for creating a new contact.
    pub fn contact_form() -> Self {
        let mut p = Self::new(
            "New Contact".to_string(),
            PopoutKind::ContactForm,
            vec![
                FormField::new("Name"),
                FormField::new("Email"),
                FormField::new("Phone"),
            ],
        );
        p.active_field = Some(0);
        p
    }

    /// A form for creating a new calendar event. Start is prefilled with now.
    pub fn event_form(now_iso8601: &str) -> Self {
        let mut p = Self::new(
            "New Event".to_string(),
            PopoutKind::EventForm,
            vec![
                FormField::new("Title"),
                FormField::with_value("Start", now_iso8601),
                FormField::with_value("Duration", "PT1H"),
            ],
        );
        p.active_field = Some(0);
        p
    }

    /// Is this popout editable (any form or mail editor)?
    pub fn is_editor(&self) -> bool {
        !matches!(self.kind, PopoutKind::EmailView { .. })
    }

    /// Does this popout have a free-text body editor below its fields?
    pub fn has_body_editor(&self) -> bool {
        matches!(
            self.kind,
            PopoutKind::Compose | PopoutKind::Reply { .. } | PopoutKind::Forward { .. }
        )
    }

    /// The value of the field with the given label ("" if absent).
    pub fn field(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map(|f| f.value.as_str())
            .unwrap_or("")
    }

    /// Advance editing to the next field; after the last field, move to the
    /// body editor (mail) or wrap to the first field (forms).
    pub fn next_field(&mut self) {
        self.active_field = match self.active_field {
            Some(i) if i + 1 < self.fields.len() => Some(i + 1),
            Some(_) if self.has_body_editor() => None,
            Some(_) => Some(0),
            None => Some(0),
        };
    }
}

fn mail_fields(to: &str, subject: &str) -> Vec<FormField> {
    vec![
        FormField::with_value("To", to),
        FormField::new("Cc"),
        FormField::new("Bcc"),
        FormField::with_value("Subject", subject),
    ]
}

fn signature_block(signature: Option<&str>) -> String {
    signature
        .map(|s| format!("\n\n-- \n{s}"))
        .unwrap_or_default()
}

/// Manages the set of open popouts.
#[derive(Debug, Clone, Default)]
pub struct PopoutManager {
    /// Open popouts, in the order they appear in the popout bar (1-based keys).
    pub popouts: Vec<Popout>,
    /// Index of the currently focused popout (must be active if set).
    pub focused: Option<usize>,
}

impl PopoutManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a new popout as an active overlay and focus it.
    /// At capacity the oldest popout is closed first.
    pub fn open(&mut self, popout: Popout) {
        if self.popouts.len() >= MAX_POPOUTS {
            self.popouts.remove(0);
        }
        self.popouts.push(popout);
        let idx = self.popouts.len() - 1;
        self.enforce_active_limit(idx);
        self.focused = Some(idx);
    }

    /// Toggle popout `idx` between active and minimized (number-key handler).
    pub fn toggle(&mut self, idx: usize) {
        let Some(popout) = self.popouts.get_mut(idx) else {
            return;
        };
        if popout.state == PopoutState::Minimized {
            popout.state = PopoutState::Normal;
            self.enforce_active_limit(idx);
            self.focused = Some(idx);
        } else {
            popout.state = PopoutState::Minimized;
            if self.focused == Some(idx) {
                self.focused = self.first_active();
            }
        }
    }

    /// Minimize the focused popout; focus moves to another active one (if any).
    pub fn minimize_focused(&mut self) {
        if let Some(idx) = self.focused {
            if let Some(popout) = self.popouts.get_mut(idx) {
                popout.state = PopoutState::Minimized;
            }
            self.focused = self.first_active();
        }
    }

    /// Close (discard) the focused popout entirely.
    pub fn close_focused(&mut self) {
        if let Some(idx) = self.focused {
            if idx < self.popouts.len() {
                self.popouts.remove(idx);
            }
            self.focused = self.first_active();
        }
    }

    /// Cycle focus through all open popouts in bar order, activating a
    /// minimized one when focus reaches it (the visible limit still applies).
    pub fn focus_next(&mut self) {
        if self.popouts.is_empty() {
            self.focused = None;
            return;
        }
        let next = match self.focused {
            Some(current) => (current + 1) % self.popouts.len(),
            None => 0,
        };
        if self.popouts[next].state == PopoutState::Minimized {
            self.popouts[next].state = PopoutState::Normal;
            self.enforce_active_limit(next);
        }
        self.focused = Some(next);
    }

    /// Toggle the focused popout between Normal and Maximized.
    pub fn toggle_maximize(&mut self) {
        if let Some(idx) = self.focused {
            if let Some(popout) = self.popouts.get_mut(idx) {
                popout.state = match popout.state {
                    PopoutState::Maximized => PopoutState::Normal,
                    _ => PopoutState::Maximized,
                };
            }
        }
    }

    pub fn focused_popout(&self) -> Option<&Popout> {
        self.focused.and_then(|idx| self.popouts.get(idx))
    }

    pub fn focused_popout_mut(&mut self) -> Option<&mut Popout> {
        self.focused.and_then(|idx| self.popouts.get_mut(idx))
    }

    /// Indices of active (non-minimized) popouts.
    pub fn active_indices(&self) -> Vec<usize> {
        self.popouts
            .iter()
            .enumerate()
            .filter(|(_, p)| p.state != PopoutState::Minimized)
            .map(|(i, _)| i)
            .collect()
    }

    /// Is any popout shown as an overlay?
    pub fn has_active(&self) -> bool {
        self.popouts
            .iter()
            .any(|p| p.state != PopoutState::Minimized)
    }

    pub fn has_maximized(&self) -> bool {
        self.popouts
            .iter()
            .any(|p| p.state == PopoutState::Maximized)
    }

    fn first_active(&self) -> Option<usize> {
        self.active_indices().first().copied()
    }

    /// Keep at most MAX_ACTIVE popouts visible; `keep` is never minimized.
    fn enforce_active_limit(&mut self, keep: usize) {
        let active = self.active_indices();
        if active.len() <= MAX_ACTIVE {
            return;
        }
        let excess = active.len() - MAX_ACTIVE;
        let mut minimized = 0;
        for idx in active {
            if minimized >= excess {
                break;
            }
            if idx != keep {
                self.popouts[idx].state = PopoutState::Minimized;
                minimized += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_activates_and_minimizes() {
        let mut mgr = PopoutManager::new();
        mgr.open(Popout::contact_form());
        assert!(mgr.has_active());
        mgr.toggle(0);
        assert!(!mgr.has_active());
        assert_eq!(mgr.focused, None);
        mgr.toggle(0);
        assert!(mgr.has_active());
        assert_eq!(mgr.focused, Some(0));
    }

    #[test]
    fn at_most_two_popouts_are_active() {
        let mut mgr = PopoutManager::new();
        mgr.open(Popout::contact_form());
        mgr.open(Popout::contact_form());
        mgr.open(Popout::contact_form());
        assert_eq!(mgr.active_indices().len(), MAX_ACTIVE);
        // The newest one stays active and focused
        assert_eq!(mgr.focused, Some(2));
        assert_ne!(mgr.popouts[2].state, PopoutState::Minimized);
    }

    #[test]
    fn minimizing_all_returns_focus_to_app() {
        let mut mgr = PopoutManager::new();
        mgr.open(Popout::contact_form());
        mgr.open(Popout::contact_form());
        mgr.minimize_focused();
        assert!(mgr.focused.is_some());
        mgr.minimize_focused();
        assert_eq!(mgr.focused, None);
        assert!(!mgr.has_active());
    }

    #[test]
    fn next_field_cycles_forms_and_falls_into_mail_body() {
        let mut contact = Popout::contact_form();
        assert_eq!(contact.active_field, Some(0));
        contact.next_field();
        contact.next_field();
        assert_eq!(contact.active_field, Some(2));
        contact.next_field(); // wraps — forms have no body editor
        assert_eq!(contact.active_field, Some(0));

        let mut compose = Popout::compose(None);
        assert_eq!(compose.active_field, Some(0)); // To
        compose.next_field(); // Cc
        compose.next_field(); // Bcc
        compose.next_field(); // Subject
        compose.next_field(); // body
        assert_eq!(compose.active_field, None);
    }

    #[test]
    fn focus_next_reaches_minimized_popouts() {
        let mut mgr = PopoutManager::new();
        mgr.open(Popout::contact_form());
        mgr.open(Popout::contact_form());
        mgr.open(Popout::contact_form());
        // Popout 0 got minimized by the active limit; focus is on 2.
        assert_eq!(mgr.focused, Some(2));
        assert_eq!(mgr.popouts[0].state, PopoutState::Minimized);

        // Tab wraps to popout 0 and re-activates it.
        mgr.focus_next();
        assert_eq!(mgr.focused, Some(0));
        assert_ne!(mgr.popouts[0].state, PopoutState::Minimized);
        assert_eq!(mgr.active_indices().len(), MAX_ACTIVE);

        // Continues in bar order through every popout.
        mgr.focus_next();
        assert_eq!(mgr.focused, Some(1));
        assert_ne!(mgr.popouts[1].state, PopoutState::Minimized);
        mgr.focus_next();
        assert_eq!(mgr.focused, Some(2));
    }

    #[test]
    fn capacity_is_capped_at_max_popouts() {
        let mut mgr = PopoutManager::new();
        for _ in 0..(MAX_POPOUTS + 3) {
            mgr.open(Popout::contact_form());
        }
        assert_eq!(mgr.popouts.len(), MAX_POPOUTS);
    }
}
