# Herald — JMAP CLI for Stalwart Mail Server

An opinionated command-line interface for JMAP mail servers, built in Rust. Tested against [Stalwart Mail Server](https://stalw.art/).

## Features

- **Multiple auth methods**: app-password, API key, OAuth browser (PKCE), OAuth device flow
- **Mail operations**: send, list inbox, read messages, list mailboxes
- **Multi-profile config**: XDG-compliant TOML or environment variables
- **Token caching**: OAuth tokens stored securely in `$XDG_DATA_HOME/herald/tokens/`
- **JSON-friendly**: designed for scripting and piping

## Installation

```bash
# From source
cargo install --path .

# Or build locally
cargo build --release
./target/release/herald --help
```

## Quick Start

### Option A: Environment variables (`.env` file)

```bash
cp .env.example .env
# Edit .env with your server URL and credentials
herald auth login
herald mail mailboxes
herald mail send --to user@example.com --subject "Hello" --body "World"
```

### Option B: Config file

```bash
herald config init
# Edit ~/.config/herald/config.toml
herald auth login
```

## Commands

```
herald auth login          # Test authentication
herald auth status         # Show session info
herald mail send           # Send an email
herald mail list           # List recent inbox emails
herald mail read --id <ID> # Read a specific email
herald mail mailboxes      # List mailboxes
herald config show         # Show current config
herald config path         # Print config file path
herald config init         # Create starter config
```

## Configuration

### Config file (`~/.config/herald/config.toml`)

```toml
default_profile = "work"

[profiles.work]
server_url = "https://mail.example.com"
from_email = "you@example.com"
from_name = "Your Name"

[profiles.work.auth]
method = "app-password"
username = "you@example.com"
password = "your-app-password"
```

### Auth methods

| Method | Config key | Description |
|--------|-----------|-------------|
| `app-password` | `username` + `password` | HTTP Basic auth |
| `api-key` | `token` | Bearer token |
| `oauth-browser` | `client_id` | Authorization Code + PKCE |
| `oauth-device` | `client_id` | Device Authorization Grant |

### OAuth token storage

OAuth tokens are cached at `~/.local/share/herald/tokens/` and automatically refreshed when expired.

## Architecture

```
crate-herald/
├── src/
│   ├── main.rs          # CLI entry point (clap)
│   ├── config.rs        # XDG + TOML + env config
│   ├── auth.rs          # Auth orchestration + token cache
│   ├── output.rs        # Error formatting
│   └── commands/
│       ├── auth.rs      # auth login/status
│       ├── mail.rs      # mail send/list/read/mailboxes
│       └── config.rs    # config show/path/init
└── crate-stalwart-rs/
    └── src/
        ├── lib.rs
        └── oauth.rs     # OAuth discovery, PKCE, device flow
```

## Dependencies

Built on the [crate-jmap](https://github.com/MarkAtwood/crate-jmap) protocol library:
- `jmap-base-client` — session, auth, blob, transport
- `jmap-mail-client` — Email, Mailbox, Identity, EmailSubmission methods
- `jmap-types` / `jmap-mail-types` — wire format types

## License

MIT OR Apache-2.0
