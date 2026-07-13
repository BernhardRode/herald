//! Preview pane — displays content for the currently selected item.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::state::App;

/// Draw the preview pane on the right side.
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let (title, content) = if let Some(preview) = &app.preview_content {
        (format!(" {} ", preview.title), preview.body.clone())
    } else {
        (
            " Preview ".to_string(),
            vec![Line::from(Span::styled(
                "No item selected",
                Style::default().fg(Color::DarkGray),
            ))],
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(title);

    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Gray));

    frame.render_widget(paragraph, area);
}

/// Content to display in the preview pane.
#[derive(Debug, Clone)]
pub struct PreviewContent {
    /// Title shown in the preview border.
    pub title: String,
    /// Body lines to display.
    pub body: Vec<Line<'static>>,
}
