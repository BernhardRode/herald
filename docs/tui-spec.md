# Herald TUI — Specification (v2)

Status: implemented by the `src/tui/` rewrite (2026-07-13).
Architecture based on the ideas in the [eilmeldung](https://github.com/christo-auer/eilmeldung)
TUI RSS reader: message-passing components, non-blocking async operations,
render-on-demand.

---

## 1. Functionality inventory (carried over from v1)

### Mail
- Profile selection (from config), folder list, per-folder mail list, preview.
- Folder list: tree with indentation; **action folders** (inbox, drafts, sent,
  archive, trash, spam — resolved from config `[profiles.X.folders]` with
  precedence *config override → server role → default name*) are hoisted to
  the top as whole subtree blocks; only the resolved action target shows its
  `[tag]`.
- Mail list: newest first, **server-paginated** (50/page via `Email/query`
  position); scrolling near the bottom lazy-loads the next page.
- Search: `/` opens the input bar. On the mail screen a non-empty query
  switches to all-folder search (results show `[folder]`). Typing filters
  fuzzily (nucleo). **Enter commits** the search: back to Normal with the
  first result selected. **Esc cancels** and restores the previous view.
- Mail actions: reply `r`, forward `f`, archive `a`, delete `d`, spam `s`,
  compose `c`, send from the compose popup. Target folders resolved from
  config. Optional y/n confirmation (`confirm_actions`).
- Open mail (`Enter`) in a popup with the **full text body** (not just the
  preview).

### Contacts
- List with name/email/phone, detail pane.
- **Add (`c`), edit (`e`), delete (`d`)** — v1 had no edit; v2 adds
  `ContactCard/set update`.

### Calendar (refactored in v2)
- v1: flat event list. v2: **month grid + day agenda** (see mock).
- **Add (`c`), edit (`e`), delete (`d`)** — v1 had no edit; v2 adds
  `CalendarEvent/set update`.
- `Enter` opens the event in a popup.

### Global
- `Tab`/`Shift-Tab` cycle screens Mail → Contacts → Calendar.
- **Popups**: entries (mail, contact, event) and editors open as overlay
  popups on top of the full-size main app. A numbered **popup bar** above the
  status line lists open popups; `1`–`9`,`0` toggle/minimize them.
- **Esc ladder**: Esc always steps back towards the inbox — popup → minimize;
  search → cancel; mail list (non-inbox) → folder view; folder view → inbox;
  contacts/calendar → mail/inbox; profiles → folder view. On the inbox, Esc
  opens the quit dialog which is confirmed **only with Enter**.
- `q` quits from anywhere (with dialog). `?` shows the help popup.
- Status bar: context-sensitive key hints + tooltip messages with flavor
  (info/warn/error), async spinner while an operation runs.

### Issues corrected relative to v1
1. **Blocking JMAP calls** — v1 ran every JMAP call with `block_on` inside the
   render loop, freezing the UI. v2 spawns tokio tasks (the `JmapClient` is
   `Clone`) and delivers results as events. The UI never blocks.
2. **Mail body** — v1 popups showed the ~256-char `preview`; v2 fetches the
   full text body on open.
3. **Contact/event editing** — not possible in v1; v2 adds update flows.
4. **Calendar view** — flat list replaced by month grid + agenda.
5. **Selection loss on refresh** — v2 components keep selection by id across
   data reloads.
6. **Modal readability** — explicit background colors on all popups/dialogs.

---

## 2. Architecture

One unbounded `tokio::mpsc` channel carries `Message`s. Every component
implements `MessageReceiver` and sees every message. Rendering happens only
when a `Command::Redraw` message is processed.

```
┌──────────────┐  Event::Key            ┌─────────────────────────────┐
│ input reader │ ─────────────────────▶ │        App::run loop        │
│ (blocking    │                        │  tokio::select! {           │
│  task)       │                        │    rx.recv() → dispatch to  │
└──────────────┘                        │      keymap → Command       │
┌──────────────┐  Event::Async*Finished │      every component        │
│ JmapWorker   │ ─────────────────────▶ │    tick interval → Tick     │
│ tokio::spawn │ ◀───────────────────── │  }                          │
│ per operation│  Command::Load*, Send* │  Command::Redraw → draw()   │
└──────────────┘                        └─────────────────────────────┘
```

- `messages.rs` — `Message { Command, Event }`, `Command` (user intents:
  `SelectNext`, `OpenItem`, `LoadMailPage`, `SendMail{..}`, …), `Event`
  (facts: `Key`, `Tick`, `FoldersLoaded(..)`, `MailSent`, `AsyncFailed(..)`).
- `keymap.rs` — pure `fn map_key(key, &Focus, ..) -> Option<Command>`; unit
  tested.
- `app.rs` — owns screen/focus state, the Esc ladder, popup stack, and all
  components; routes messages; draws frames.
- `worker.rs` — owns `Option<JmapClient>`; on `Command::Load*/Send*/Create*/
  Update*/Delete*` spawns a task with a cloned client that sends
  `Event::*Finished` or `Event::AsyncFailed(context, error)`.
- `model/` — pure, unit-tested logic: folder tree/tagging/sorting, generic
  windowed list with lazy-load, form model, calendar month math.
- `components/` — one file per panel/popup: state + `process` + `render`.

### Focus model

```
Screen  = Mail | Contacts | Calendar
Focus   = the active panel within the screen
Overlay = popup stack on top (topmost focused popup captures keys)
Input   = search/command bar (captures keys while open)
Confirm = quit dialog / destructive-action confirm (captures keys)
```

Key routing priority: Confirm > Input > Popup > Screen panel.

---

## 3. UI mocks

### 3.1 Mail screen

```
┌ Folders ────────────────┐┌ Inbox (128) ────────────────────────────────────┐
│▶ 📁 Inbox      [inbox] 3││▶ Alice — Quarterly report                12:01  │
│  📁 Drafts    [drafts]  ││  Bob   — Re: lunch?                      11:48  │
│  📁 Sent Messages [sent]││  GitHub — [rsjmap] PR #42 merged         09:15  │
│  📁 Archive             ││  Carol — Fotos vom Wochenende            08:02  │
│  📁  └ 2026  [archive]  ││  …                                              │
│  📁 Deleted Msgs [trash]│└─────────────────────────────────────────────────┘
│  📁 Junk Mail    [spam] │┌ Preview ────────────────────────────────────────┐
│  📁 Sent Items          ││ From: Alice <alice@example.com>                 │
│  📁 Deleted Items       ││ Date: 2026-07-13T12:01:44Z                     │
│  📁 Outbox              ││ Subject: Quarterly report                       │
│  …                      ││ ───────────────────────────────────────────    │
│                         ││ Hi, please find attached the numbers for Q2 …  │
└─────────────────────────┘└─────────────────────────────────────────────────┘
 [1 ✉ Quarterly report] [2 ✏ Re: lunch?]                            popup bar
  Mail  Contacts  Calendar   j/k nav  Enter open  / search  c new  q quit
```

Focused panel has a cyan border; `h`/`l` move focus folders ↔ list.
`/` opens the input bar at the bottom (replaces hints line while open):

```
❯ quarterly re▏                                    3/128  ── search (Enter select, Esc cancel)
```

### 3.2 Mail popup (Enter on a message) and compose

```
        ┌ [1] ✉ Quarterly report ───────────────────────────────┐
        │ From:    Alice <alice@example.com>                    │
        │ To:      me@rode.io                                   │
        │ Date:    2026-07-13T12:01:44Z                         │
        │ ──────────────────────────────────────────────────    │
        │ Hi,                                                   │
        │ please find attached the numbers for Q2. The revenue  │
        │ grew by 14% compared to …            (full text body) │
        │                                                       │
        │ r reply  f fwd  a archive  d delete  Esc min  x close │
        └───────────────────────────────────────────────────────┘

        ┌ [2] ✏ Reply: Quarterly report ────────────────────────┐
        │ To:      alice@example.com                            │
        │ Cc:                                                   │
        │ Subject: Re: Quarterly report                         │
        │ ──────────────────────────────────────────────────    │
        │ Thanks Alice!▏                                        │
        │                                                       │
        │ > Hi,                                                 │
        │ > please find attached the numbers for Q2 …           │
        │ Tab next field  s send  Esc min  x discard            │
        └───────────────────────────────────────────────────────┘
```

### 3.3 Contacts screen

```
┌ Contacts (42) ──────────────────┐┌ Detail ───────────────────────────┐
│▶ Alice Example  <alice@ex.com>  ││ Name:  Alice Example              │
│  Bob Meyer      <bob@meyer.de>  ││ Email: alice@example.com          │
│  Carol Chen     <cc@chen.io>    ││ Phone: +49 151 2345678            │
│  …                              ││                                   │
└─────────────────────────────────┘└───────────────────────────────────┘
  Mail  Contacts  Calendar   c add  e edit  d delete  Enter open  q quit
```

`c`/`e` open the contact form popup (edit pre-fills and updates in place):

```
        ┌ ✏ Edit Contact ────────────────────────┐
        │ Name:  Alice Example▏                  │
        │ Email: alice@example.com               │
        │ Phone: +49 151 2345678                 │
        │                                        │
        │ Tab next  s save  Esc min  x discard   │
        └────────────────────────────────────────┘
```

### 3.4 Calendar screen (refactored)

```
┌ July 2026 ──────────────────────────┐┌ Mon 13 Jul — 2 events ────────────┐
│  Mo  Tu  We  Th  Fr  Sa  Su         ││▶ 09:00  Standup (PT30M)           │
│         1   2   3   4   5           ││  14:00  Dentist (PT1H)            │
│   6   7   8   9  10  11  12         ││                                   │
│ [13] 14  15  16· 17  18  19         ││                                   │
│  20  21· 22  23  24  25  26         ││                                   │
│  27  28  29  30  31                 ││                                   │
│                                     ││                                   │
└─────────────────────────────────────┘└───────────────────────────────────┘
  Mail  Contacts  Calendar  h/l day  H/L month  t today  c add  e edit  d del
```

- `[13]` = selected day, `·` = day has events, agenda shows selected day.
- `Enter` on an agenda entry opens the event popup; `e` edits, `d` deletes:

```
        ┌ 📅 Standup ────────────────────────────┐
        │ Title:    Standup                      │
        │ Start:    2026-07-13T09:00:00          │
        │ Duration: PT30M                        │
        │ Status:   confirmed                    │
        │                                        │
        │ e edit  d delete  Esc min  x close     │
        └────────────────────────────────────────┘
```

### 3.5 Dialogs

```
        ┌ Quit Herald? ──────────────┐   ┌ Delete this email? ────────┐
        │                            │   │                            │
        │  Enter quit   any key stay │   │  y confirm    n/Esc cancel │
        └────────────────────────────┘   └────────────────────────────┘
```

Explicit black background, white text, green/red accent on the action keys.

---

## 4. Key bindings

| Context   | Key            | Action                                        |
|-----------|----------------|-----------------------------------------------|
| global    | `Tab`/`S-Tab`  | next/previous screen                          |
| global    | `q`            | quit (dialog, Enter confirms)                 |
| global    | `?`            | help popup                                    |
| global    | `1`–`9`,`0`    | toggle popup N                                |
| lists     | `j`/`k`,`↓`/`↑`| select next/prev (windowed, lazy-loads mail)  |
| mail      | `h`/`l`        | focus folders ↔ mail list                     |
| mail      | `Enter`        | open folder / open mail popup                 |
| mail      | `c r f a d s`  | compose, reply, forward, archive, delete, spam|
| mail      | `/`            | search (Enter commit, Esc cancel)             |
| contacts  | `c e d Enter`  | add, edit, delete, open popup                 |
| calendar  | `h/l H/L t`    | day ±1, month ±1, today                       |
| calendar  | `c e d Enter`  | add, edit, delete, open popup                 |
| popup     | `Esc x m Tab`  | minimize, close/discard, maximize, next popup |
| popup     | `i`/`e`        | edit fields (editor popups)                   |
| editor    | `Tab`/`Enter`  | next field / newline in body                  |
| editor    | `s`            | send / save                                   |
| confirm   | `y n Esc`      | confirm / cancel (quit dialog: Enter/any key) |
| Esc       | —              | ladder: popup→search→list→folders→inbox→quit  |

---

## 5. Testing strategy (TDD)

Pure logic is unit-tested; I/O layers are thin.

- `keymap` — every context mapping (tests: routing priority, esc ladder keys).
- `model::folders` — tree building, action tagging precedence, subtree
  hoisting order.
- `model::window` — windowed selection, scroll, clamp, lazy-load trigger.
- `model::form` — field navigation, editing, body cursor, submit payloads.
- `model::calendar` — month grids (leap years, Monday start), day/month
  navigation, event bucketing by day.
- `components` — reducer tests: feed events in, assert state + emitted
  commands (via a test channel).
- `app` — esc ladder transitions, focus routing, quit dialog ack.
