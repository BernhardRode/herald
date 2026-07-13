//! Popout window system for Herald TUI.
//!
//! Opened emails, new drafts, and reply compositions appear as popout panels.
//! Max 2 popouts can be open simultaneously, shown side-by-side.
//! Each popout can be minimized (title bar only), normal (half screen), or maximized (full overlay).

use ratatui::text::Line;

/// Maximum number of simultaneously open popouts.
pub const MAX_POPOUTS: usize = 2;

/// Visual state of a popout panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopoutState {
    /// Minimized — only shows as a tab/title bar at the bottom.
    Minimized,
    /// Normal — takes half the screen width (side-by-side with main or other popout).
    Normal,
    /// Maximized — full overlay on top of everything.
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
}

/// Which header field is being edited in a draft popout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderField {
    To,
    Subject,
    Cc,
    Bcc,
}

/// A single popout panel.
#[derive(Debug, Clone)]
pub struct Popout {
    /// Unique identifier for this popout.
    pub id: u32,
    /// Display title for the panel border.
    pub title: String,
    /// Body content lines.
    pub body: Vec<Line<'static>>,
    /// Current visual state.
    pub state: PopoutState,
    /// What kind of content this popout holds.
    pub kind: PopoutKind,
    /// Editable text buffer (for compose/reply).
    pub editor_buffer: String,
    /// Cursor position within editor_buffer (byte offset).
    pub editor_cursor: usize,
    // --- Email header fields (for drafts/replies/forwards) ---
    /// To recipients.
    pub to: String,
    /// Subject line.
    pub subject: String,
    /// CC recipients.
    pub cc: String,
    /// BCC recipients.
    pub bcc: String,
    /// Which header field is currently being edited (None = body editor).
    pub editing_field: Option<HeaderField>,
}

impl Popout {
    /// Create a new popout for viewing an email.
    pub fn email_view(id: u32, title: String, body: Vec<Line<'static>>, email_id: String) -> Self {
        Self {
            id,
            title,
            body,
            state: PopoutState::Normal,
            kind: PopoutKind::EmailView { email_id },
            editor_buffer: String::new(),
            editor_cursor: 0,
            to: String::new(),
            subject: String::new(),
            cc: String::new(),
            bcc: String::new(),
            editing_field: None,
        }
    }

    /// Create a new popout for composing a draft.
    pub fn compose(id: u32, signature: Option<&str>) -> Self {
        // Body shows the header info; editor_buffer is where typing goes
        let body = vec![
            Line::from("To: "),
            Line::from("Subject: "),
            Line::from(""),
            Line::from("─".repeat(30)),
            Line::from(""),
            Line::from(ratatui::text::Span::styled(
                "▊",
                ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
            )),
        ];

        // Pre-fill editor buffer with signature if present
        let editor_buffer = signature
            .map(|s| format!("\n\n-- \n{s}"))
            .unwrap_or_default();

        Self {
            id,
            title: "New Draft".to_string(),
            body,
            state: PopoutState::Normal,
            kind: PopoutKind::Compose,
            editor_buffer,
            editor_cursor: 0,
            to: String::new(),
            subject: String::new(),
            cc: String::new(),
            bcc: String::new(),
            editing_field: None,
        }
    }

    /// Create a new popout for replying to an email.
    /// Shows: cursor area → signature → quoted original below.
    pub fn reply(
        id: u32,
        original_id: String,
        from: &str,
        subject: &str,
        quoted: &str,
        signature: Option<&str>,
    ) -> Self {
        let body = vec![
            Line::from(format!("To: {from}")),
            Line::from(format!("Subject: Re: {subject}")),
            Line::from(""),
            Line::from("─".repeat(30)),
            Line::from(""),
            Line::from(ratatui::text::Span::styled(
                "▊",
                ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
            )),
        ];

        // Pre-fill editor with signature + quoted original
        let mut editor_buffer = String::new();
        if let Some(sig) = signature {
            editor_buffer.push_str("\n\n-- \n");
            editor_buffer.push_str(sig);
        }
        editor_buffer.push_str("\n\n");
        for line in quoted.lines() {
            editor_buffer.push_str("> ");
            editor_buffer.push_str(line);
            editor_buffer.push('\n');
        }

        Self {
            id,
            title: format!("Reply: {subject}"),
            body,
            state: PopoutState::Normal,
            kind: PopoutKind::Reply { original_id },
            editor_buffer,
            editor_cursor: 0,
            to: from.to_string(),
            subject: format!("Re: {subject}"),
            cc: String::new(),
            bcc: String::new(),
            editing_field: None,
        }
    }

