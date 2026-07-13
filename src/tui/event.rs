//! Terminal event handling — context-based modes and key bindings.
//!
//! Modes:
//! - **Normal**: list navigation and context actions. `j/k` move, `h/l` walk
//!   the hierarchy, `Enter` opens, `/` searches, `Tab` switches Mail/Contacts/
//!   Calendar, `c` creates (mail draft / contact / event depending on view),
//!   `1`–`9`/`0` toggle popout overlays, `q` quits.
//! - **Search**: typing filters the list; `/command` runs a slash command.
//! - **Overlay**: a popout is focused. `Esc` minimizes it (all minimized →
//!   back in the app), `x` discards it, `s` sends/saves, `i` edits, `Tab`
//!   cycles focus, `m` maximizes, numbers toggle popouts.
//! - **Editing**: typing goes into the focused popout's field or body.
//!   `Tab`/`Enter` advance fields, `Esc` returns to Overlay.
//! - **Confirm**: `y` confirms a destructive action, `n`/`Esc` cancels.

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

/// A destructive action awaiting user confirmation. Contains the JMAP id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    DeleteMail(String),
    ArchiveMail(String),
    SpamMail(String),
    DeleteContact(String),
    DeleteEvent(String),
}

impl ConfirmAction {
    /// Human-readable prompt for this action.
    pub fn prompt(&self) -> &'static str {
        match self {
            ConfirmAction::DeleteMail(_) => "Delete this email? [y/n]",
            ConfirmAction::ArchiveMail(_) => "Archive this email? [y/n]",
            ConfirmAction::SpamMail(_) => "Mark as spam? [y/n]",
            ConfirmAction::DeleteContact(_) => "Delete this contact? [y/n]",
            ConfirmAction::DeleteEvent(_) => "Delete this event? [y/n]",
        }
    }
}

/// Input mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    /// List navigation and structural commands.
    Normal,
    /// Typing into the search bar / slash commands.
    Search,
    /// A popout overlay is focused — single-key actions apply to it.
    Overlay,
    /// Typing into the focused popout's field or body editor.
    Editing,
    /// Awaiting confirmation for a destructive action.
    Confirm(ConfirmAction),
}

/// Actions that can result from a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    SelectPrev,
    SelectNext,
    NavigateRight,
    NavigateLeft,
    SwitchModeNext,
    SwitchModePrev,
    /// Enter Search mode.
    EnterSearch,
    /// Return to Normal mode.
    ExitToNormal,
    /// Open/select the current item.
    OpenItem,
    /// Append a character to the search input.
    InsertChar(char),
    /// Delete the last character from the search input.
    Backspace,
    /// Clear the entire search input.
    ClearInput,
    /// Execute the current input as a slash command (Enter in Search mode).
    ExecuteCommand,
    // --- Context actions (Normal + Overlay) ---
    /// Reply to the selected/open email.
    Reply,
    /// Forward the selected/open email.
    Forward,
    /// Archive the selected/open email.
    Archive,
    /// Delete the selected item (email/contact/event, context-based).
    Delete,
    /// Mark the selected/open email as spam.
    Spam,
    /// Create something new — mail draft / contact / event, context-based.
    Create,
    // --- Overlay actions ---
    /// Send/save the focused popout (mail send, contact/event create).
    Send,
    /// Toggle popout N (0-based index; key `1` → 0, key `0` → 9).
    TogglePopout(usize),
    /// Minimize the focused popout (Esc — all minimized returns to the app).
    MinimizeOverlay,
    /// Discard/close the focused popout.
    CloseOverlay,
    /// Toggle maximize on the focused popout.
    ToggleMaximize,
    /// Cycle focus between active popouts.
    FocusNextPopout,
    /// Start editing the focused popout (first field / body).
    EditPopout,
    // --- Editing actions ---
    EditorChar(char),
    EditorBackspace,
    /// Enter: next field (on a field) or newline (in the body).
    EditorEnter,
    /// Tab: advance to the next field.
    EditorNextField,
    /// Esc: stop editing, back to Overlay.
    EditorEscape,
    // --- Confirm actions ---
    ConfirmYes,
    ConfirmNo,
    /// No-op (unhandled key).
    None,
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
        InputMode::Search => map_search(key),
        InputMode::Overlay => map_overlay(key),
        InputMode::Editing => map_editing(key),
        InputMode::Confirm(_) => map_confirm(key),
    }
}

/// Map a digit key to a 0-based popout index (`1`→0 … `9`→8, `0`→9).
fn popout_index(c: char) -> Option<usize> {
    match c {
        '1'..='9' => Some(c as usize - '1' as usize),
        '0' => Some(9),
        _ => None,
    }
}

