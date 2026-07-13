//! Popup overlays: entries open as popups on top of the full-size main app.
//! A numbered popup bar lists every open popup; `1`–`9`,`0` toggle them.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::text::truncate_str;

use super::model::form::Form;

const MAX_POPUPS: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub enum PopupKind {
    /// Read an email (full body).
    MailView { email_id: String },
    /// Compose a new email.
    Compose,
    /// Reply to an email.
    Reply { email_id: String },
    /// Forward an email.
    Forward { email_id: String },
    /// Add (`id: None`) or edit a contact.
    ContactForm { contact_id: Option<String> },
    /// Add (`id: None`) or edit a calendar event.
    EventForm { event_id: Option<String> },
    /// Read a calendar event.
    EventView { event_id: String },
    /// Read a contact.
    ContactView,
    /// Key binding help.
    Help,
}

impl PopupKind {
    pub fn icon(&self) -> &'static str {
        match self {
            PopupKind::MailView { .. } => "✉",
            PopupKind::Compose | PopupKind::Reply { .. } | PopupKind::Forward { .. } => "✏",
            PopupKind::ContactForm { .. } | PopupKind::ContactView => "📇",
            PopupKind::EventForm { .. } | PopupKind::EventView { .. } => "📅",
            PopupKind::Help => "?",
        }
    }

    /// Editor popups hold a form and are submitted with `s`.
    pub fn is_editor(&self) -> bool {
        matches!(
            self,
            PopupKind::Compose
                | PopupKind::Reply { .. }
                | PopupKind::Forward { .. }
                | PopupKind::ContactForm { .. }
                | PopupKind::EventForm { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupState {
    Active,
    Minimized,
    Maximized,
}

#[derive(Debug, Clone)]
pub struct Popup {
    pub kind: PopupKind,
    pub title: String,
    pub state: PopupState,
    /// Editor form (editor kinds only).
    pub form: Option<Form>,
    /// Read-only body lines (view kinds only).
    pub body: Vec<Line<'static>>,
    /// Footer hint line.
    pub hint: &'static str,
}

impl Popup {
    pub fn view(kind: PopupKind, title: String, body: Vec<Line<'static>>) -> Self {
        let hint = match kind {
            PopupKind::MailView { .. } => "r reply · f fwd · a archive · d delete · Esc min · x close",
            PopupKind::EventView { .. } => "e edit · d delete · Esc min · x close",
            PopupKind::Help => "Esc min · x close",
            _ => "Esc min · x close",
        };
        Self {
            kind,
            title,
            state: PopupState::Active,
            form: None,
            body: Vec::new(),
            hint,
        }
        .with_body(body)
    }

    fn with_body(mut self, body: Vec<Line<'static>>) -> Self {
        self.body = body;
        self
    }

    pub fn editor(kind: PopupKind, title: String, form: Form) -> Self {
        Self {
            kind,
            title,
            state: PopupState::Active,
            form: Some(form),
            body: Vec::new(),
            hint: "Tab next field · s send/save · i edit · Esc min · x discard",
        }
    }
}

/// The popup stack.
#[derive(Default)]
pub struct Popups {
    pub items: Vec<Popup>,
    pub focused: Option<usize>,
}

impl Popups {
    pub fn open(&mut self, popup: Popup) {
        if self.items.len() >= MAX_POPUPS {
            self.items.remove(0);
        }
        self.items.push(popup);
        self.focused = Some(self.items.len() - 1);
    }

    pub fn focused_popup(&self) -> Option<&Popup> {
        self.focused.and_then(|i| self.items.get(i))
    }

    pub fn focused_popup_mut(&mut self) -> Option<&mut Popup> {
        self.focused.and_then(|i| self.items.get_mut(i))
    }

    /// Indices of non-minimized popups.
    pub fn active_indices(&self) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, p)| p.state != PopupState::Minimized)
            .map(|(i, _)| i)
            .collect()
    }

    pub fn has_active(&self) -> bool {
        !self.active_indices().is_empty()
    }

    /// Toggle popup N between active and minimized.
    pub fn toggle(&mut self, idx: usize) {
        let Some(p) = self.items.get_mut(idx) else {
            return;
        };
        match p.state {
            PopupState::Minimized => {
                p.state = PopupState::Active;
                self.focused = Some(idx);
            }
            _ => {
                p.state = PopupState::Minimized;
                self.refocus();
            }
        }
    }

    pub fn minimize_focused(&mut self) {
        if let Some(p) = self.focused_popup_mut() {
            p.state = PopupState::Minimized;
        }
        self.refocus();
    }

    /// Close the focused popup entirely. Returns the closed popup.
    pub fn close_focused(&mut self) -> Option<Popup> {
        let idx = self.focused?;
        if idx >= self.items.len() {
            return None;
        }
        let popup = self.items.remove(idx);
        self.refocus();
        Some(popup)
    }

    pub fn toggle_maximize(&mut self) {
        if let Some(p) = self.focused_popup_mut() {
            p.state = match p.state {
                PopupState::Maximized => PopupState::Active,
                _ => PopupState::Maximized,
            };
        }
    }

    pub fn focus_next(&mut self) {
        let active = self.active_indices();
        if active.is_empty() {
            return;
        }
        let next = match self.focused {
            Some(cur) => active
                .iter()
                .copied()
                .find(|&i| i > cur)
                .unwrap_or(active[0]),
            None => active[0],
        };
        self.focused = Some(next);
    }

    /// After a close/minimize, focus the last remaining active popup.
    fn refocus(&mut self) {
        self.focused = self.active_indices().last().copied();
    }

    /// Find an open popup by kind (avoid duplicates).
    pub fn find(&self, kind: &PopupKind) -> Option<usize> {
        self.items.iter().position(|p| &p.kind == kind)
    }

    // -- rendering ----------------------------------------------------------

    /// Draw active popups over the given area. Returns the cursor position
    /// if an editor form is being edited.
    pub fn render(&self, frame: &mut Frame, area: Rect, editing: bool) -> Option<(u16, u16)> {
        let active = self.active_indices();
        if active.is_empty() {
            return None;
        }

        // A maximized popup takes the whole area alone
        let maximized: Vec<usize> = active
            .iter()
            .copied()
            .filter(|&i| self.items[i].state == PopupState::Maximized)
            .collect();
        let (visible, areas) = if let Some(&m) = maximized.last() {
            (vec![m], vec![inset(area, 1, 0)])
        } else {
            let shown: Vec<usize> = active.iter().rev().take(2).rev().copied().collect();
            (shown.clone(), popup_areas(area, shown.len()))
        };

        let mut cursor = None;
        for (slot, &idx) in visible.iter().enumerate() {
            let popup = &self.items[idx];
            let rect = areas[slot];
            let is_focused = self.focused == Some(idx);
            self.draw_popup(frame, popup, idx, is_focused, rect);
            if is_focused && editing {
                if let Some(form) = &popup.form {
                    let (row, col) = form.cursor();
                    let label_w = form
                        .fields
                        .iter()
                        .map(|f| f.label.len())
                        .max()
                        .unwrap_or(0) as u16
                        + 2; // "Label: "
                    let x = if form.body_focused() {
                        rect.x + 1 + col
                    } else {
                        rect.x + 1 + label_w + col
                    };
                    let y = rect.y + 1 + row;
                    cursor = Some((
                        x.min(rect.right().saturating_sub(2)),
                        y.min(rect.bottom().saturating_sub(2)),
                    ));
                }
            }
        }
        cursor
    }

    fn draw_popup(&self, frame: &mut Frame, popup: &Popup, idx: usize, focused: bool, area: Rect) {
        let border = if focused { Color::Cyan } else { Color::DarkGray };
        let title = format!(" [{}] {} {} ", popup_key(idx), popup.kind.icon(), popup.title);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border).bg(Color::Black))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(if focused { Color::Cyan } else { Color::White })
                    .bg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ));

        let mut lines: Vec<Line<'static>> = if let Some(form) = &popup.form {
            render_form(form)
        } else {
            popup.body.clone()
        };
        // Footer hint pinned under the content
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            popup.hint,
            Style::default().fg(Color::DarkGray),
        )));

        let paragraph = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::White).bg(Color::Black));

        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }

    /// The popup bar: every open popup with its toggle key.
    pub fn render_bar(&self, frame: &mut Frame, area: Rect) {
        let mut spans: Vec<Span<'_>> = vec![Span::raw(" ")];
        for (idx, popup) in self.items.iter().enumerate() {
            let label = format!(
                " {} {} {} ",
                popup_key(idx),
                popup.kind.icon(),
                truncate_str(&popup.title, 18)
            );
            let style = match (popup.state, self.focused == Some(idx)) {
                (PopupState::Minimized, _) => Style::default().fg(Color::DarkGray),
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
}

/// Render a form: "Label: value" rows, separator, body.
fn render_form(form: &Form) -> Vec<Line<'static>> {
    let label_w = form.fields.iter().map(|f| f.label.len()).max().unwrap_or(0);
    let mut lines: Vec<Line<'static>> = form
        .fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let label_style = if i == form.focus {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(vec![
                Span::styled(format!("{:<w$}: ", f.label, w = label_w), label_style),
                Span::styled(f.value.clone(), Style::default().fg(Color::White)),
            ])
        })
        .collect();

    if let Some(body) = &form.body {
        lines.push(Line::from(Span::styled(
            "─".repeat(40),
            Style::default().fg(Color::DarkGray),
        )));
        for l in body.lines() {
            lines.push(Line::from(l.to_string()));
        }
        if body.ends_with('\n') || body.is_empty() {
            lines.push(Line::from(""));
        }
    }
    lines
}

