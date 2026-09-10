# Security

## Reporting a vulnerability

Please report privately through GitHub's
[security advisory form](https://github.com/Ramonvdo/qr-code-generator/security/advisories/new)
rather than opening a public issue. You will get an acknowledgement within a
few days.

## Supported versions

| Version | Supported |
|---|---|
| 0.1.x | Yes |

## Security posture

The app is local by default. It has no account system and sends no telemetry,
and the webview is granted no filesystem, network or shell permission. Every
privileged action happens in a Rust command.

### What leaves the machine, and when

One feature makes a network request, and only when you press Send:

- **Emailing tickets.** The buyer's address, the subject, the message body and
  their ticket PDF are sent to Resend's API over HTTPS. Nothing is scheduled,
  nothing polls, and no other feature opens a socket.
- **Signing keys never leave.** Neither do buyer lists as a whole, ticket
  ledgers, redemption logs, templates or any code you generate.

If you never configure a Resend key, no network code runs at all: sending is
refused locally before a request is built.

Two further caveats:

- The Windows installer may download the WebView2 runtime if the machine does
  not already have it. That is Microsoft's bootstrapper, not this app.
- There is no auto-updater, so security fixes require a manual download.

### The Resend API key

Stored in plain text in the app's config directory
(`%APPDATA%\app.qrcodegenerator.desktop\settings.json` on Windows). Anyone who
can read your user profile can read it, and it can send mail as your verified
domain. Treat it like any other saved password, and revoke it from the Resend
dashboard if the machine is lost.

### Ticket signing keys

The ticket feature generates an Ed25519 key per event, seeded from the
operating system's randomness, and stores the private seed under the app data
directory (`%APPDATA%\app.qrcodegenerator.desktop\events\` on Windows).

- The file is stored unencrypted. Anyone who can read your user profile can
  read it, and anyone holding it can issue tickets that verify as genuine.
- The exported `<event>-public.key` contains only the public half and cannot be
  used to create tickets.
- Signed tickets prove origin. Preventing a second use needs a record kept
  where tickets are checked, which the Check tab maintains on the machine it
  runs on. That record covers one door: two machines checking the same event
  cannot see each other and would both admit the same ticket.
- The redemption log is not tamper-evident. Anyone who can write to the app
  data directory can delete an admission and let a used ticket back in.

### Untrusted input

The app parses files that other people may have given you: images to read a
code from, and PDF templates. Both are handled by memory-safe Rust libraries
(`image`, `rqrr`, `lopdf`) and failures are reported rather than ignored. PDF
templates are additionally validated before use, and a template whose geometry
cannot be interpreted is refused rather than guessed at.
