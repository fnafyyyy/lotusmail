# lotusMail

A lightweight desktop mail client that combines what Outlook does well (folders,
rules, many accounts) with what Spark does well (Smart Inbox, snooze, send later,
unified inbox).

**Stack:** Tauri 2 (Rust core + system webview) · Svelte 5 + Tailwind 4 · SQLite.

Offline-first by design: the interface only ever reads from the local database and
never waits on the network. A background engine synchronises over IMAP and emits
events; the interface reacts by reloading from SQLite.

## Getting started

```
npm install
npm run tauri dev        # development build
npm run tauri build      # installer for the current platform
```

On first launch you can load demo data to see the interface without connecting a
real account. The database lives in the app's data directory
(`%APPDATA%\com.pachura.mailmanager\mail.db` on Windows).

## Architecture

```
IMAP/SMTP <-> sync.rs <-> SQLite (db.rs) <-> commands.rs <-> src/lib/api.ts <-> Svelte
```

```
src/                     Svelte 5 interface (runes), Tailwind 4
src-tauri/src/
  commands.rs            Tauri commands - the only bridge to the interface
  db.rs                  SQLite schema and migrations (FTS5, triggers)
  mail.rs                MIME parsing, HTML sanitising (ammonia), categorisation
  sync.rs                synchronisation engine and scheduler (snooze, outbox)
  send.rs                SMTP queue, "send later", sent copy via IMAP APPEND
  oauth.rs               OAuth2 sign-in for Microsoft accounts (PKCE, loopback)
  accounts.rs            passwords in the system keychain, never in SQLite
  sync_config.rs         account transfer between devices, no server involved
```

Two rules the code sticks to. **Passwords never touch SQLite** - they live in the
system keychain (Credential Manager on Windows, Keychain on Apple platforms).
**The database mutex is never held across an await** - network data is collected
into vectors first, then written in short locked blocks.

Message bodies are sanitised on the Rust side (no scripts, remote images stripped)
and rendered inside a sandboxed iframe.

## Platforms

One branch, one codebase. Platform differences live in exactly three places:
`tauri.macos.conf.json` (window chrome and vibrancy), `cfg` conditions in Rust
(keychain, notifications, spell checking) and a handful of `isMac` checks in the
interface. There are no per-platform branches.

```
npm run tauri dev              # Windows and macOS
npm run tauri ios dev          # iOS simulator (macOS only)
npm run tauri build            # installer for the current platform
```

Signing on macOS needs a developer identity:
`export APPLE_SIGNING_IDENTITY="Apple Development: …"` before `tauri build`.
Without it the bundle is ad-hoc signed, which the system treats as a different
application after every rebuild - and it keeps asking for keychain access.

## Diagnostics

Probes that run without opening a window:

```
cargo run --example spell_probe                       # is a system dictionary available
cargo run --example flags_probe -- <account>          # read state: database vs IMAP server
cargo run --example newmail_probe -- <account>        # does the server hold newer mail
cargo run --example trash_probe -- <account>          # trash contents, server vs database
```

## Roadmap

- [x] Skeleton: database, commands, three-pane interface, demo data, dark mode
- [x] IMAP synchronisation with server autodiscovery (known providers, Thunderbird
      autoconfig, TCP probing)
- [x] SMTP sending with an outbox queue, "send later", attachments, signatures,
      sent copies via IMAP APPEND
- [x] Smart Inbox: New/Seen sections, categories, snooze, mailbox cleanup
- [x] Account transfer between devices - an encrypted package carried by the user's
      own mailbox or a copied code, never through a third-party server
- [x] macOS and iOS support: Apple keychain, single-pane layout on narrow screens,
      native window chrome on macOS
- [ ] OAuth2 for Microsoft accounts - core is in place, wiring in progress
- [ ] Two-way status synchronisation (read state and flags back to the server)
- [ ] Rules engine, templates, list virtualisation, POP3
