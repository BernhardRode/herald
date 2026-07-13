//! Frame layout: full-size main app, popout overlays on top, popout bar.
//!
//! The main app (results + input + preview) always occupies the full frame.
//! Active popouts are drawn as centered overlays: one popout covers most of
//! the screen, two are shown side by side, a maximized popout covers the whole
//! main area. When any popouts exist, a one-line popout bar sits above the
//! status bar.

use ratatui::layout::{Constraint, Direction, Layout as RatatuiLayout, Rect};

/// Pre-computed layout rectangles for a single frame.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The results list area (left panel, above input).
    pub results: Rect,
    /// The input/search bar area (left panel, below results).
    pub input: Rect,
    /// The preview pane area (right panel).
    pub preview: Rect,
    /// Status bar at the very bottom.
    pub status_bar: Rect,
    /// Overlay areas for the active popouts (0, 1, or 2).
    pub popout_areas: Vec<Rect>,
    /// Popout bar (above status bar) — present when any popouts are open.
    pub popout_bar: Option<Rect>,
}

/// Height of the input bar.
const INPUT_BAR_HEIGHT: u16 = 3;
/// Default preview panel size percentage.
const PREVIEW_PANEL_PERCENT: u16 = 50;
/// Overlay size as a percentage of the main area.
const OVERLAY_PERCENT: u16 = 88;

impl Layout {
    /// Build the frame layout.
    pub fn build(
        area: Rect,
        active_popout_count: usize,
        has_maximized: bool,
        has_popouts: bool,
    ) -> Self {
        let popout_bar_height = if has_popouts { 1 } else { 0 };
        let [main_area, popout_bar_area, status_bar] = RatatuiLayout::vertical([
            Constraint::Min(0),
            Constraint::Length(popout_bar_height),
            Constraint::Length(1),
        ])
        .areas(area);

        let popout_bar = has_popouts.then_some(popout_bar_area);

        // Main app: results+input | preview
        let [left_panel, preview] = RatatuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(100 - PREVIEW_PANEL_PERCENT),
                Constraint::Percentage(PREVIEW_PANEL_PERCENT),
            ])
            .areas(main_area);

        let [results, input] =
            RatatuiLayout::vertical([Constraint::Min(3), Constraint::Length(INPUT_BAR_HEIGHT)])
                .areas(left_panel);

        let popout_areas = if has_maximized {
            vec![main_area]
        } else {
            overlay_areas(main_area, active_popout_count)
        };

        Self {
            results,
            input,
            preview,
            status_bar,
            popout_areas,
            popout_bar,
        }
    }
}

/// Compute centered overlay rects for the active popouts.
fn overlay_areas(main: Rect, count: usize) -> Vec<Rect> {
    match count {
        0 => vec![],
        1 => vec![centered(main, OVERLAY_PERCENT, OVERLAY_PERCENT)],
        _ => {
            let overlay = centered(main, 96, OVERLAY_PERCENT);
            let [left, right] = RatatuiLayout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(overlay);
            vec![left, right]
        }
    }
}

/// A rect centered inside `area` covering the given percentages.
fn centered(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let width = area.width * percent_x / 100;
    let height = area.height * percent_y / 100;
    Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    )
}
