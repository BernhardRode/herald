//! Terminal User Interface for Herald.
//!
//! Implements a television-style split-pane fuzzy search interface:
//! - Left panel: results list with fuzzy search input bar
//! - Right panel: preview of the selected item
//!
//! Hierarchical navigation: Profiles → Folders → Mails
//! Arrow left goes back, Arrow right / Enter enters.
//!
//! Uses nucleo for high-performance fuzzy matching.
//!
//! # Security: Terminal output sanitization
//!
//! All server-supplied content (email subjects, sender names, body text, contact
//! names, calendar titles) is rendered exclusively through ratatui widgets
//! (`Paragraph`, `List`, `Line`, `Span`). Ratatui's cell-based rendering via
//! crossterm does not pass raw strings to the terminal — each character is
//! written into a screen buffer cell, preventing ANSI escape injection.
//!
//! There are **no** raw `println!`, `eprintln!`, `print!`, or direct
//! stdout/stderr writes of server-controlled data in this module. Therefore
//! manual `sanitize_display()` calls are not required here.
//!
//! If raw terminal output paths are added in the future (e.g., post-TUI debug
//! prints, error messages after `ratatui::restore()`), they MUST use
//! `crate::sanitize::sanitize_display()` before printing any server-supplied
//! strings.

mod app;
mod event;
pub mod popout;
mod search;
mod ui;

use crate::config::Config;

/// Launch the TUI with the given config and optional profile name.
pub fn run(config: Config, profile_name: Option<&str>) -> std::io::Result<()> {
    app::run(config, profile_name)
}