    /// Create a new popout for forwarding an email.
    pub fn forward(
        id: u32,
        original_id: String,
        subject: &str,
        content: &str,
        signature: Option<&str>,
    ) -> Self {
        let body = vec![
            Line::from("To: "),
            Line::from(format!("Subject: Fwd: {subject}")),
            Line::from(""),
            Line::from("─".repeat(30)),
            Line::from(""),
            Line::from(ratatui::text::Span::styled(
                "▊",
                ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
            )),
        ];

        // Pre-fill editor with signature + forwarded content
        let mut editor_buffer = String::new();
        if let Some(sig) = signature {
            editor_buffer.push_str("\n\n-- \n");
            editor_buffer.push_str(sig);
        }
        editor_buffer.push_str("\n\n--- Forwarded message ---\n\n");
        editor_buffer.push_str(content);

        Self {
            id,
            title: format!("Fwd: {subject}"),
            body,
            state: PopoutState::Normal,
            kind: PopoutKind::Forward { original_id },
            editor_buffer,
            editor_cursor: 0,
            to: String::new(),
            subject: format!("Fwd: {subject}"),
            cc: String::new(),
            bcc: String::new(),
            editing_field: None,
        }
    }

    /// Is this popout an editor (compose/reply/forward)?
    #[allow(dead_code)]
    pub fn is_editor(&self) -> bool {
        matches!(
            self.kind,
            PopoutKind::Compose | PopoutKind::Reply { .. } | PopoutKind::Forward { .. }
        )
    }
}

/// Manages the set of open popouts (max 2).
#[derive(Debug, Clone, Default)]
pub struct PopoutManager {
    /// Open popouts (max MAX_POPOUTS).
    pub popouts: Vec<Popout>,
    /// Index of the currently focused popout (if any).
    pub focused: Option<usize>,
    /// Auto-incrementing ID counter.
    next_id: u32,
}

impl PopoutManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a new popout. If at capacity, closes the oldest one first.
    pub fn open(&mut self, mut popout: Popout) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        popout.id = id;

        if self.popouts.len() >= MAX_POPOUTS {
            // Close the first (oldest) popout
            self.popouts.remove(0);
        }

        self.popouts.push(popout);
        self.focused = Some(self.popouts.len() - 1);
        id
    }

    /// Close a popout by id.
    pub fn close(&mut self, id: u32) {
        self.popouts.retain(|p| p.id != id);
        // Adjust focus
        if self.popouts.is_empty() {
            self.focused = None;
        } else if let Some(f) = self.focused {
            if f >= self.popouts.len() {
                self.focused = Some(self.popouts.len() - 1);
            }
        }
    }

    /// Close the focused popout.
    pub fn close_focused(&mut self) {
        if let Some(idx) = self.focused {
            if idx < self.popouts.len() {
                let id = self.popouts[idx].id;
                self.close(id);
            }
        }
    }

    /// Switch focus to the other popout.
    pub fn switch_focus(&mut self) {
        if self.popouts.len() <= 1 {
            return;
        }
        self.focused = Some(match self.focused {
            Some(0) => 1,
            _ => 0,
        });
    }

    /// Toggle the focused popout's state between Normal and Maximized.
    pub fn toggle_maximize(&mut self) {
        if let Some(idx) = self.focused {
            if let Some(popout) = self.popouts.get_mut(idx) {
                popout.state = match popout.state {
                    PopoutState::Normal => PopoutState::Maximized,
                    PopoutState::Maximized => PopoutState::Normal,
                    PopoutState::Minimized => PopoutState::Normal,
                };
            }
        }
    }

    /// Minimize the focused popout.
    pub fn minimize_focused(&mut self) {
        if let Some(idx) = self.focused {
            if let Some(popout) = self.popouts.get_mut(idx) {
                popout.state = PopoutState::Minimized;
            }
        }
    }

    /// Get the focused popout (if any).
    #[allow(dead_code)]
    pub fn focused_popout(&self) -> Option<&Popout> {
        self.focused.and_then(|idx| self.popouts.get(idx))
    }

    /// Get mutable focused popout.
    #[allow(dead_code)]
    pub fn focused_popout_mut(&mut self) -> Option<&mut Popout> {
        self.focused.and_then(|idx| self.popouts.get_mut(idx))
    }

    /// Are there any visible (non-minimized) popouts?
    #[allow(dead_code)]
    pub fn has_visible(&self) -> bool {
        self.popouts
            .iter()
            .any(|p| p.state != PopoutState::Minimized)
    }

    /// Is any popout maximized?
    pub fn has_maximized(&self) -> bool {
        self.popouts
            .iter()
            .any(|p| p.state == PopoutState::Maximized)
    }

    /// Get visible (non-minimized) popouts.
    pub fn visible_popouts(&self) -> Vec<&Popout> {
        self.popouts
            .iter()
            .filter(|p| p.state != PopoutState::Minimized)
            .collect()
    }

    /// Get minimized popouts (for tab bar display).
    pub fn minimized_popouts(&self) -> Vec<&Popout> {
        self.popouts
            .iter()
            .filter(|p| p.state == PopoutState::Minimized)
            .collect()
    }
}