/// The key that toggles popup `idx` (1–9, then 0).
pub fn popup_key(idx: usize) -> char {
    match idx {
        0..=8 => char::from(b'1' + idx as u8),
        _ => '0',
    }
}

/// Popup areas: one → centered ~88%, two → side by side.
fn popup_areas(area: Rect, count: usize) -> Vec<Rect> {
    let inner = inset(area, area.width / 16, area.height / 12);
    if count <= 1 {
        return vec![inner];
    }
    let half = inner.width / 2;
    vec![
        Rect::new(inner.x, inner.y, half.saturating_sub(1), inner.height),
        Rect::new(inner.x + half + 1, inner.y, half.saturating_sub(1), inner.height),
    ]
}

fn inset(area: Rect, dx: u16, dy: u16) -> Rect {
    Rect::new(
        area.x + dx,
        area.y + dy,
        area.width.saturating_sub(dx * 2),
        area.height.saturating_sub(dy * 2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(title: &str) -> Popup {
        Popup::view(
            PopupKind::MailView {
                email_id: title.to_string(),
            },
            title.to_string(),
            vec![],
        )
    }

    #[test]
    fn open_focuses_new_popup() {
        let mut p = Popups::default();
        p.open(view("a"));
        p.open(view("b"));
        assert_eq!(p.focused, Some(1));
        assert_eq!(p.active_indices(), vec![0, 1]);
    }

    #[test]
    fn toggle_minimizes_and_restores() {
        let mut p = Popups::default();
        p.open(view("a"));
        p.toggle(0);
        assert!(!p.has_active());
        p.toggle(0);
        assert!(p.has_active());
        assert_eq!(p.focused, Some(0));
    }

    #[test]
    fn close_refocuses_last_active() {
        let mut p = Popups::default();
        p.open(view("a"));
        p.open(view("b"));
        let closed = p.close_focused().unwrap();
        assert_eq!(closed.title, "b");
        assert_eq!(p.focused, Some(0));
    }

    #[test]
    fn minimize_all_leaves_no_focus() {
        let mut p = Popups::default();
        p.open(view("a"));
        p.minimize_focused();
        assert_eq!(p.focused, None);
        assert!(!p.has_active());
    }

    #[test]
    fn focus_next_cycles_active_only() {
        let mut p = Popups::default();
        p.open(view("a"));
        p.open(view("b"));
        p.open(view("c"));
        p.toggle(1); // minimize b
        p.focused = Some(0);
        p.focus_next();
        assert_eq!(p.focused, Some(2), "skips the minimized popup");
        p.focus_next();
        assert_eq!(p.focused, Some(0), "wraps around");
    }

    #[test]
    fn find_prevents_duplicate_views() {
        let mut p = Popups::default();
        p.open(view("a"));
        assert!(p
            .find(&PopupKind::MailView {
                email_id: "a".into()
            })
            .is_some());
        assert!(p
            .find(&PopupKind::MailView {
                email_id: "zzz".into()
            })
            .is_none());
    }

    #[test]
    fn capacity_is_bounded() {
        let mut p = Popups::default();
        for i in 0..15 {
            p.open(view(&i.to_string()));
        }
        assert_eq!(p.items.len(), MAX_POPUPS);
    }
}
