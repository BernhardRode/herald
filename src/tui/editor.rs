//! Text editing for popout fields and body, plus body rendering and cursor math.
//!
//! Every popout renders its editable state through [`refresh_body`]: one line
//! per form field (fixed label column), then — for mail editors — a separator
//! and the body buffer. Because the layout is uniform, the cursor position is
//! a direct function of the active field index or the buffer offset; there is
//! no per-kind row arithmetic to get wrong.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::popout::Popout;

/// Width of the rendered "▶ Label:  " prefix before a field value.
pub const FIELD_PREFIX_WIDTH: u16 = 12;

/// Number of chrome lines between the last field and the body text
/// (blank + separator + blank).
const BODY_GAP_LINES: usize = 3;

/// Insert a character at the cursor (active field or body).
pub fn insert_char(p: &mut Popout, c: char) {
    match p.active_field {
        Some(i) => {
            if let Some(field) = p.fields.get_mut(i) {
                field.value.push(c);
            }
        }
        None => {
            p.editor_buffer.insert(p.editor_cursor, c);
            p.editor_cursor += c.len_utf8();
        }
    }
    refresh_body(p);
}

/// Delete the character before the cursor (active field or body).
pub fn backspace(p: &mut Popout) {
    match p.active_field {
        Some(i) => {
            if let Some(field) = p.fields.get_mut(i) {
                field.value.pop();
            }
        }
        None => {
            if p.editor_cursor > 0 {
                let mut idx = p.editor_cursor;
                loop {
                    idx -= 1;
                    if p.editor_buffer.is_char_boundary(idx) {
                        break;
                    }
                }
                p.editor_buffer.remove(idx);
                p.editor_cursor = idx;
            }
        }
    }
    refresh_body(p);
}

/// Enter: advance to the next field, or insert a newline in the body.
pub fn enter(p: &mut Popout) {
    match p.active_field {
        Some(_) => p.next_field(),
        None => {
            p.editor_buffer.insert(p.editor_cursor, '\n');
            p.editor_cursor += 1;
        }
    }
    refresh_body(p);
}

/// Rebuild a popout's body lines from its fields and editor buffer.
pub fn refresh_body(p: &mut Popout) {
    if !p.is_editor() {
        return; // email views keep their static body
    }

    let mut body: Vec<Line<'static>> = Vec::new();

    for (i, field) in p.fields.iter().enumerate() {
        let marker = if p.active_field == Some(i) {
            "▶"
        } else {
            " "
        };
        let label = format!("{}:", field.label);
        body.push(Line::from(format!("{marker} {label:<9}{}", field.value)));
    }

    if p.has_body_editor() {
        body.push(Line::from(""));
        body.push(Line::from("─".repeat(30)));
        body.push(Line::from(""));

        for line in p.editor_buffer.lines() {
            let styled =
                if line.starts_with("> ") || line.starts_with("-- ") || line.starts_with("---") {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::DarkGray),
                    ))
                } else {
                    Line::from(line.to_string())
                };
            body.push(styled);
        }

        // Visible cursor block when the body is empty
        if p.active_field.is_none() && p.editor_buffer.is_empty() {
            body.push(Line::from(Span::styled(
                "▊",
                Style::default().fg(Color::Cyan),
            )));
        }
    }

    p.body = body;
}

/// Cursor position (column, row) relative to the popout's inner area.
pub fn cursor_position(p: &Popout) -> (u16, u16) {
    use unicode_width::UnicodeWidthStr;

    match p.active_field {
        Some(i) => {
            let col = p
                .fields
                .get(i)
                .map(|f| f.value.as_str().width() as u16)
                .unwrap_or(0);
            (FIELD_PREFIX_WIDTH + col, i as u16)
        }
        None => {
            let before = &p.editor_buffer[..p.editor_cursor];
            let line = before.matches('\n').count();
            let col = before
                .rsplit('\n')
                .next()
                .map(|l| l.width())
                .unwrap_or(0);
            let body_start = p.fields.len() + BODY_GAP_LINES;
            (col as u16, (body_start + line) as u16)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::popout::Popout;

    #[test]
    fn typing_goes_to_active_field_then_body() {
        let mut p = Popout::compose(None);
        insert_char(&mut p, 'a');
        insert_char(&mut p, '@');
        insert_char(&mut p, 'b');
        assert_eq!(p.field("To"), "a@b");
        assert_eq!(p.editor_buffer, "");

        // Advance To → Cc → Bcc → Subject → body
        for _ in 0..4 {
            enter(&mut p);
        }
        assert_eq!(p.active_field, None);
        insert_char(&mut p, 'h');
        insert_char(&mut p, 'i');
        assert_eq!(p.editor_buffer, "hi");
        assert_eq!(p.field("To"), "a@b");
    }

    #[test]
    fn backspace_edits_field_and_body() {
        let mut p = Popout::contact_form();
        insert_char(&mut p, 'J');
        insert_char(&mut p, 'o');
        backspace(&mut p);
        assert_eq!(p.field("Name"), "J");
    }

    #[test]
    fn backspace_handles_multibyte_body_chars() {
        let mut p = Popout::compose(None);
        p.active_field = None;
        insert_char(&mut p, 'é');
        insert_char(&mut p, '本');
        backspace(&mut p);
        assert_eq!(p.editor_buffer, "é");
        backspace(&mut p);
        assert_eq!(p.editor_buffer, "");
        backspace(&mut p); // no panic on empty
    }

    #[test]
    fn cursor_row_matches_field_index() {
        let mut p = Popout::compose(None);
        p.fields[0].value = "user@example.com".into();
        p.active_field = Some(3); // Subject
        let (_, row) = cursor_position(&p);
        assert_eq!(row, 3);

        p.active_field = Some(0);
        let (col, row) = cursor_position(&p);
        assert_eq!(row, 0);
        assert_eq!(col, FIELD_PREFIX_WIDTH + 16);
    }

    #[test]
    fn cursor_in_body_counts_lines_and_chars() {
        let mut p = Popout::compose(None);
        p.active_field = None;
        for c in "ab\ncd".chars() {
            if c == '\n' {
                enter(&mut p);
            } else {
                insert_char(&mut p, c);
            }
        }
        let (col, row) = cursor_position(&p);
        // 4 fields + 3 gap lines + line 1 of the body
        assert_eq!(row, (4 + 3 + 1) as u16);
        assert_eq!(col, 2);
    }
}
