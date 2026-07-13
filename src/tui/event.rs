//! Terminal event handling — vi-style Normal/Insert/EmailOpen modes.
//!
//! Three modes:
//! - Normal: j/k navigate, h/l drill, Enter opens, / or i enters Insert, q quits
//! - Insert: type to search (slash commands), Esc returns to Normal
//! - EmailOpen: email is selected/open — r reply, f forward, a archive, d delete, Esc closes

use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

/// Poll for a terminal event with a timeout.
pub fn poll_event(timeout: Duration) -> std::io::Result<Option<Event>> {
    if event::poll(timeout)? {
        Ok(Some(event::read()?))
    } else {
        Ok(None)
    }
}

/// A destructive mail action awaiting user confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Delete an email (move to Trash). Contains the email JMAP id.
    Delete(String),
    /// Archive an email. Contains the email JMAP id.
    Archive(String),
    /// Mark an email as spam. Contains the email JMAP id.
    Spam(String),
}

impl ConfirmAction {
    /// Human-readable prompt for this action.
    pub fn prompt(&self) -> &'static str {
        match self {
            ConfirmAction::Delete(_) => "Delete this email? [y/n]",
            ConfirmAction::Archive(_) => "Archive this email? [y/n]",
            ConfirmAction::Spam(_) => "Mark as spam? [y/n]",
        }
    }

    /// The target folder name (used to resolve the JMAP action).
    pub fn target_name(&self) -> &'static str {
        match self {
            ConfirmAction::Delete(_) => "delete",
            ConfirmAction::Archive(_) => "archive",
            ConfirmAction::Spam(_) => "spam",
        }
    }

    /// The email id associated with this action.
    pub fn email_id(&self) -> &str {
        match self {
            ConfirmAction::Delete(id) | ConfirmAction::Archive(id) | ConfirmAction::Spam(id) => id,
        }
    }
}

/// Vi-style input mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    /// Normal mode: navigation and structural commands.
    Normal,
    /// Insert mode: typing into search / slash commands.
    Insert,
    /// An email is open/selected — single-key actions available.
    EmailOpen,
    /// Editing a draft/reply/forward in a popout — text goes to the editor buffer.
    Editing,
    /// Awaiting confirmation for a destructive action.
    Confirm(ConfirmAction),
}

/// Actions that can result from a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Quit the application.
    Quit,
    /// Move selection up in the results list.
    SelectPrev,
    /// Move selection down in the results list.
    SelectNext,
    /// Navigate deeper (enter a folder, open a mail, select a profile).
    NavigateRight,
    /// Navigate back (from mails → folders → profiles).
    NavigateLeft,
    /// Switch to the next top-level mode (Mail → Contacts → Calendar).
    SwitchModeNext,
    /// Switch to the previous top-level mode.
    SwitchModePrev,
    /// Enter Insert mode (start typing search/commands).
    EnterInsert,
    /// Return to Normal mode (from Insert or EmailOpen).
    ExitToNormal,
    /// Open/select the current item (Enter on an email → EmailOpen).
    OpenItem,
    /// Append a character to the search input.
    InsertChar(char),
    /// Delete the last character from the search input.
    Backspace,
    /// Clear the entire search input.
    ClearInput,
    /// Execute the current input as a slash command (Enter in Insert mode with /...).
    ExecuteCommand,
    // --- Email-open single-key actions ---
    /// Reply to the open email.
    Reply,
    /// Forward the open email.
    Forward,
    /// Archive the open email.
    Archive,
    /// Delete the open email.
    Delete,
    /// Send the current draft/reply/forward.
    Send,
    /// Compose a new email.
    Compose,
    /// Mark as spam.
    Spam,
    // --- Header editing (draft/reply/forward popout) ---
    /// Edit the To field.
    EditTo,
    /// Edit the Subject field.
    EditSubject,
    /// Edit/add CC field.
    EditCc,
    /// Edit/add BCC field.
    EditBcc,
    // --- Popout actions ---
    /// Close the focused popout.
    PopoutClose,
    /// Toggle maximize/normal on focused popout.
    PopoutToggleMax,
    /// Minimize the focused popout.
    PopoutMinimize,
    /// Switch focus between popouts.
    PopoutSwitchFocus,
    // --- Editor actions (Editing mode) ---
    /// Insert a character into the editor buffer.
    EditorChar(char),
    /// Delete last character in editor.
    EditorBackspace,
    /// Insert newline in editor.
    EditorNewline,
    /// Escape from editing: save draft and return to EmailOpen.
    EditorEscape,
    /// No-op (unhandled key).
    None,
    /// Confirm the pending destructive action.
    ConfirmYes,
    /// Cancel the pending destructive action.
    ConfirmNo,
}

/// Map a key event to an action based on the current input mode.
pub fn map_key(key: KeyEvent, mode: &InputMode) -> Action {
    // Ctrl+C always quits
    if matches!(
        key,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    ) {
        return Action::Quit;
    }

    match mode {
        InputMode::Normal => map_normal(key),
        InputMode::Insert => map_insert(key),
        InputMode::EmailOpen => map_email_open(key),
        InputMode::Editing => map_editing(key),
        InputMode::Confirm(_) => map_confirm(key),
    }
}

