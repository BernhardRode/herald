//! Pure key → command mapping, routed by the current input mode.
//!
//! Routing priority (computed by the app): QuitDialog > Confirm > Editing >
//! Popup > Search > Normal(screen).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::messages::Command;

/// The three top-level screens, cycled with Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Mail,
    Contacts,
    Calendar,
}

impl Screen {
    pub const fn all() -> &'static [Screen] {
        &[Screen::Mail, Screen::Contacts, Screen::Calendar]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Screen::Mail => "Mail",
            Screen::Contacts => "Contacts",
            Screen::Calendar => "Calendar",
        }
    }

    pub const fn next(self) -> Screen {
        match self {
            Screen::Mail => Screen::Contacts,
            Screen::Contacts => Screen::Calendar,
            Screen::Calendar => Screen::Mail,
        }
    }

    pub const fn prev(self) -> Screen {
        match self {
            Screen::Mail => Screen::Calendar,
            Screen::Contacts => Screen::Mail,
            Screen::Calendar => Screen::Contacts,
        }
    }
}

/// Which input context receives keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    Normal(Screen),
    /// The search/input bar is open.
    Search,
    /// A popup is focused (not editing).
    Popup,
    /// Typing into a popup form.
    Editing,
    /// y/n confirmation for a destructive action.
    Confirm,
    /// The quit dialog: only Enter confirms.
    QuitDialog,
}

/// Map a digit to a popup index (`1`→0 … `9`→8, `0`→9).
fn popup_index(c: char) -> Option<usize> {
    match c {
        '1'..='9' => Some(c as usize - '1' as usize),
        '0' => Some(9),
        _ => None,
    }
}

pub fn map_key(key: KeyEvent, mode: KeyMode) -> Command {
    // Ctrl+C always quits
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Command::Quit;
    }

    match mode {
        KeyMode::Normal(screen) => map_normal(key, screen),
        KeyMode::Search => map_search(key),
        KeyMode::Popup => map_popup(key),
        KeyMode::Editing => map_editing(key),
        KeyMode::Confirm => map_confirm(key),
        KeyMode::QuitDialog => match key.code {
            KeyCode::Enter => Command::ConfirmYes,
            _ => Command::ConfirmNo,
        },
    }
}

fn map_normal(key: KeyEvent, screen: Screen) -> Command {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return Command::NoOp;
    }
    // Screen-specific bindings first
    if screen == Screen::Calendar {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => return Command::DayPrev,
            KeyCode::Char('l') | KeyCode::Right => return Command::DayNext,
            KeyCode::Char('H') => return Command::MonthPrev,
            KeyCode::Char('L') => return Command::MonthNext,
            KeyCode::Char('t') => return Command::Today,
            _ => {}
        }
    }
    match key.code {
        KeyCode::Char('q') => Command::Quit,
        KeyCode::Esc => Command::Escape,
        KeyCode::Char('?') => Command::Help,
        KeyCode::Char('k') | KeyCode::Up => Command::SelectPrev,
        KeyCode::Char('j') | KeyCode::Down => Command::SelectNext,
        KeyCode::Char('h') | KeyCode::Left => Command::FocusLeft,
        KeyCode::Char('l') | KeyCode::Right => Command::FocusRight,
        KeyCode::Enter => Command::OpenItem,
        KeyCode::Tab => Command::ScreenNext,
        KeyCode::BackTab => Command::ScreenPrev,
        KeyCode::Char('/') | KeyCode::Char('i') => Command::EnterSearch,
        KeyCode::Char('c') => Command::CreateItem,
        KeyCode::Char('e') => Command::EditItem,
        KeyCode::Char('d') => Command::DeleteItem,
        KeyCode::Char('r') => Command::Reply,
        KeyCode::Char('f') => Command::Forward,
        KeyCode::Char('a') => Command::Archive,
        KeyCode::Char('s') => Command::Spam,
        KeyCode::Char(c) => popup_index(c)
            .map(Command::TogglePopup)
            .unwrap_or(Command::NoOp),
        _ => Command::NoOp,
    }
}

fn map_search(key: KeyEvent) -> Command {
    match key.code {
        KeyCode::Esc => Command::CancelSearch,
        KeyCode::Enter => Command::SubmitInput,
        KeyCode::Up => Command::SelectPrev,
        KeyCode::Down => Command::SelectNext,
        KeyCode::Backspace => Command::Backspace,
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Command::ClearInput,
        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            Command::InsertChar(c)
        }
        _ => Command::NoOp,
    }
}

