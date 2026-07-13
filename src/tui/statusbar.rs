//! Bottom chrome: screen tabs, context-sensitive key hints, tooltip messages
//! with severity flavors, and an async spinner.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::keymap::{KeyMode, Screen};
use super::screens::mail::MailFocus;

/// Tooltip severity (colors the message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Tooltip {
    pub text: String,
    pub flavor: Flavor,
}

impl Tooltip {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            flavor: Flavor::Info,
        }
    }

    pub fn warn(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            flavor: Flavor::Warning,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            flavor: Flavor::Error,
        }
    }
}

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn hint(spans: &mut Vec<Span<'static>>, key: &'static str, label: &'static str, color: Color) {
    spans.push(Span::styled(
        key,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(format!(" {label}  ")));
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    area: Rect,
    screen: Screen,
    mode: KeyMode,
    mail_focus: MailFocus,
    tooltip: Option<&Tooltip>,
    busy: bool,
    spinner_step: usize,
    has_popups: bool,
) {
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    for s in Screen::all() {
        let style = if *s == screen {
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(format!(" {} ", s.label()), style));
    }
    spans.push(Span::raw("  "));

    match mode {
        KeyMode::Normal(Screen::Mail) => {
            hint(&mut spans, "j/k", "nav", Color::Yellow);
            hint(&mut spans, "h/l", "panel", Color::Yellow);
            hint(&mut spans, "Enter", "open", Color::Yellow);
            hint(&mut spans, "/", "search", Color::Yellow);
            if mail_focus == MailFocus::List {
                hint(&mut spans, "c", "new", Color::Cyan);
                hint(&mut spans, "r", "reply", Color::Cyan);
                hint(&mut spans, "f", "fwd", Color::Cyan);
                hint(&mut spans, "a", "arch", Color::Cyan);
                hint(&mut spans, "d", "del", Color::Cyan);
                hint(&mut spans, "s", "spam", Color::Cyan);
            }
            hint(&mut spans, "Tab", "screen", Color::Yellow);
            hint(&mut spans, "Esc", "back", Color::Yellow);
            hint(&mut spans, "q", "quit", Color::Yellow);
        }
        KeyMode::Normal(Screen::Contacts) => {
            hint(&mut spans, "j/k", "nav", Color::Yellow);
            hint(&mut spans, "Enter", "open", Color::Yellow);
            hint(&mut spans, "/", "filter", Color::Yellow);
            hint(&mut spans, "c", "add", Color::Cyan);
            hint(&mut spans, "e", "edit", Color::Cyan);
            hint(&mut spans, "d", "delete", Color::Cyan);
            hint(&mut spans, "Tab", "screen", Color::Yellow);
            hint(&mut spans, "Esc", "back", Color::Yellow);
        }
        KeyMode::Normal(Screen::Calendar) => {
            hint(&mut spans, "h/l", "day", Color::Yellow);
            hint(&mut spans, "H/L", "month", Color::Yellow);
            hint(&mut spans, "t", "today", Color::Yellow);
            hint(&mut spans, "j/k", "agenda", Color::Yellow);
            hint(&mut spans, "Enter", "open", Color::Yellow);
            hint(&mut spans, "c", "add", Color::Cyan);
            hint(&mut spans, "e", "edit", Color::Cyan);
            hint(&mut spans, "d", "delete", Color::Cyan);
            hint(&mut spans, "Esc", "back", Color::Yellow);
        }
        KeyMode::Search => {
            hint(&mut spans, "Esc", "cancel", Color::Yellow);
            hint(&mut spans, "↑↓", "nav", Color::Yellow);
            hint(&mut spans, "Enter", "select", Color::Yellow);
        }
        KeyMode::Popup => {
            hint(&mut spans, "Esc", "minimize", Color::Yellow);
            hint(&mut spans, "x", "close", Color::Red);
            hint(&mut spans, "i", "edit", Color::Yellow);
            hint(&mut spans, "s", "send/save", Color::Green);
            hint(&mut spans, "m", "max", Color::Yellow);
            hint(&mut spans, "Tab", "focus", Color::Yellow);
        }
        KeyMode::Editing => {
            hint(&mut spans, "Tab/Enter", "next field", Color::Yellow);
            hint(&mut spans, "Esc", "done", Color::Yellow);
            spans.push(Span::styled(
                "type to edit…",
                Style::default().fg(Color::DarkGray),
            ));
        }
        KeyMode::Confirm => {
            hint(&mut spans, "y", "confirm", Color::Green);
            hint(&mut spans, "n/Esc", "cancel", Color::Red);
        }
        KeyMode::QuitDialog => {
            hint(&mut spans, "Enter", "quit", Color::Red);
            hint(&mut spans, "any key", "stay", Color::Green);
        }
    }

    if has_popups && matches!(mode, KeyMode::Normal(_) | KeyMode::Popup) {
        hint(&mut spans, "1-0", "popups", Color::Magenta);
    }

    if let Some(t) = tooltip {
        let color = match t.flavor {
            Flavor::Info => Color::Green,
            Flavor::Warning => Color::Yellow,
            Flavor::Error => Color::Red,
        };
        spans.push(Span::styled(
            format!("  {}", t.text),
            Style::default().fg(color),
        ));
    }

    if busy {
        spans.push(Span::styled(
            format!("  {}", SPINNER[spinner_step % SPINNER.len()]),
            Style::default().fg(Color::Cyan),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
