//! Terminal User Interface for Herald.
//!
//! Architecture (after the eilmeldung TUI reader): one unbounded message
//! channel carries [`messages::Command`]s (user intents / data operations)
//! and [`messages::Event`]s (input, ticks, async results). A blocking input
//! reader task feeds key events in; the [`worker`] spawns a tokio task per
//! JMAP operation, so the render loop never blocks on the network.
//!
//! Module map:
//! - [`messages`] — Command/Event enums, the message channel vocabulary
//! - [`keymap`] — pure key → command mapping per input mode
//! - [`app`] — state, routing, Esc ladder, frame composition
//! - [`worker`] — non-blocking JMAP operations
//! - [`model`] — pure logic: folders, forms, list windows, month math
//! - [`screens`] — Mail, Contacts, Calendar panels
//! - [`overlay`] — popup stack (entries open as popups) + popup bar
//! - [`statusbar`] — tabs, key hints, tooltips, spinner
//! - [`search`] — nucleo fuzzy matcher wrapper
//!
//! See `docs/tui-spec.md` for the full specification and UI mocks.
//!
//! # Security: Terminal output sanitization
//!
//! All server-supplied content (email subjects, sender names, body text,
//! contact names, calendar titles) is rendered exclusively through ratatui
//! widgets (`Paragraph`, `List`, `Line`, `Span`). Ratatui's cell-based
//! rendering via crossterm does not pass raw strings to the terminal — each
//! character is written into a screen buffer cell, preventing ANSI escape
//! injection.
//!
//! There are **no** raw `println!`/`eprintln!` writes of server-controlled
//! data in this module. If raw terminal output paths are added (e.g. debug
//! prints after `ratatui::restore()`), they MUST go through
//! `crate::text::sanitize_display()`.

mod app;
mod keymap;
mod messages;
mod model;
mod overlay;
mod screens;
mod search;
mod statusbar;
mod types;
mod worker;

use std::io;
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::config::Config;

use app::App;
use keymap::KeyMode;
use messages::{Command, Event, Message};
use worker::Worker;

/// Render/tick interval (spinner, fuzzy matcher progress).
const TICK_MILLIS: u64 = 80;

/// Launch the TUI with the given config and optional profile name.
pub fn run(config: Config, profile_name: Option<&str>) -> io::Result<()> {
    let profile_name = profile_name
        .map(str::to_string)
        .or_else(|| config.default_profile.clone())
        .or_else(|| config.profiles.keys().next().cloned())
        .unwrap_or_default();

    // We're already inside a tokio runtime (#[tokio::main]); block_in_place
    // lets us own the thread for the interactive loop.
    tokio::task::block_in_place(|| {
        let handle = tokio::runtime::Handle::current();
        let terminal = ratatui::init();
        let result = handle.block_on(run_loop(terminal, config, profile_name));
        ratatui::restore();
        let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show);
        result
    })
}

async fn run_loop(
    mut terminal: ratatui::DefaultTerminal,
    config: Config,
    profile_name: String,
) -> io::Result<()> {
    let (tx, mut rx) = unbounded_channel::<Message>();

    // Blocking input reader feeding the channel
    let input_tx = tx.clone();
    tokio::task::spawn_blocking(move || input_reader(input_tx));

    let profile = config.profiles.get(&profile_name).cloned();
    let mut app = App::new(&config, &profile_name, tx.clone());
    let mut worker = Worker::new(
        tx.clone(),
        profile.unwrap_or_else(|| {
            // App::new already tolerates a missing profile; the worker will
            // fail Connect with a clear error message.
            crate::config::Profile {
                server_url: String::new(),
                auth: crate::config::AuthMethod::OAuthBrowser {
                    client_id: "herald".into(),
                },
                from_email: None,
                from_name: None,
                folders: Default::default(),
                compose_format: None,
                signature: None,
                allow_insecure: false,
                confirm_actions: true,
            }
        }),
        profile_name,
    );

    let _ = tx.send(Message::Command(Command::Connect));

    let mut tick = tokio::time::interval(Duration::from_millis(TICK_MILLIS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    while app.running {
        tokio::select! {
            _ = tick.tick() => {
                app.process(&Message::Event(Event::Tick));
                draw(&mut terminal, &mut app)?;
            }
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                app.process(&msg);
                worker.handle(&msg);
                // Drain whatever queued up before redrawing once
                while let Ok(next) = rx.try_recv() {
                    app.process(&next);
                    worker.handle(&next);
                }
                draw(&mut terminal, &mut app)?;
            }
        }
    }

    Ok(())
}

fn draw(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    terminal.draw(|frame| app.draw(frame))?;
    set_cursor_shape(app.key_mode());
    Ok(())
}

fn set_cursor_shape(mode: KeyMode) {
    use crossterm::cursor::{Hide, SetCursorStyle, Show};
    use crossterm::execute;

    match mode {
        KeyMode::Editing | KeyMode::Search => {
            let _ = execute!(std::io::stdout(), Show, SetCursorStyle::BlinkingBar);
        }
        _ => {
            let _ = execute!(std::io::stdout(), Hide);
        }
    }
}

/// Blocking crossterm event loop; exits when the channel closes.
fn input_reader(tx: UnboundedSender<Message>) {
    loop {
        match crossterm::event::poll(Duration::from_millis(100)) {
            Ok(true) => match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(key)) => {
                    if tx.send(Message::Event(Event::Key(key))).is_err() {
                        return;
                    }
                }
                Ok(crossterm::event::Event::Resize(..)) => {
                    if tx.send(Message::Event(Event::Resized)).is_err() {
                        return;
                    }
                }
                _ => {}
            },
            Ok(false) => {
                if tx.is_closed() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}
