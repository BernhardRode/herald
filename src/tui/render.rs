//! Frame rendering: main panels, popout overlays, popout bar, status bar.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::text::truncate_str;

use super::editor;
use super::event::InputMode;
use super::popout::{Popout, PopoutManager, PopoutState};
use super::state::{App, Mode, Panel};
use super::ui::{input, layout::Layout, preview, results};

/// Draw one full frame.
pub fn draw_frame(frame: &mut Frame, app: &mut App) {
    let active = app.popouts.active_indices();
    let layout = Layout::build(
        frame.area(),
        active.len(),
        app.popouts.has_maximized(),
        !app.popouts.popouts.is_empty(),
    );
    app.results_height = layout.results.height.saturating_sub(2);

    // Main app (always drawn; overlays go on top)
    results::draw(frame, app, layout.results);
    input::draw(frame, app, layout.input);
    preview::draw(frame, app, layout.preview);

    // Popout overlays
    let overlay_indices: Vec<usize> = if app.popouts.has_maximized() {
        active
            .iter()
            .copied()
            .filter(|&i| app.popouts.popouts[i].state == PopoutState::Maximized)
            .take(1)
            .collect()
    } else {
        active
    };
    let mut focused_cursor: Option<(u16, u16)> = None;
    for (slot, &idx) in overlay_indices.iter().enumerate() {
        if let Some(area) = layout.popout_areas.get(slot) {
            let popout = &app.popouts.popouts[idx];
            let is_focused = app.popouts.focused == Some(idx);
            draw_popout(frame, popout, idx, is_focused, *area);
            if is_focused && app.input_mode == InputMode::Editing {
                let (col, row) = editor::cursor_position(popout);
                let x = (area.x + 1 + col).min(area.right().saturating_sub(2));
                let y = (area.y + 1 + row).min(area.bottom().saturating_sub(2));
                focused_cursor = Some((x, y));
            }
        }
    }
    if let Some(pos) = focused_cursor {
        frame.set_cursor_position(pos);
    }

    // Popout bar and status bar
    if let Some(bar_area) = layout.popout_bar {
        draw_popout_bar(frame, &app.popouts, bar_area);
    }
    draw_status_bar(frame, layout.status_bar, app);

    // Quit confirmation popup
    if app.show_quit_confirm {
        draw_quit_confirm(frame, frame.area());
    }
}

/// Draw a single popout overlay.
fn draw_popout(frame: &mut Frame, popout: &Popout, idx: usize, is_focused: bool, area: Rect) {
    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let title_style = if is_focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let number = popout_key(idx);
    let title = format!(" [{number}] {} {} ", popout.kind.icon(), popout.title);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(title, title_style));

    let paragraph = Paragraph::new(popout.body.clone())
        .block(block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));

    frame.render_widget(Clear, area);
    frame.render_widget(paragraph, area);
}

