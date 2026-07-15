## [0.4.0] - 2026-07-15

### 🚀 Features

- Add calendar/contacts/mail write commands

### 🐛 Bug Fixes

- *(calendar)* Default to account calendar; publish with --no-verify
## [0.3.4] - 2026-07-15

### 🚜 Refactor

- Rename stalwart-rs crate to herald-jmap-stalwart

### ⚙️ Miscellaneous Tasks

- Cut redundant builds and pipeline overlap
- Release v0.3.4
## [0.3.3] - 2026-07-15

### 🐛 Bug Fixes

- Repair workflow YAML indentation and update actions to latest
- Add explicit toolchain input to all rust-toolchain steps
- *(tui)* Resolve clippy collapsible_match in event reader

### 🎨 Styling

- Apply cargo fmt

### ⚙️ Miscellaneous Tasks

- Release v0.3.3
## [0.3.2] - 2026-07-15

### ⚙️ Miscellaneous Tasks

- Release v0.3.2
## [0.3.1] - 2026-07-15

### 🐛 Bug Fixes

- Update actions/upload-artifact to v4
- Correct taiki-e/install-action SHA for v2.49.5
- Pin rust-toolchain to stable explicitly

### ⚙️ Miscellaneous Tasks

- Release v0.3.1
## [0.3.0] - 2026-07-13

### 🚀 Features

- *(herald)* Add ratatui TUI with television-style fuzzy search
- *(herald)* Embed example configs at compile time
- *(herald)* Support path-based folder mappings in config
- *(herald)* Apply folder config to all mail actions including send
- *(herald)* Add 'mail move' and 'mail folder-delete' CLI commands
- *(herald)* Restructure mail screen layout and navigation
- *(herald)* Show search results as overlay panel on left column
- *(herald)* Cooldown-based deep search across folders
- *(herald)* Open folder with right arrow (l) from folder view
- Implement ratatui for TUI and add JMAP push support

### 🐛 Bug Fixes

- *(herald)* Cursor position uses unicode display width
- *(herald)* Folder resolution uses defaults → role → config override
- *(herald)* Folder view shows resolved action targets from config
- *(herald)* Style 'any key' in quit dialog with yellow/orange color
- *(herald)* Hide cursor in normal mode, show blinking bar in edit/search
- *(herald)* Resolve borrow checker issue in contacts screen render
- *(herald)* Layout 1/3 left, 2/3 right; immediate search tick
- *(herald)* Correct mail focus navigation chain

### 🚜 Refactor

- *(herald)* Split TUI into modular components
- *(herald)* Restructure source into focused modules
- *(stalwart-rs)* Split oauth.rs into submodules

### 📚 Documentation

- *(herald)* Add documentation and packaging scripts

### ⚙️ Miscellaneous Tasks

- *(ci)* Update workflows and add AUR publish
- Cleanup
- Release v0.3.0
## [0.2.0] - 2026-07-12

### 🚀 Features

- Publish to crates.io as herald-jmap-cli

### ⚙️ Miscellaneous Tasks

- *(ci)* Skip release PR for ci/docs/build/chore-only fixes
- Release v0.2.0 (#8)
## [0.1.1] - 2026-07-12

### 🐛 Bug Fixes

- *(ci)* Bump actions/checkout to v7 across all workflows
- *(ci)* Remove non-existent label requirement from Hermes Dispatch

### 💼 Other

- *(deps)* Bump actions/upload-artifact from 4 to 7 (#1)
- *(deps)* Bump toml from 0.8.23 to 1.1.2+spec-1.1.0 (#2)
- *(deps)* Bump sha2 from 0.10.9 to 0.11.0 (#4)

### 🚜 Refactor

- *(ci)* Replace third-party release-plz with native Hermes Dispatch

### ⚙️ Miscellaneous Tasks

- *(ci)* Rename secret to HERMES_TOKEN, clean step names
- Release v0.1.1 (#6)
## [0.1.0] - 2026-07-12

### 🚀 Features

- *(herald)* Initial release — JMAP CLI with full auth support
- *(ci)* Fully automated release pipeline

### 🐛 Bug Fixes

- *(ci)* Exclude local .cargo/config.toml from repo
- *(ci)* Track Cargo.lock for reproducible builds
