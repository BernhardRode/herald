//! Calendar screen: month grid (left) + day agenda (right).
//!
//! `h`/`l` move by day, `H`/`L` by month, `t` jumps to today, `j`/`k` move
//! through the selected day's agenda; Enter opens the event popup.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::tui::model::month::{month_grid, month_name, Date};
use crate::tui::model::window::ListWindow;
use crate::tui::types::EventEntry;

pub struct CalendarScreen {
    pub events: Vec<EventEntry>,
    pub selected: Date,
    pub win: ListWindow,
}

impl CalendarScreen {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            selected: Date::today(),
            win: ListWindow::new(),
        }
    }

    pub fn on_loaded(&mut self, mut events: Vec<EventEntry>) {
        events.sort_by(|a, b| a.start.cmp(&b.start));
        self.events = events;
        self.win.clamp(self.agenda().len());
    }

    /// Events on the selected day, in start order.
    pub fn agenda(&self) -> Vec<&EventEntry> {
        let iso = self.selected.iso();
        self.events
            .iter()
            .filter(|e| e.start_date() == iso)
            .collect()
    }

    pub fn day_has_events(&self, day: u32) -> bool {
        let date = Date {
            year: self.selected.year,
            month: self.selected.month,
            day,
        };
        let iso = date.iso();
        self.events.iter().any(|e| e.start_date() == iso)
    }

    pub fn selected_event(&self) -> Option<&EventEntry> {
        self.agenda().get(self.win.selected()).copied()
    }

    pub fn move_days(&mut self, delta: i64) {
        self.selected = self.selected.add_days(delta);
        self.win.reset();
    }

    pub fn move_months(&mut self, delta: i32) {
        self.selected = self.selected.add_months(delta);
        self.win.reset();
    }

    pub fn today(&mut self) {
        self.selected = Date::today();
        self.win.reset();
    }

    pub fn select_next(&mut self) {
        let total = self.agenda().len();
        self.win.select_next(total);
    }

    pub fn select_prev(&mut self) {
        self.win.select_prev();
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(35), Constraint::Min(20)])
            .split(area);

        self.render_grid(frame, chunks[0], focused);
        self.render_agenda(frame, chunks[1]);
    }

    fn render_grid(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let border = if focused { Color::Cyan } else { Color::DarkGray };
        let today = Date::today();

        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            "  Mo  Tu  We  Th  Fr  Sa  Su",
            Style::default().fg(Color::DarkGray),
        ))];

        for week in month_grid(self.selected.year, self.selected.month) {
            let mut spans: Vec<Span> = Vec::new();
            for day in week {
                match day {
                    None => spans.push(Span::raw("    ")),
                    Some(d) => {
                        let is_selected = d == self.selected.day;
                        let is_today = today.year == self.selected.year
                            && today.month == self.selected.month
                            && today.day == d;
                        let marker = if self.day_has_events(d) { "·" } else { " " };
                        let text = if is_selected {
                            format!("[{d:>2}]")
                        } else {
                            format!(" {d:>2}{marker}")
                        };
                        let mut style = Style::default().fg(Color::White);
                        if is_selected {
                            style = Style::default()
                                .fg(Color::Black)
                                .bg(Color::Cyan)
                                .add_modifier(Modifier::BOLD);
                        } else if is_today {
                            style = Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD);
                        } else if self.day_has_events(d) {
                            style = Style::default().fg(Color::Green);
                        }
                        spans.push(Span::styled(text, style));
                    }
                }
            }
            lines.push(Line::from(spans));
        }

        let title = format!(
            " {} {} ",
            month_name(self.selected.month),
            self.selected.year
        );
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
                    .title(title),
            ),
            area,
        );
    }

    fn render_agenda(&mut self, frame: &mut Frame, area: Rect) {
        self.win.set_height(area.height.saturating_sub(2) as usize);
        let agenda_len = self.agenda().len();
        self.win.clamp(agenda_len);
        let agenda = self.agenda();

        let visible = self.win.offset..(self.win.offset + self.win.height).min(agenda.len());
        let items: Vec<ListItem> = agenda[visible]
            .iter()
            .map(|e| {
                let time = if e.start_time().is_empty() {
                    "--:--".to_string()
                } else {
                    e.start_time().to_string()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{time}  "), Style::default().fg(Color::Yellow)),
                    Span::styled(e.title.clone(), Style::default().fg(Color::White)),
                    Span::styled(
                        format!("  ({})", e.duration),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            })
            .collect();

        let weekday = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            [self.selected.weekday() as usize];
        let title = format!(
            " {weekday} {} {} — {} event{} ",
            self.selected.day,
            month_name(self.selected.month),
            agenda.len(),
            if agenda.len() == 1 { "" } else { "s" }
        );

        let mut state = ListState::default().with_selected(Some(self.win.cursor));
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(title),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, start: &str, title: &str) -> EventEntry {
        EventEntry {
            id: id.into(),
            title: title.into(),
            start: start.into(),
            duration: "PT1H".into(),
            status: "confirmed".into(),
        }
    }

    fn on(date: Date) -> CalendarScreen {
        let mut s = CalendarScreen::new();
        s.selected = date;
        s
    }

    #[test]
    fn agenda_shows_only_selected_day_sorted() {
        let mut s = on(Date {
            year: 2026,
            month: 7,
            day: 13,
        });
        s.on_loaded(vec![
            ev("b", "2026-07-13T14:00:00", "Dentist"),
            ev("a", "2026-07-13T09:00:00", "Standup"),
            ev("c", "2026-07-14T09:00:00", "Other day"),
        ]);
        let agenda = s.agenda();
        assert_eq!(agenda.len(), 2);
        assert_eq!(agenda[0].title, "Standup");
        assert_eq!(agenda[1].title, "Dentist");
    }

    #[test]
    fn day_markers_and_navigation() {
        let mut s = on(Date {
            year: 2026,
            month: 7,
            day: 13,
        });
        s.on_loaded(vec![ev("a", "2026-07-16T10:00:00", "X")]);
        assert!(s.day_has_events(16));
        assert!(!s.day_has_events(15));
        s.move_days(3);
        assert_eq!(s.selected.day, 16);
        assert_eq!(s.selected_event().unwrap().title, "X");
        s.move_months(1);
        assert_eq!(s.selected.month, 8);
        assert!(s.agenda().is_empty());
    }

    #[test]
    fn agenda_selection_resets_on_day_change() {
        let mut s = on(Date {
            year: 2026,
            month: 7,
            day: 13,
        });
        s.on_loaded(vec![
            ev("a", "2026-07-13T09:00:00", "A"),
            ev("b", "2026-07-13T10:00:00", "B"),
        ]);
        s.select_next();
        assert_eq!(s.selected_event().unwrap().id, "b");
        s.move_days(1);
        assert_eq!(s.win.selected(), 0);
    }

    #[test]
    fn events_without_time_are_handled() {
        let mut s = on(Date {
            year: 2026,
            month: 7,
            day: 13,
        });
        s.on_loaded(vec![ev("a", "", "no start")]);
        assert!(s.agenda().is_empty(), "event without a date is not on any day");
    }
}
