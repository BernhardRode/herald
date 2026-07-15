//! Mail screen: mail list (left), content preview (right). Search is fuzzy
//! (nucleo) over server-paginated pages; scrolling near the bottom lazy-loads
//! the next page. Navigation: List (focus) ← Folders (h) ← Account (h).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::model::folders::format_folder;
use crate::tui::model::window::ListWindow;
use crate::tui::search::{MatchedItem, Matcher};
use crate::tui::types::{FolderEntry, MailEntry};
use crate::tui::worker::MAIL_PAGE_SIZE;

/// Fetch the next page when the selection is this close to the end.
pub const LAZY_LOAD_THRESHOLD: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailFocus {
    List,
    Folders,
    Account,
    SearchResults,
}

pub struct MailScreen {
    pub focus: MailFocus,

    pub folders: Vec<FolderEntry>,
    pub folder_win: ListWindow,

    pub active_folder_id: Option<String>,
    pub active_folder_name: String,

    pub mails: Vec<MailEntry>,
    matcher: Matcher<String>,
    pub results: Vec<MatchedItem<String>>,
    pub matched_count: u32,
    pub win: ListWindow,

    pub query: String,
    pub all_folders: bool,
    pub exhausted: bool,
    pub loading_more: bool,
}

impl MailScreen {
    pub fn new() -> Self {
        Self {
            focus: MailFocus::List,
            folders: Vec::new(),
            folder_win: ListWindow::new(),
            active_folder_id: None,
            active_folder_name: "Inbox".to_string(),
            mails: Vec::new(),
            matcher: Matcher::new(),
            results: Vec::new(),
            matched_count: 0,
            win: ListWindow::new(),
            query: String::new(),
            all_folders: false,
            exhausted: false,
            loading_more: false,
        }
    }

    // -- data updates --------------------------------------------------------

    pub fn on_folders_loaded(&mut self, folders: Vec<FolderEntry>) {
        let selected_id = self.selected_folder().map(|f| f.id.clone());
        self.folders = folders;
        // keep the folder cursor on the same folder across reloads
        if let Some(id) = selected_id {
            if let Some(pos) = self.folders.iter().position(|f| f.id == id) {
                self.folder_win.select(pos, self.folders.len());
            }
        }
        self.folder_win.clamp(self.folders.len());
    }

    pub fn on_mails_loaded(&mut self, mails: Vec<MailEntry>, position: usize, all: bool) {
        self.loading_more = false;
        if position == 0 {
            self.exhausted = mails.len() < MAIL_PAGE_SIZE;
            self.mails = mails;
            self.all_folders = all;
            self.rebuild_matcher();
            self.win.clamp(self.matched_count as usize);
        } else {
            if mails.len() < MAIL_PAGE_SIZE {
                self.exhausted = true;
            }
            let existing: std::collections::HashSet<String> =
                self.mails.iter().map(|m| m.id.clone()).collect();
            let injector = self.matcher.injector();
            for mail in mails {
                if existing.contains(&mail.id) {
                    continue;
                }
                let display = self.format_mail(&mail);
                injector.push(mail.id.clone(), |_item, cols| {
                    cols[0] = display.as_str().into();
                });
                self.mails.push(mail);
            }
        }
    }

    fn rebuild_matcher(&mut self) {
        self.matcher = Matcher::new();
        let injector = self.matcher.injector();
        for mail in &self.mails {
            let display = self.format_mail(mail);
            injector.push(mail.id.clone(), |_item, cols| {
                cols[0] = display.as_str().into();
            });
        }
        self.matcher.find(&self.query);
        self.matcher.tick();
    }

    /// The string the fuzzy matcher sees. Deliberately excludes the unread
    /// marker: read state changes without a reload, and rebuilding the async
    /// matcher mid-scroll would blank the results and reset the cursor. The
    /// marker is drawn from live state in [`Self::row_icon`] instead.
    fn format_mail(&self, mail: &MailEntry) -> String {
        if self.all_folders {
            let folder = mail
                .folder_id
                .as_deref()
                .and_then(|id| self.folders.iter().find(|f| f.id == id))
                .map(|f| f.name.as_str())
                .unwrap_or("?");
            format!("[{}] {} — {}", folder, mail.from, mail.subject)
        } else {
            format!("{} — {}", mail.from, mail.subject)
        }
    }