/// Normal mode: navigation, drill, open, search entry.
fn map_normal(key: KeyEvent) -> Action {
    match key {
        // Quit
        KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Esc, ..
        } => Action::Quit,

        // Navigation: j/k or arrows
        KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Up, ..
        } => Action::SelectPrev,
        KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Down,
            ..
        } => Action::SelectNext,

        // Drill: h/l or arrows
        KeyEvent {
            code: KeyCode::Char('h'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Left,
            ..
        } => Action::NavigateLeft,
        KeyEvent {
            code: KeyCode::Char('l'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Right,
            ..
        } => Action::NavigateRight,

        // Open item (Enter)
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => Action::OpenItem,

        // Tab switching
        KeyEvent {
            code: KeyCode::Tab, ..
        } => Action::SwitchModeNext,
        KeyEvent {
            code: KeyCode::BackTab,
            ..
        } => Action::SwitchModePrev,

        // Enter Insert mode: / or i
        KeyEvent {
            code: KeyCode::Char('/'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::EnterInsert,
        KeyEvent {
            code: KeyCode::Char('i'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::EnterInsert,

        // Direct mail actions from list view (context-aware in app.rs)
        KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Reply,
        KeyEvent {
            code: KeyCode::Char('f'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Forward,
        KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Archive,
        KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Delete,
        KeyEvent {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Spam,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Compose,

        _ => Action::None,
    }
}

/// Insert mode: type to search, slash commands, Esc exits.
fn map_insert(key: KeyEvent) -> Action {
    match key {
        // Exit Insert mode
        KeyEvent {
            code: KeyCode::Esc, ..
        } => Action::ExitToNormal,

        // Navigation still works with arrows
        KeyEvent {
            code: KeyCode::Up, ..
        } => Action::SelectPrev,
        KeyEvent {
            code: KeyCode::Down,
            ..
        } => Action::SelectNext,

        // Enter: execute command if starts with /, otherwise open item
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => Action::ExecuteCommand,

        // Editing
        KeyEvent {
            code: KeyCode::Backspace,
            ..
        } => Action::Backspace,
        KeyEvent {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => Action::ClearInput,

        // Character input
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            ..
        } => Action::InsertChar(c),

        _ => Action::None,
    }
}

/// EmailOpen mode: single-key actions on the open email/draft.
fn map_email_open(key: KeyEvent) -> Action {
    match key {
        // Close popout (back to list)
        KeyEvent {
            code: KeyCode::Esc, ..
        }
        | KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::PopoutClose,

        // Email/draft actions
        KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Reply,
        KeyEvent {
            code: KeyCode::Char('f'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Forward,
        KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Archive,
        KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Delete,
        KeyEvent {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::Send,

        // Header editing (for drafts/replies/forwards)
        KeyEvent {
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::EditTo,
        KeyEvent {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::EditSubject,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::EditCc,
        KeyEvent {
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::EditBcc,

        // Popout window management
        KeyEvent {
            code: KeyCode::Char('m'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::PopoutToggleMax,
        KeyEvent {
            code: KeyCode::Char('n'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::PopoutMinimize,
        KeyEvent {
            code: KeyCode::Tab, ..
        } => Action::PopoutSwitchFocus,

        // Re-enter editing mode
        KeyEvent {
            code: KeyCode::Char('i'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Char('e'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Action::EnterInsert,

        // Scroll within the email
        KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Down,
            ..
        } => Action::SelectNext,
        KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Up, ..
        } => Action::SelectPrev,

        _ => Action::None,
    }
}

/// Editing mode: typing into the editor buffer (compose/reply/forward).
/// Esc saves draft and returns to EmailOpen view.
fn map_editing(key: KeyEvent) -> Action {
    match key {
        // Escape: save draft, exit to EmailOpen
        KeyEvent {
            code: KeyCode::Esc, ..
        } => Action::EditorEscape,

        // Newline
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => Action::EditorNewline,

        // Editing
        KeyEvent {
            code: KeyCode::Backspace,
            ..
        } => Action::EditorBackspace,

        // Character input
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            ..
        } => Action::EditorChar(c),

        _ => Action::None,
    }
}

/// Confirm mode: awaiting y/n response for a destructive action.
/// Only y confirms, n/Esc cancels, everything else is ignored.
fn map_confirm(key: KeyEvent) -> Action {
    match key {
        KeyEvent {
            code: KeyCode::Char('y') | KeyCode::Char('Y'),
            ..
        } => Action::ConfirmYes,
        KeyEvent {
            code: KeyCode::Char('n') | KeyCode::Char('N'),
            ..
        }
        | KeyEvent {
            code: KeyCode::Esc, ..
        } => Action::ConfirmNo,
        _ => Action::None,
    }
}
