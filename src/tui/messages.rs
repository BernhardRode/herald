//! Message-passing core (after eilmeldung): every user intent is a
//! [`Command`], every fact/result is an [`Event`]. One unbounded channel
//! carries them; the app and the worker both see every message.

use crossterm::event::KeyEvent;
use jmap_base_client::JmapClient;

use crate::jmap::mail::FullEmail;

use super::types::{ContactEntry, EventEntry, FolderEntry, MailEntry};

pub enum Message {
    Command(Command),
    Event(Event),
}

/// User intents and data operations.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    // application
    Quit,
    Help,
    NoOp,

    // navigation
    SelectNext,
    SelectPrev,
    FocusLeft,
    FocusRight,
    ScreenNext,
    ScreenPrev,
    OpenItem,
    /// Esc in Normal mode — steps back towards the inbox.
    Escape,

    // search / input bar
    EnterSearch,
    InsertChar(char),
    Backspace,
    ClearInput,
    /// Enter in the input bar: commit the search.
    SubmitInput,
    /// Esc in the input bar: drop the filter, restore the previous view.
    CancelSearch,

    // context actions
    CreateItem,
    EditItem,
    DeleteItem,
    Reply,
    Forward,
    Archive,
    Spam,
    /// `s` on a focused editor popup: send mail / save contact / save event.
    Submit,

    // popups
    TogglePopup(usize),
    MinimizePopup,
    ClosePopup,
    ToggleMaximize,
    FocusNextPopup,
    EditPopup,

    // editor (typing into a popup form)
    EditorChar(char),
    EditorBackspace,
    EditorEnter,
    EditorNextField,
    EditorEscape,

    // confirmation
    ConfirmYes,
    ConfirmNo,

    // calendar navigation
    DayPrev,
    DayNext,
    MonthPrev,
    MonthNext,
    Today,

    // data operations (handled by the worker, all non-blocking)
    Connect,
    LoadFolders,
    LoadMailPage {
        folder_id: Option<String>,
        position: usize,
    },
    LoadMailBody(String),
    LoadContacts,
    LoadEvents,
    SendMail {
        to: String,
        cc: String,
        bcc: String,
        subject: String,
        body: String,
        sent_mailbox_id: Option<String>,
    },
    MoveMail {
        email_id: String,
        source_mailbox_id: String,
        target_mailbox_id: String,
        action: String,
    },
    /// Fire-and-forget `$seen` keyword update; not tracked as busy and does
    /// not reload on completion (the optimistic local update suffices).
    MarkMailRead(String),
    CreateContact {
        name: String,
        email: String,
        phone: String,
    },
    UpdateContact {
        id: String,
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
    UpdateEvent {
        id: String,
        title: String,
        start: String,
        duration: String,
    },
    DeleteEvent(String),
}

impl Command {
    /// Data commands are executed by the worker (and tracked as busy by the
    /// app); everything else is UI-local.
    pub fn is_data(&self) -> bool {
        matches!(
            self,
            Command::Connect
                | Command::LoadFolders
                | Command::LoadMailPage { .. }
                | Command::LoadMailBody(_)
                | Command::LoadContacts
                | Command::LoadEvents
                | Command::SendMail { .. }
                | Command::MoveMail { .. }
                | Command::CreateContact { .. }
                | Command::UpdateContact { .. }
                | Command::DeleteContact(_)
                | Command::CreateEvent { .. }
                | Command::UpdateEvent { .. }
                | Command::DeleteEvent(_)
        )
    }
}

/// Facts: raw input, timer ticks, and async operation results.
pub enum Event {
    Key(KeyEvent),
    Tick,
    Resized,

    ClientReady(Box<JmapClient>),
    ConnectFailed(String),

    FoldersLoaded(Vec<FolderEntry>),
    MailPageLoaded {
        mails: Vec<MailEntry>,
        position: usize,
        all_folders: bool,
    },
    MailBodyLoaded(Box<FullEmail>),
    ContactsLoaded(Vec<ContactEntry>),
    EventsLoaded(Vec<EventEntry>),

    /// A server push (RFC 8620 §7.3) reported changed state; flags say which
    /// screens are stale. `new_mail` is set when the server distinguishes a
    /// fresh delivery (the `EmailDelivery` pseudo-type).
    RemoteChanged {
        mail: bool,
        contacts: bool,
        calendar: bool,
        new_mail: bool,
    },

    /// A mutating operation succeeded; the message is shown as a tooltip and
    /// the current screen's data is reloaded.
    ActionCompleted(String),
    /// Any async operation failed.
    ActionFailed(String),
}
