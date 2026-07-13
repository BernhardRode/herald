//! Terminal User Interface for Herald.
//!
//! A television-style split-pane fuzzy search interface (nucleo-powered):
//! results list + search bar on the left, preview on the right. Opened emails
//! and create-forms (draft / contact / event) appear as numbered popout
//! overlays managed from the popout bar.
//!
//! Module map:
//! - [`state`] — the `App` struct, modes, panels, pending server actions
//! - [`actions`] — keyboard action handling (impl blocks on `App`)
//! - [`data`] — JMAP fetches and execution of pending actions
//! - [`editor`] — popout field/body editing and cursor math
//! - [`entries`] — list entry types and display formatting
//! - [`event`] — key → action mapping per input mode
//! - [`popout`] — the popout overlay system
//! - [`render`] — frame composition
//! - [`search`] — nucleo fuzzy matcher wrapper
//! - [`ui`] — main-panel widgets (results, input, preview, layout)
//!
//! # Security: Terminal output sanitization
//!
//! All server-supplied content (email subjects, sender names, body text,
//! contact names, calendar titles) is rendered exclusively through ratatui
//! widgets (`Paragraph`, `List`, `Line`, `Span`). Ratatui's cell-based
//! rendering via crossterm does not pass raw strings to the terminal — each
//! character is written into a screen buffer cell, preventing ANSI escape
//! injection.
//!
//! There are **no** raw `println!`, `eprintln!`, `print!`, or direct
//! stdout/stderr writes of server-controlled data in this module. If raw
//! terminal output paths are added in the future (e.g., post-TUI debug prints,
//! error messages after `ratatui::restore()`), they MUST use
//! `crate::text::sanitize_display()` before printing any server-supplied
//! strings.

mod actions;
mod app;
mod data;
mod editor;
mod entries;
mod event;
mod popout;
mod render;
mod search;
mod state;
mod ui;

use crate::config::Config;

/// Launch the TUI with the given config and optional profile name.
pub fn run(config: Config, profile_name: Option<&str>) -> std::io::Result<()> {
    app::run(config, profile_name)
}
