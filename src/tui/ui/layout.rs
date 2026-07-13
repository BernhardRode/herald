//! Split-pane layout computation with popout support.
//!
//! Layout modes:
//! - No popouts: standard split (results+input left, preview right)
//! - 1 normal popout: main shrinks to left half, popout takes right half
//! - 2 normal popouts: main hidden, two popouts side-by-side
//! - 1 maximized popout: full overlay

use ratatui::layout::{Constraint, Direction, Layout as RatatuiLayout, Rect};

/// Pre-computed layout rectangles for a single frame.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The results list area (left panel, above input).
    pub results: Rect,
    /// The input/search bar area (left panel, below results).
    pub input: Rect,
    /// The preview pane area (right panel) — None if popouts cover it.
    pub preview: Option<Rect>,
    /// Status bar at the very bottom.
    pub status_bar: Rect,
    /// Popout panel areas (0, 1, or 2).
    pub popout_areas: Vec<Rect>,
    /// Minimized popout tab bar area (above status bar, if any minimized).
    pub minimized_bar: Option<Rect>,
}

/// Height of the input bar.
const INPUT_BAR_HEIGHT: u16 = 3;
/// Default preview panel size percentage.
const PREVIEW_PANEL_PERCENT: u16 = 50;

impl Layout {
    /// Build layout based on number of visible popouts and their state.
    pub fn build(
        area: Rect,
        visible_popout_count: usize,
        has_maximized: bool,
        has_minimized: bool,
    ) -> Self {
        // Reserve status bar (1 line) and optional minimized bar (1 line)
        let minimized_height = if has_minimized { 1 } else { 0 };
        let [main_area, minimized_bar_area, status_bar] = RatatuiLayout::vertical([
            Constraint::Min(0),
            Constraint::Length(minimized_height),
            Constraint::Length(1),
        ])
        .areas(area);

        let minimized_bar = if has_minimized {
            Some(minimized_bar_area)
        } else {
            None
        };

        // If a popout is maximized, it takes the full main area
        if has_maximized {
            // Main view is hidden; show only the maximized popout(s)
            let (results, input) = empty_left_panel(main_area);
            return Self {
                results,
                input,
                preview: None,
                status_bar,
                popout_areas: vec![main_area],
                minimized_bar,
            };
        }

        match visible_popout_count {
            0 => {
                // Standard layout: results+input | preview
                let [left_panel, preview] = RatatuiLayout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(100 - PREVIEW_PANEL_PERCENT),
                        Constraint::Percentage(PREVIEW_PANEL_PERCENT),
                    ])
                    .areas(main_area);

                let [results, input] = RatatuiLayout::vertical([
                    Constraint::Min(3),
                    Constraint::Length(INPUT_BAR_HEIGHT),
                ])
                .areas(left_panel);

                Self {
                    results,
                    input,
                    preview: Some(preview),
                    status_bar,
                    popout_areas: vec![],
                    minimized_bar,
                }
            }
            1 => {
                // Main (results+input) on left 40%, popout on right 60%
                let [left_panel, popout_area] = RatatuiLayout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .areas(main_area);

                let [results, input] = RatatuiLayout::vertical([
                    Constraint::Min(3),
                    Constraint::Length(INPUT_BAR_HEIGHT),
                ])
                .areas(left_panel);

                Self {
                    results,
                    input,
                    preview: None,
                    status_bar,
                    popout_areas: vec![popout_area],
                    minimized_bar,
                }
            }
            _ => {
                // 2 popouts side-by-side, main list compressed to narrow strip
                let [list_strip, pop1, pop2] = RatatuiLayout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(20),
                        Constraint::Percentage(40),
                        Constraint::Percentage(40),
                    ])
                    .areas(main_area);

                let [results, input] = RatatuiLayout::vertical([
                    Constraint::Min(3),
                    Constraint::Length(INPUT_BAR_HEIGHT),
                ])
                .areas(list_strip);

                Self {
                    results,
                    input,
                    preview: None,
                    status_bar,
                    popout_areas: vec![pop1, pop2],
                    minimized_bar,
                }
            }
        }
    }
}

/// Create empty zero-height rects for the left panel when it's hidden.
fn empty_left_panel(area: Rect) -> (Rect, Rect) {
    let zero = Rect::new(area.x, area.y, 0, 0);
    (zero, zero)
}
