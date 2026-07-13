//! Input bar rendering — search prompt with vi mode indicator, cursor, and counter.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::event::InputMode;

/// Draw the search input bar.
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let title = format!(" {} ", app.context_title());

    let border_color = match app.input_mode {
        InputMode::Insert => Color::Cyan,
        InputMode::Normal => Color::DarkGray,
        InputMode::EmailOpen => Color::Magenta,
        InputMode::Editing => Color::Green,
        InputMode::Confirm(_) => Color::Red,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split inner into: [mode badge] | [prompt + input] | [counter]
    let [mode_area, input_area, counter_area] = Layout::horizontal([
        Constraint::Length(8),
        Constraint::Min(1),
        Constraint::Length(14),
    ])
    .areas(inner);

    // Mode badge
    let (mode_text, mode_style) = match app.input_mode {
        InputMode::Normal => (
            " NOR ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        InputMode::Insert => (
            " INS ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        InputMode::EmailOpen => (
            " MAIL",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        InputMode::Editing => (
            " EDT ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        InputMode::Confirm(_) => (
            " CFM ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(mode_text, mode_style))),
        mode_area,
    );

    // Prompt + query text
    let prompt = Span::styled("❯ ", Style::default().fg(Color::Cyan));
    let query = Span::raw(&app.input);
    let input_line = Line::from(vec![prompt, query]);
    frame.render_widget(Paragraph::new(input_line), input_area);

    // Show cursor only in Insert mode
    if app.input_mode == InputMode::Insert {
        let cursor_x = input_area.x + 2 + app.input.len() as u16;
        let cursor_y = input_area.y;
        frame.set_cursor_position((cursor_x.min(input_area.right().saturating_sub(1)), cursor_y));
    }

    // Result counter: matched/total
    let counter_text = format!("{}/{}", app.matched_count, app.total_count);
    let mut counter_spans = vec![Span::styled(
        counter_text,
        Style::default().fg(Color::DarkGray),
    )];

    if app.matcher_running {
        counter_spans.push(Span::styled(
            " ●",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let counter =
        Paragraph::new(Line::from(counter_spans)).alignment(ratatui::layout::Alignment::Right);
    frame.render_widget(counter, counter_area);
}