/// Normal mode: navigation, context actions, popout toggles.
fn map_normal(key: KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return Action::None;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('k') | KeyCode::Up => Action::SelectPrev,
        KeyCode::Char('j') | KeyCode::Down => Action::SelectNext,
        KeyCode::Char('h') | KeyCode::Left => Action::NavigateLeft,
        KeyCode::Char('l') | KeyCode::Right => Action::NavigateRight,
        KeyCode::Enter => Action::OpenItem,
        KeyCode::Tab => Action::SwitchModeNext,
        KeyCode::BackTab => Action::SwitchModePrev,
        KeyCode::Char('/') | KeyCode::Char('i') => Action::EnterSearch,
        KeyCode::Char('c') => Action::Create,
        KeyCode::Char('r') => Action::Reply,
        KeyCode::Char('f') => Action::Forward,
        KeyCode::Char('a') => Action::Archive,
        KeyCode::Char('d') => Action::Delete,
        KeyCode::Char('s') => Action::Spam,
        KeyCode::Char(c) => popout_index(c)
            .map(Action::TogglePopout)
            .unwrap_or(Action::None),
        _ => Action::None,
    }
}

/// Search mode: type to filter, slash commands, Esc exits.
fn map_search(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::ExitToNormal,
        KeyCode::Up => Action::SelectPrev,
        KeyCode::Down => Action::SelectNext,
        KeyCode::Enter => Action::ExecuteCommand,
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::ClearInput,
        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            Action::InsertChar(c)
        }
        _ => Action::None,
    }
}

/// Overlay mode: single-key actions on the focused popout.
fn map_overlay(key: KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return Action::None;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('n') => Action::MinimizeOverlay,
        KeyCode::Char('x') | KeyCode::Char('q') => Action::CloseOverlay,
        KeyCode::Char('s') => Action::Send,
        KeyCode::Char('i') | KeyCode::Char('e') | KeyCode::Enter => Action::EditPopout,
        KeyCode::Char('r') => Action::Reply,
        KeyCode::Char('f') => Action::Forward,
        KeyCode::Char('a') => Action::Archive,
        KeyCode::Char('d') => Action::Delete,
        KeyCode::Char('m') => Action::ToggleMaximize,
        KeyCode::Tab => Action::FocusNextPopout,
        KeyCode::Char(c) => popout_index(c)
            .map(Action::TogglePopout)
            .unwrap_or(Action::None),
        _ => Action::None,
    }
}

/// Editing mode: typing into the focused popout.
fn map_editing(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::EditorEscape,
        KeyCode::Enter => Action::EditorEnter,
        KeyCode::Tab => Action::EditorNextField,
        KeyCode::Backspace => Action::EditorBackspace,
        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            Action::EditorChar(c)
        }
        _ => Action::None,
    }
}

/// Confirm mode: y confirms, n/Esc cancels, everything else is ignored.
fn map_confirm(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Action::ConfirmYes,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Action::ConfirmNo,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn number_keys_toggle_popouts_in_normal_and_overlay() {
        for mode in [InputMode::Normal, InputMode::Overlay] {
            assert_eq!(
                map_key(key(KeyCode::Char('1')), &mode),
                Action::TogglePopout(0)
            );
            assert_eq!(
                map_key(key(KeyCode::Char('9')), &mode),
                Action::TogglePopout(8)
            );
            assert_eq!(
                map_key(key(KeyCode::Char('0')), &mode),
                Action::TogglePopout(9)
            );
        }
    }

    #[test]
    fn number_keys_type_into_search() {
        assert_eq!(
            map_key(key(KeyCode::Char('1')), &InputMode::Search),
            Action::InsertChar('1')
        );
    }

    #[test]
    fn create_is_context_neutral() {
        assert_eq!(
            map_key(key(KeyCode::Char('c')), &InputMode::Normal),
            Action::Create
        );
    }

    #[test]
    fn overlay_esc_minimizes_x_closes() {
        assert_eq!(
            map_key(key(KeyCode::Esc), &InputMode::Overlay),
            Action::MinimizeOverlay
        );
        assert_eq!(
            map_key(key(KeyCode::Char('x')), &InputMode::Overlay),
            Action::CloseOverlay
        );
    }

    #[test]
    fn confirm_only_accepts_y_n_esc() {
        let mode = InputMode::Confirm(ConfirmAction::DeleteMail("id".into()));
        assert_eq!(map_key(key(KeyCode::Char('y')), &mode), Action::ConfirmYes);
        assert_eq!(map_key(key(KeyCode::Char('n')), &mode), Action::ConfirmNo);
        assert_eq!(map_key(key(KeyCode::Esc), &mode), Action::ConfirmNo);
        assert_eq!(map_key(key(KeyCode::Char('d')), &mode), Action::None);
    }
}
