//! Results list rendering with fuzzy match highlighting.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::tui::app::{App, Panel};

/// Draw the results list with highlighted matches.
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(" {} ", app.panel.title());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(title);

    let icon = match app.panel {
        Panel::Profiles => "👤 ",
        Panel::Folders => "📁 ",
        Panel::Mails => "✉  ",
        Panel::Contacts => "📇 ",
        Panel::Calendar => "📅 ",
    };

    let items: Vec<ListItem> = app
        .results
        .iter()
        .map(|entry| {
            let line = build_highlighted_line(icon, &entry.matched_string, &entry.match_indices);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

/// Build a line with an icon prefix and highlighted match characters.
fn build_highlighted_line(icon: &str, text: &str, indices: &[u32]) -> Line<'static> {
    let icon_span = Span::styled(icon.to_string(), Style::default().fg(Color::Cyan));

    if indices.is_empty() {
        return Line::from(vec![
            icon_span,
            Span::styled(text.to_string(), Style::default().fg(Color::White)),
        ]);
    }

    let highlight_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let normal_style = Style::default().fg(Color::White);

    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span<'static>> = vec![icon_span];
    let mut current_run = String::new();
    let mut is_highlight = false;

    for (i, ch) in chars.iter().enumerate() {
        let should_highlight = indices.contains(&(i as u32));

        if should_highlight != is_highlight {
            if !current_run.is_empty() {
                let style = if is_highlight {
                    highlight_style
                } else {
                    normal_style
                };
                spans.push(Span::styled(current_run.clone(), style));
                current_run.clear();
            }
            is_highlight = should_highlight;
        }
        current_run.push(*ch);
    }

    if !current_run.is_empty() {
        let style = if is_highlight {
            highlight_style
        } else {
            normal_style
        };
        spans.push(Span::styled(current_run, style));
    }

    Line::from(spans)
}