/// Draw the popout bar: every open popout with its toggle number.
fn draw_popout_bar(frame: &mut Frame, manager: &PopoutManager, area: Rect) {
    let mut spans: Vec<Span<'_>> = vec![Span::raw(" ")];
    for (idx, popout) in manager.popouts.iter().enumerate() {
        let key = popout_key(idx);
        let label = format!(
            " {key} {} {} ",
            popout.kind.icon(),
            truncate_str(&popout.title, 18)
        );
        let style = match (popout.state, manager.focused == Some(idx)) {
            (PopoutState::Minimized, _) => Style::default().fg(Color::DarkGray),
            (_, true) => Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            (_, false) => Style::default().fg(Color::Cyan),
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The key that toggles the popout at `idx` (1–9, then 0).
fn popout_key(idx: usize) -> char {
    match idx {
        0..=8 => char::from(b'1' + idx as u8),
        _ => '0',
    }
}

fn hint(spans: &mut Vec<Span<'static>>, key: &'static str, label: &'static str, color: Color) {
    spans.push(Span::styled(
        key,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(format!(" {label}  ")));
}

/// Draw the bottom status bar with mode tabs and context key hints.
fn draw_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    // Mode tabs indicator
    let mut spans: Vec<Span<'_>> = vec![Span::raw(" ")];
    for mode in Mode::all() {
        let style = if *mode == app.mode {
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(format!(" {} ", mode.label()), style));
    }
    spans.push(Span::raw("  "));

    match &app.input_mode {
        InputMode::Normal => {
            hint(&mut spans, "j/k", "nav", Color::Yellow);
            hint(&mut spans, "h/l", "drill", Color::Yellow);
            hint(&mut spans, "Enter", "open", Color::Yellow);
            hint(&mut spans, "/", "search", Color::Yellow);
            hint(&mut spans, "Tab", "view", Color::Yellow);
            match app.panel {
                Panel::Mails => {
                    hint(&mut spans, "c", "new", Color::Cyan);
                    hint(&mut spans, "r", "reply", Color::Cyan);
                    hint(&mut spans, "f", "fwd", Color::Cyan);
                    hint(&mut spans, "a", "arch", Color::Cyan);
                    hint(&mut spans, "d", "del", Color::Cyan);
                    hint(&mut spans, "s", "spam", Color::Cyan);
                }
                Panel::Contacts => {
                    hint(&mut spans, "c", "new contact", Color::Cyan);
                    hint(&mut spans, "d", "delete", Color::Cyan);
                }
                Panel::Calendar => {
                    hint(&mut spans, "c", "new event", Color::Cyan);
                    hint(&mut spans, "d", "delete", Color::Cyan);
                }
                _ => {}
            }
            if !app.popouts.popouts.is_empty() {
                hint(&mut spans, "1-0", "popouts", Color::Magenta);
            }
            hint(&mut spans, "q", "quit", Color::Yellow);
        }
        InputMode::Search => {
            hint(&mut spans, "Esc", "back", Color::Yellow);
            hint(&mut spans, "↑↓", "nav", Color::Yellow);
            hint(&mut spans, "Enter", "run", Color::Yellow);
            spans.push(Span::styled(
                "/compose /reply /archive /delete /add-contact /add-event",
                Style::default().fg(Color::DarkGray),
            ));
        }
        InputMode::Overlay => {
            let is_editor = app.popouts.focused_popout().is_some_and(Popout::is_editor);
            if is_editor {
                hint(&mut spans, "s", "send/save", Color::Green);
                hint(&mut spans, "i", "edit", Color::Yellow);
                hint(&mut spans, "x", "discard", Color::Red);
            } else {
                hint(&mut spans, "r", "reply", Color::Cyan);
                hint(&mut spans, "f", "fwd", Color::Cyan);
                hint(&mut spans, "a", "arch", Color::Cyan);
                hint(&mut spans, "d", "del", Color::Cyan);
                hint(&mut spans, "x", "close", Color::Red);
            }
            hint(&mut spans, "Esc", "minimize", Color::Yellow);
            hint(&mut spans, "m", "max", Color::Yellow);
            hint(&mut spans, "Tab", "focus", Color::Yellow);
            hint(&mut spans, "1-0", "popouts", Color::Magenta);
        }
        InputMode::Editing => {
            hint(&mut spans, "Tab/Enter", "next field", Color::Yellow);
            hint(&mut spans, "Esc", "done", Color::Yellow);
            spans.push(Span::raw("Type to edit…"));
        }
        InputMode::Confirm(action) => {
            spans.push(Span::styled(
                format!(" ⚠ {} ", action.prompt()),
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("  "));
            hint(&mut spans, "y", "confirm", Color::Green);
            hint(&mut spans, "n/Esc", "cancel", Color::Red);
        }
    }

    if let Some(msg) = &app.status_message {
        spans.push(Span::styled(
            format!("  {msg}"),
            Style::default().fg(Color::Yellow),
        ));
    }

    if app.loading {
        spans.push(Span::styled(
            "  ⏳ loading...",
            Style::default().fg(Color::Green),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Draw a centered quit confirmation popup.
fn draw_quit_confirm(frame: &mut Frame, area: Rect) {
    let popup_width = 40u16.min(area.width.saturating_sub(4));
    let popup_height = 5u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Quit Herald? ");

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Enter",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" quit    "),
            Span::styled("any key", Style::default().fg(Color::DarkGray)),
            Span::raw(" cancel"),
        ]),
    ];

    let paragraph = Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(Color::White));

    frame.render_widget(paragraph, popup_area);
}