fn map_popup(key: KeyEvent) -> Command {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return Command::NoOp;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('n') => Command::MinimizePopup,
        KeyCode::Char('x') | KeyCode::Char('q') => Command::ClosePopup,
        KeyCode::Char('s') => Command::Submit,
        KeyCode::Char('i') | KeyCode::Char('e') | KeyCode::Enter => Command::EditPopup,
        KeyCode::Char('r') => Command::Reply,
        KeyCode::Char('f') => Command::Forward,
        KeyCode::Char('a') => Command::Archive,
        KeyCode::Char('d') => Command::DeleteItem,
        KeyCode::Char('m') => Command::ToggleMaximize,
        KeyCode::Tab => Command::FocusNextPopup,
        KeyCode::Char(c) => popup_index(c)
            .map(Command::TogglePopup)
            .unwrap_or(Command::NoOp),
        _ => Command::NoOp,
    }
}

fn map_editing(key: KeyEvent) -> Command {
    match key.code {
        KeyCode::Esc => Command::EditorEscape,
        KeyCode::Enter => Command::EditorEnter,
        KeyCode::Tab => Command::EditorNextField,
        KeyCode::Backspace => Command::EditorBackspace,
        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            Command::EditorChar(c)
        }
        _ => Command::NoOp,
    }
}

fn map_confirm(key: KeyEvent) -> Command {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Command::ConfirmYes,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Command::ConfirmNo,
        _ => Command::NoOp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn quit_dialog_only_enter_confirms() {
        assert_eq!(
            map_key(key(KeyCode::Enter), KeyMode::QuitDialog),
            Command::ConfirmYes
        );
        for k in [KeyCode::Char('l'), KeyCode::Right, KeyCode::Char('y')] {
            assert_eq!(map_key(key(k), KeyMode::QuitDialog), Command::ConfirmNo);
        }
    }

    #[test]
    fn esc_maps_per_mode() {
        let normal = KeyMode::Normal(Screen::Mail);
        assert_eq!(map_key(key(KeyCode::Esc), normal), Command::Escape);
        assert_eq!(
            map_key(key(KeyCode::Esc), KeyMode::Search),
            Command::CancelSearch
        );
        assert_eq!(
            map_key(key(KeyCode::Esc), KeyMode::Popup),
            Command::MinimizePopup
        );
        assert_eq!(
            map_key(key(KeyCode::Esc), KeyMode::Editing),
            Command::EditorEscape
        );
        assert_eq!(
            map_key(key(KeyCode::Esc), KeyMode::Confirm),
            Command::ConfirmNo
        );
    }

    #[test]
    fn calendar_gets_day_month_navigation() {
        let cal = KeyMode::Normal(Screen::Calendar);
        assert_eq!(map_key(key(KeyCode::Char('h')), cal), Command::DayPrev);
        assert_eq!(map_key(key(KeyCode::Char('l')), cal), Command::DayNext);
        assert_eq!(map_key(key(KeyCode::Char('H')), cal), Command::MonthPrev);
        assert_eq!(map_key(key(KeyCode::Char('L')), cal), Command::MonthNext);
        assert_eq!(map_key(key(KeyCode::Char('t')), cal), Command::Today);
        // Mail screen keeps h/l for panel focus
        let mail = KeyMode::Normal(Screen::Mail);
        assert_eq!(map_key(key(KeyCode::Char('h')), mail), Command::FocusLeft);
    }

    #[test]
    fn digits_toggle_popups_in_normal_and_popup_modes() {
        for mode in [KeyMode::Normal(Screen::Mail), KeyMode::Popup] {
            assert_eq!(
                map_key(key(KeyCode::Char('1')), mode),
                Command::TogglePopup(0)
            );
            assert_eq!(
                map_key(key(KeyCode::Char('0')), mode),
                Command::TogglePopup(9)
            );
        }
        // …but type into the search bar
        assert_eq!(
            map_key(key(KeyCode::Char('1')), KeyMode::Search),
            Command::InsertChar('1')
        );
    }

    #[test]
    fn search_enter_submits() {
        assert_eq!(
            map_key(key(KeyCode::Enter), KeyMode::Search),
            Command::SubmitInput
        );
    }

    #[test]
    fn popup_submit_and_close() {
        assert_eq!(
            map_key(key(KeyCode::Char('s')), KeyMode::Popup),
            Command::Submit
        );
        assert_eq!(
            map_key(key(KeyCode::Char('x')), KeyMode::Popup),
            Command::ClosePopup
        );
    }

    #[test]
    fn ctrl_c_always_quits() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        for mode in [
            KeyMode::Normal(Screen::Mail),
            KeyMode::Search,
            KeyMode::Popup,
            KeyMode::Editing,
            KeyMode::Confirm,
            KeyMode::QuitDialog,
        ] {
            assert_eq!(map_key(ctrl_c, mode), Command::Quit);
        }
    }

    #[test]
    fn screen_cycle() {
        assert_eq!(Screen::Mail.next(), Screen::Contacts);
        assert_eq!(Screen::Calendar.next(), Screen::Mail);
        assert_eq!(Screen::Mail.prev(), Screen::Calendar);
    }
}
