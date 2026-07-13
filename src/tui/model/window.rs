//! Windowed list selection: a cursor inside a visible window that scrolls
//! over a larger (possibly lazily growing) item set.

/// A scrolling window over `total` items with an inner cursor.
#[derive(Debug, Default, Clone)]
pub struct ListWindow {
    /// Index of the first visible item.
    pub offset: usize,
    /// Cursor position within the window (0..height).
    pub cursor: usize,
    /// Number of visible rows.
    pub height: usize,
}

impl ListWindow {
    pub fn new() -> Self {
        Self {
            offset: 0,
            cursor: 0,
            height: 10,
        }
    }

    pub fn set_height(&mut self, height: usize) {
        self.height = height.max(1);
    }

    /// Absolute index of the selected item.
    pub fn selected(&self) -> usize {
        self.offset + self.cursor
    }

    /// Number of items currently visible.
    pub fn visible_len(&self, total: usize) -> usize {
        total.saturating_sub(self.offset).min(self.height)
    }

    /// Move the selection down; scrolls the window at the bottom edge.
    /// Returns true if the selection moved.
    pub fn select_next(&mut self, total: usize) -> bool {
        if self.selected() + 1 >= total {
            return false;
        }
        if self.cursor + 1 < self.visible_len(total) {
            self.cursor += 1;
        } else {
            self.offset += 1;
        }
        true
    }

    /// Move the selection up; scrolls the window at the top edge.
    pub fn select_prev(&mut self) -> bool {
        if self.cursor > 0 {
            self.cursor -= 1;
            true
        } else if self.offset > 0 {
            self.offset -= 1;
            true
        } else {
            false
        }
    }

    /// Jump to an absolute index, positioning the window around it.
    pub fn select(&mut self, index: usize, total: usize) {
        let index = index.min(total.saturating_sub(1));
        if index < self.offset || index >= self.offset + self.height {
            self.offset = index.saturating_sub(self.height / 2);
        }
        self.cursor = index - self.offset.min(index);
        self.clamp(total);
    }

    /// Keep window and cursor within the item set (it may have shrunk).
    pub fn clamp(&mut self, total: usize) {
        if total == 0 {
            self.offset = 0;
            self.cursor = 0;
            return;
        }
        let max_offset = total.saturating_sub(self.height);
        if self.offset > max_offset {
            self.offset = max_offset;
        }
        let visible = self.visible_len(total);
        if self.cursor >= visible {
            self.cursor = visible.saturating_sub(1);
        }
    }

    pub fn reset(&mut self) {
        self.offset = 0;
        self.cursor = 0;
    }

    /// Whether the selection is within `threshold` items of the end —
    /// the signal to lazily fetch the next page.
    pub fn near_end(&self, total: usize, threshold: usize) -> bool {
        self.selected() + threshold >= total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(height: usize) -> ListWindow {
        let mut w = ListWindow::new();
        w.set_height(height);
        w
    }

    #[test]
    fn cursor_moves_before_window_scrolls() {
        let mut w = win(3);
        assert!(w.select_next(10));
        assert!(w.select_next(10));
        assert_eq!((w.offset, w.cursor), (0, 2));
        // window is full → next moves the offset
        assert!(w.select_next(10));
        assert_eq!((w.offset, w.cursor), (1, 2));
        assert_eq!(w.selected(), 3);
    }

    #[test]
    fn stops_at_last_item() {
        let mut w = win(3);
        for _ in 0..20 {
            w.select_next(5);
        }
        assert_eq!(w.selected(), 4);
        assert!(!w.select_next(5));
    }

    #[test]
    fn select_prev_scrolls_at_top_edge() {
        let mut w = win(3);
        for _ in 0..5 {
            w.select_next(10);
        }
        assert_eq!((w.offset, w.cursor), (3, 2));
        w.cursor = 0;
        assert!(w.select_prev());
        assert_eq!((w.offset, w.cursor), (2, 0));
    }

    #[test]
    fn clamp_after_shrink() {
        let mut w = win(5);
        for _ in 0..50 {
            w.select_next(100);
        }
        w.clamp(3);
        assert!(w.selected() < 3);
        w.clamp(0);
        assert_eq!(w.selected(), 0);
    }

    #[test]
    fn near_end_triggers_lazy_load() {
        let mut w = win(10);
        assert!(!w.near_end(50, 10));
        for _ in 0..39 {
            w.select_next(50);
        }
        assert_eq!(w.selected(), 39);
        assert!(!w.near_end(50, 10));
        w.select_next(50);
        assert!(w.near_end(50, 10));
    }

    #[test]
    fn select_absolute_centers_window() {
        let mut w = win(10);
        w.select(50, 100);
        assert_eq!(w.selected(), 50);
        assert!(w.offset <= 50 && 50 < w.offset + 10);
        w.select(999, 100);
        assert_eq!(w.selected(), 99);
    }

    #[test]
    fn empty_list_is_safe() {
        let mut w = win(5);
        assert!(!w.select_next(0));
        assert!(!w.select_prev());
        w.clamp(0);
        assert_eq!(w.selected(), 0);
    }
}
