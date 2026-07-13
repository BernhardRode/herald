//! Contacts screen: filterable list (left) and detail pane (right).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::tui::model::window::ListWindow;
use crate::tui::types::ContactEntry;

pub struct ContactsScreen {
    pub contacts: Vec<ContactEntry>,
    pub win: ListWindow,
    pub query: String,
}

impl ContactsScreen {
    pub fn new() -> Self {
        Self {
            contacts: Vec::new(),
            win: ListWindow::new(),
            query: String::new(),
        }
    }

    pub fn on_loaded(&mut self, contacts: Vec<ContactEntry>) {
        let selected_id = self.selected().map(|c| c.id.clone());
        self.contacts = contacts;
        if let Some(id) = selected_id {
            if let Some(pos) = self.filtered().iter().position(|c| c.id == id) {
                self.win.select(pos, self.filtered().len());
            }
        }
        self.win.clamp(self.filtered().len());
    }

    /// Case-insensitive substring filter over name/email/phone.
    pub fn filtered(&self) -> Vec<&ContactEntry> {
        if self.query.is_empty() {
            return self.contacts.iter().collect();
        }
        let q = self.query.to_lowercase();
        self.contacts
            .iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&q)
                    || c.email.to_lowercase().contains(&q)
                    || c.phone.contains(&q)
            })
            .collect()
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.win.reset();
    }

    pub fn selected(&self) -> Option<&ContactEntry> {
        self.filtered().get(self.win.selected()).copied()
    }

    pub fn select_next(&mut self) {
        let total = self.filtered().len();
        self.win.select_next(total);
    }

    pub fn select_prev(&mut self) {
        self.win.select_prev();
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        self.win.set_height(chunks[0].height.saturating_sub(2) as usize);
        let filtered = self.filtered();
        self.win.clamp(filtered.len());

        let border = if focused { Color::Cyan } else { Color::DarkGray };
        let visible = self.win.offset..(self.win.offset + self.win.height).min(filtered.len());
        let items: Vec<ListItem> = filtered[visible]
            .iter()
            .map(|c| {
                let label = if c.email.is_empty() {
                    c.name.clone()
                } else {
                    format!("{}  <{}>", c.name, c.email)
                };
                ListItem::new(Line::from(vec![
                    Span::styled("📇 ", Style::default().fg(Color::Cyan)),
                    Span::styled(label, Style::default().fg(Color::White)),
                ]))
            })
            .collect();

        let mut state = ListState::default().with_selected(Some(self.win.cursor));
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
                    .title(format!(" Contacts ({}) ", filtered.len())),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, chunks[0], &mut state);

        // Detail pane
        let detail: Vec<Line> = match self.selected() {
            Some(c) => vec![
                Line::from(format!("Name:  {}", c.name)),
                Line::from(format!("Email: {}", c.email)),
                Line::from(format!("Phone: {}", c.phone)),
            ],
            None => vec![Line::from(Span::styled(
                "no contact selected",
                Style::default().fg(Color::DarkGray),
            ))],
        };
        frame.render_widget(
            Paragraph::new(detail).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(" Detail "),
            ),
            chunks[1],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, name: &str, email: &str) -> ContactEntry {
        ContactEntry {
            id: id.into(),
            name: name.into(),
            email: email.into(),
            phone: String::new(),
        }
    }

    #[test]
    fn filter_matches_name_and_email() {
        let mut s = ContactsScreen::new();
        s.on_loaded(vec![
            c("1", "Alice Example", "alice@example.com"),
            c("2", "Bob Meyer", "bob@meyer.de"),
        ]);
        s.set_query("meyer");
        assert_eq!(s.filtered().len(), 1);
        assert_eq!(s.selected().unwrap().id, "2");
        s.set_query("ALICE");
        assert_eq!(s.filtered().len(), 1);
        s.set_query("");
        assert_eq!(s.filtered().len(), 2);
    }

    #[test]
    fn selection_survives_reload() {
        let mut s = ContactsScreen::new();
        s.on_loaded(vec![c("1", "A", ""), c("2", "B", ""), c("3", "C", "")]);
        s.select_next();
        s.select_next();
        assert_eq!(s.selected().unwrap().id, "3");
        s.on_loaded(vec![c("0", "Z", ""), c("1", "A", ""), c("3", "C", "")]);
        assert_eq!(s.selected().unwrap().id, "3");
    }

    #[test]
    fn selection_clamps_when_filter_shrinks() {
        let mut s = ContactsScreen::new();
        s.on_loaded(vec![c("1", "A", ""), c("2", "B", ""), c("3", "AB", "")]);
        s.select_next();
        s.select_next();
        s.set_query("A");
        s.win.clamp(s.filtered().len());
        assert!(s.selected().is_some());
    }
}
