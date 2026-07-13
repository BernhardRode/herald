//! TUI entry point and event loop.

use std::io;
use std::time::Duration;

use crossterm::event::Event;
use ratatui::DefaultTerminal;

use crate::config::Config;

use super::data;
use super::event::{self, InputMode};
use super::render;
use super::state::App;

/// Entry point: initialize terminal, run the event loop, restore terminal.
pub fn run(config: Config, profile_name: Option<&str>) -> io::Result<()> {
    // We're already inside a tokio runtime (from #[tokio::main]),
    // so use block_in_place to run the synchronous TUI loop without
    // conflicting with the outer runtime.
    tokio::task::block_in_place(|| {
        let mut terminal = ratatui::init();
        let result = run_loop(&mut terminal, config, profile_name);
        ratatui::restore();
        result
    })
}

/// Main event loop.
fn run_loop(
    terminal: &mut DefaultTerminal,
    config: Config,
    profile_name: Option<&str>,
) -> io::Result<()> {
    let handle = tokio::runtime::Handle::current();
    let mut app = App::new(config, profile_name);

    // Initial data load: connect to default profile and load inbox
    handle.block_on(data::load_data_for_panel(&mut app));
    app.refresh_matcher();

    let tick_rate = Duration::from_millis(50);

    loop {
        terminal.draw(|frame| render::draw_frame(frame, &mut app))?;
        set_cursor_shape(&app.input_mode);

        if let Some(Event::Key(key)) = event::poll_event(tick_rate)? {
            // Only handle key press events (ignore release/repeat on kitty-protocol terminals)
            if key.kind != crossterm::event::KeyEventKind::Press {
                continue;
            }
            let action = event::map_key(key, &app.input_mode);
            let panel_before = app.panel;
            app.handle_action(action);

            // Panel changed → load that panel's data
            if app.panel != panel_before {
                handle.block_on(data::load_data_for_panel(&mut app));
                app.refresh_matcher();
            }

            // Search mode changed (all-folder ↔ single-folder) → reload mail data
            if app.needs_search_reload {
                app.needs_search_reload = false;
                handle.block_on(data::load_search_mails(&mut app));
                app.refresh_matcher();
            }

            // Server-side actions queued → execute, then re-fetch
            if app.needs_reload {
                app.needs_reload = false;
                handle.block_on(async {
                    data::execute_pending(&mut app).await;
                    data::load_data_for_panel(&mut app).await;
                });
                app.refresh_matcher();
            }
        }

        app.tick();

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn set_cursor_shape(mode: &InputMode) {
    use crossterm::cursor::SetCursorStyle;
    use crossterm::execute;

    let style = match mode {
        InputMode::Editing | InputMode::Search => SetCursorStyle::BlinkingBar,
        _ => SetCursorStyle::BlinkingUnderScore,
    };
    let _ = execute!(std::io::stdout(), style);
}
