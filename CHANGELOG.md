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
## [0.1.0] - 2026-07-12

### 🚀 Features

- *(herald)* Initial release — JMAP CLI with full auth support
- *(ci)* Fully automated release pipeline

### 🐛 Bug Fixes

- *(ci)* Exclude local .cargo/config.toml from repo
- *(ci)* Track Cargo.lock for reproducible builds