    /// Row prefix reflecting the current read state: a dot for unread,
    /// blank for read.
    fn row_icon(&self, id: &str) -> &'static str {
        let unread = self
            .mails
            .iter()
            .find(|m| m.id == id)
            .is_some_and(|m| m.is_unread());
        if unread {
            "✉  ● "
        } else {
            "✉    "
        }
    }

    /// Flip an entry to read. The marker is rendered from live state, so no
    /// matcher rebuild is needed (rebuilding would reset the cursor while the
    /// async matcher catches up).
    pub fn mark_read(&mut self, id: &str) {
        if let Some(m) = self.mails.iter_mut().find(|m| m.id == id) {
            m.is_read = true;
        }
    }

    /// Tick the matcher and refresh the visible result window.
    pub fn tick(&mut self) {
        self.matcher.tick();
        self.matched_count = self.matcher.matched_item_count;
        // Skip the clamp while the async matcher reports 0 matches for a
        // non-empty list (transient state right after a rebuild) — clamping
        // then would yank the cursor back to the top of the list.
        if self.matched_count > 0 || self.mails.is_empty() {
            self.win.clamp(self.matched_count as usize);
        }
        self.results = self
            .matcher
            .results(self.win.height as u32, self.win.offset as u32);
    }

    /// Update the fuzzy pattern after the query changed. Returns true if the
    /// list scope flipped between single-folder and all-folder (needs reload).
    pub fn set_query(&mut self, query: &str) -> bool {
        self.query = query.to_string();
        self.win.reset();
        let want_all = !self.query.is_empty();
        let flipped = want_all != self.all_folders;
        self.matcher.find(&self.query);
        flipped
    }

    // -- selection -----------------------------------------------------------

    pub fn selected_folder(&self) -> Option<&FolderEntry> {
        self.folders.get(self.folder_win.selected())
    }

    pub fn selected_mail(&self) -> Option<&MailEntry> {
        let id = self.results.get(self.win.cursor)?.inner.as_str();
        self.mails.iter().find(|m| m.id == id)
    }

    /// Move down. Returns true when a lazy-load of the next page is needed.
    pub fn select_next(&mut self) -> bool {
        match self.focus {
            MailFocus::List => {
                self.win.select_next(self.matched_count as usize);
                self.results = self
                    .matcher
                    .results(self.win.height as u32, self.win.offset as u32);
                !self.exhausted
                    && !self.loading_more
                    && self
                        .win
                        .near_end(self.matched_count as usize, LAZY_LOAD_THRESHOLD)
            }
            MailFocus::SearchResults => {
                self.win.select_next(self.matched_count as usize);
                self.results = self
                    .matcher
                    .results(self.win.height as u32, self.win.offset as u32);
                !self.exhausted
                    && !self.loading_more
                    && self
                        .win
                        .near_end(self.matched_count as usize, LAZY_LOAD_THRESHOLD)
            }
            MailFocus::Folders => {
                self.folder_win.select_next(self.folders.len());
                false
            }
            MailFocus::Account => false, // Account has no list
        }
    }

    pub fn select_prev(&mut self) {
        match self.focus {
            MailFocus::List => {
                self.win.select_prev();
                self.results = self
                    .matcher
                    .results(self.win.height as u32, self.win.offset as u32);
            }
            MailFocus::SearchResults => {
                self.win.select_prev();
                self.results = self
                    .matcher
                    .results(self.win.height as u32, self.win.offset as u32);
            }
            MailFocus::Folders => {
                self.folder_win.select_prev();
            }
            MailFocus::Account => {} // Account has no list
        }
    }

    /// The inbox folder, resolved from action tag or role.
    pub fn inbox(&self) -> Option<&FolderEntry> {
        self.folders
            .iter()
            .find(|f| f.action_tag.as_deref() == Some("inbox"))
            .or_else(|| {
                self.folders
                    .iter()
                    .find(|f| f.role.as_deref() == Some("inbox"))
            })
    }

    /// Whether the list currently shows the inbox (default view counts).
    pub fn is_inbox_active(&self) -> bool {
        match &self.active_folder_id {
            None => true,
            Some(id) => self.inbox().is_some_and(|f| &f.id == id),
        }
    }

    // -- rendering -----------------------------------------------------------

    pub fn render(&mut self, frame: &mut Frame, area: Rect, focused_chrome: bool) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(33), Constraint::Percentage(67)])
            .split(area);

        let list_height = chunks[0].height.saturating_sub(2) as usize;
        self.win.set_height(list_height);
        self.folder_win.set_height(list_height);

        match self.focus {
            MailFocus::List => {
                self.render_list(frame, chunks[0], focused_chrome);
            }
            MailFocus::SearchResults => {
                self.render_search_results(frame, chunks[0], focused_chrome);
            }
            MailFocus::Folders => {
                self.render_folders(frame, chunks[0], focused_chrome);
            }
            MailFocus::Account => {
                self.render_account(frame, chunks[0], focused_chrome);
            }
        }
        self.render_preview(frame, chunks[1]);
    }

    fn render_account(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let border = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let lines = vec![
            Line::from(Span::styled("📌 Profile", Style::default().fg(Color::Cyan))),
            Line::from(""),
            Line::from("Current profile (press 'l' to return to mail)"),
        ];
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
                    .title(" Account "),
            ),
            area,
        );
    }

    fn render_search_results(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let border = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let title = format!(" Search Results ({}) ", self.matched_count);

        let items: Vec<ListItem> = self
            .results
            .iter()
            .map(|entry| {
                ListItem::new(highlighted_line(
                    self.row_icon(&entry.inner),
                    &entry.matched_string,
                    &entry.match_indices,
                ))
            })
            .collect();

        let mut state = ListState::default().with_selected(Some(self.win.cursor));
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
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

    fn render_folders(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let border = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let visible = self.folder_win.offset
            ..(self.folder_win.offset + self.folder_win.height).min(self.folders.len());
        let items: Vec<ListItem> = self.folders[visible.clone()]
            .iter()
            .map(|f| {
                ListItem::new(Line::from(vec![
                    Span::styled("📁 ", Style::default().fg(Color::Cyan)),
                    Span::styled(format_folder(f), Style::default().fg(Color::White)),
                ]))
            })
            .collect();

        let mut state = ListState::default().with_selected(Some(self.folder_win.cursor));
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
                    .title(" Folders (press 'l' to return) "),
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

    fn render_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let border = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let title = if self.all_folders {
            format!(" All Mail ({}) ", self.matched_count)
        } else {
            format!(" {} ({}) ", self.active_folder_name, self.matched_count)
        };

        let items: Vec<ListItem> = self
            .results
            .iter()
            .map(|entry| {
                ListItem::new(highlighted_line(
                    self.row_icon(&entry.inner),
                    &entry.matched_string,
                    &entry.match_indices,
                ))
            })
            .collect();

        let mut state = ListState::default().with_selected(Some(self.win.cursor));
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
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

    fn render_preview(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Preview ");

        let lines: Vec<Line> = match self.selected_mail() {
            Some(mail) => {
                let mut lines = vec![
                    Line::from(format!("From: {}", mail.from)),
                    Line::from(format!("Date: {}", mail.date)),
                    Line::from(format!("Subject: {}", mail.subject)),
                    Line::from(Span::styled(
                        "─".repeat(40),
                        Style::default().fg(Color::DarkGray),
                    )),
                ];
                for l in mail.preview.lines() {
                    lines.push(Line::from(l.to_string()));
                }
                lines
            }
            None => vec![Line::from(Span::styled(
                "no message selected",
                Style::default().fg(Color::DarkGray),
            ))],
        };

        frame.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
    }
}

/// A line with an icon prefix and fuzzy-match highlighting.
pub fn highlighted_line(icon: &str, text: &str, indices: &[u32]) -> Line<'static> {
    let icon_span = Span::styled(icon.to_string(), Style::default().fg(Color::Cyan));
    if indices.is_empty() {
        return Line::from(vec![
            icon_span,
            Span::styled(text.to_string(), Style::default().fg(Color::White)),
        ]);
    }
    let highlight = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let normal = Style::default().fg(Color::White);

    let mut spans = vec![icon_span];
    let mut run = String::new();
    let mut in_highlight = false;
    for (i, ch) in text.chars().enumerate() {
        let hl = indices.contains(&(i as u32));
        if hl != in_highlight && !run.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut run),
                if in_highlight { highlight } else { normal },
            ));
        }
        in_highlight = hl;
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(
            run,
            if in_highlight { highlight } else { normal },
        ));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: &str, from: &str, subject: &str) -> MailEntry {
        MailEntry {
            id: id.into(),
            subject: subject.into(),
            from: from.into(),
            date: "2026-07-13T12:00:00Z".into(),
            preview: "preview".into(),
            folder_id: None,
            is_read: false,
        }
    }

    fn loaded(screen: &mut MailScreen, count: usize, position: usize) {
        let mails = (0..count)
            .map(|i| {
                mail(
                    &format!("m{}", position + i),
                    "alice",
                    &format!("subj {}", position + i),
                )
            })
            .collect();
        screen.on_mails_loaded(mails, position, false);
        // matcher is async; tick until settled
        for _ in 0..100 {
            screen.tick();
            if screen.matched_count as usize == screen.mails.len() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn full_page_is_not_exhausted() {
        let mut s = MailScreen::new();
        loaded(&mut s, MAIL_PAGE_SIZE, 0);
        assert!(!s.exhausted);
        loaded(&mut s, 3, MAIL_PAGE_SIZE);
        assert!(s.exhausted);
        assert_eq!(s.mails.len(), MAIL_PAGE_SIZE + 3);
    }

    #[test]
    fn append_skips_duplicates() {
        let mut s = MailScreen::new();
        loaded(&mut s, 5, 0);
        let dup = vec![mail("m0", "alice", "subj 0"), mail("x", "bob", "new")];
        s.on_mails_loaded(dup, 5, false);
        assert_eq!(s.mails.len(), 6);
    }

    #[test]
    fn scrolling_near_end_requests_more() {
        let mut s = MailScreen::new();
        s.win.set_height(10);
        loaded(&mut s, MAIL_PAGE_SIZE, 0);
        let mut wants_more = false;
        for _ in 0..MAIL_PAGE_SIZE {
            wants_more = s.select_next() || wants_more;
        }
        assert!(wants_more, "hitting the bottom must request the next page");
        s.exhausted = true;
        assert!(!s.select_next(), "exhausted list never requests more");
    }

    #[test]
    fn query_flip_detects_scope_change() {
        let mut s = MailScreen::new();
        loaded(&mut s, 5, 0);
        assert!(s.set_query("a"), "entering a query flips to all-folder");
        s.all_folders = true;
        assert!(!s.set_query("ab"), "same scope while typing");
        assert!(s.set_query(""), "clearing flips back");
    }

    #[test]
    fn folder_selection_survives_reload() {
        let mut s = MailScreen::new();
        let f = |id: &str| FolderEntry {
            id: id.into(),
            name: id.into(),
            parent_id: None,
            role: None,
            total_emails: 0,
            unread_emails: 0,
            display_name: id.into(),
            depth: 0,
            action_tag: None,
        };
        s.on_folders_loaded(vec![f("a"), f("b"), f("c")]);
        s.folder_win.select(2, 3);
        // reload with an extra folder in front
        s.on_folders_loaded(vec![f("x"), f("a"), f("b"), f("c")]);
        assert_eq!(s.selected_folder().unwrap().id, "c");
    }

    #[test]
    fn inbox_detection_by_tag_then_role() {
        let mut s = MailScreen::new();
        assert!(s.is_inbox_active(), "no folder yet = default inbox view");
        let mut f = FolderEntry {
            id: "in".into(),
            name: "Inbox".into(),
            parent_id: None,
            role: Some("inbox".into()),
            total_emails: 0,
            unread_emails: 0,
            display_name: "Inbox".into(),
            depth: 0,
            action_tag: Some("inbox".into()),
        };
        s.on_folders_loaded(vec![f.clone()]);
        s.active_folder_id = Some("in".into());
        assert!(s.is_inbox_active());
        s.active_folder_id = Some("other".into());
        assert!(!s.is_inbox_active());
        f.action_tag = None;
        s.on_folders_loaded(vec![f]);
        s.active_folder_id = Some("in".into());
        assert!(s.is_inbox_active(), "role fallback");
    }
}
