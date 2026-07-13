//! Editable form model for popup editors: labelled single-line fields plus
//! an optional multi-line body.

/// One labelled single-line input.
#[derive(Debug, Clone)]
pub struct FormField {
    pub label: &'static str,
    pub value: String,
}

/// A form: fields in order, then optionally a body. `focus` indexes the
/// fields; `fields.len()` means the body is focused (only if `body` exists).
#[derive(Debug, Clone)]
pub struct Form {
    pub fields: Vec<FormField>,
    pub body: Option<String>,
    pub focus: usize,
}

impl Form {
    pub fn new(labels: &[&'static str], with_body: bool) -> Self {
        Self {
            fields: labels
                .iter()
                .map(|l| FormField {
                    label: l,
                    value: String::new(),
                })
                .collect(),
            body: with_body.then(String::new),
            focus: 0,
        }
    }

    pub fn field(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map(|f| f.value.as_str())
            .unwrap_or("")
    }

    pub fn set_field(&mut self, label: &str, value: &str) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.label == label) {
            f.value = value.to_string();
        }
    }

    pub fn set_body(&mut self, body: &str) {
        if self.body.is_some() {
            self.body = Some(body.to_string());
        }
    }

    /// Total focusable slots: fields plus the body if present.
    fn slots(&self) -> usize {
        self.fields.len() + usize::from(self.body.is_some())
    }

    pub fn body_focused(&self) -> bool {
        self.body.is_some() && self.focus == self.fields.len()
    }

    /// Advance focus, wrapping around.
    pub fn next_focus(&mut self) {
        if self.slots() > 0 {
            self.focus = (self.focus + 1) % self.slots();
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if self.body_focused() {
            if let Some(body) = &mut self.body {
                body.push(c);
            }
        } else if let Some(f) = self.fields.get_mut(self.focus) {
            f.value.push(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.body_focused() {
            if let Some(body) = &mut self.body {
                body.pop();
            }
        } else if let Some(f) = self.fields.get_mut(self.focus) {
            f.value.pop();
        }
    }

    /// Enter: newline in the body, otherwise advance to the next slot.
    pub fn enter(&mut self) {
        if self.body_focused() {
            if let Some(body) = &mut self.body {
                body.push('\n');
            }
        } else {
            self.next_focus();
        }
    }

    /// Cursor position for rendering: (row, col) inside the form area,
    /// where each field occupies one row and the body starts after a
    /// separator row.
    pub fn cursor(&self) -> (u16, u16) {
        if self.body_focused() {
            let body = self.body.as_deref().unwrap_or("");
            let row = self.fields.len() + 1 + body.lines().count().saturating_sub(1);
            let col = if body.ends_with('\n') || body.is_empty() {
                // caret sits on a fresh line
                if body.ends_with('\n') {
                    return ((self.fields.len() + 1 + body.lines().count()) as u16, 0);
                }
                0
            } else {
                body.lines().last().map(|l| l.chars().count()).unwrap_or(0)
            };
            (row as u16, col as u16)
        } else {
            let field = &self.fields[self.focus];
            // "Label: value" — label column width is computed by the renderer;
            // report the value length, the renderer adds the label offset.
            (self.focus as u16, field.value.chars().count() as u16)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_goes_into_focused_field() {
        let mut f = Form::new(&["To", "Subject"], true);
        f.insert_char('a');
        f.insert_char('b');
        assert_eq!(f.field("To"), "ab");
        f.next_focus();
        f.insert_char('x');
        assert_eq!(f.field("Subject"), "x");
    }

    #[test]
    fn enter_advances_fields_then_newlines_in_body() {
        let mut f = Form::new(&["To"], true);
        f.insert_char('a');
        f.enter();
        assert!(f.body_focused());
        f.insert_char('h');
        f.enter();
        f.insert_char('i');
        assert_eq!(f.body.as_deref(), Some("h\ni"));
    }

    #[test]
    fn focus_wraps_around() {
        let mut f = Form::new(&["A", "B"], false);
        f.next_focus();
        f.next_focus();
        assert_eq!(f.focus, 0);
    }

    #[test]
    fn backspace_edits_focused_slot() {
        let mut f = Form::new(&["A"], true);
        f.insert_char('x');
        f.backspace();
        assert_eq!(f.field("A"), "");
        f.next_focus();
        f.insert_char('y');
        f.backspace();
        assert_eq!(f.body.as_deref(), Some(""));
    }

    #[test]
    fn prefill_and_read_by_label() {
        let mut f = Form::new(&["Name", "Email"], false);
        f.set_field("Email", "a@b.c");
        assert_eq!(f.field("Email"), "a@b.c");
        assert_eq!(f.field("Missing"), "");
    }

    #[test]
    fn cursor_tracks_body_lines() {
        let mut f = Form::new(&["To"], true);
        f.enter(); // to body
        f.insert_char('a');
        f.enter();
        f.insert_char('b');
        // fields.len()=1, separator=1, body line index 1 → row 3? rows: field 0,
        // separator 1, body rows from 2: "a" row 2, "b" row 3.
        let (row, col) = f.cursor();
        assert_eq!(row, 3);
        assert_eq!(col, 1);
    }
}
