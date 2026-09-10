# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-09-10

First release. A local QR code generator built around one idea: a code that
looks right is worthless if it does not scan, so the app reads every code back
before you can save it.

### Added

#### Making codes

- Links and plain text, with a single smart input that tells a bare domain from
  a sentence and shows the encoded string whenever it differs from what was
  typed.
- Wi-Fi network and vCard contact codes, with the escaping the formats actually
  require rather than the escaping that usually works.
- Scannability check on every render. The output is decoded back by `rqrr`, a
  different implementation from the `qrcode` encoder, and its colours are
  measured for WCAG contrast. Low contrast, inverted codes and oversized logos
  are reported before anything is written.
- SVG and PNG export, copy to the clipboard as an image or as text, and a logo
  in the centre with error correction forced to High to cover the lost modules.
- Batch export from a pasted list, named from the contents or numbered in
  order. Every failure is reported with its line number instead of being
  skipped silently.
- Reading a code back out of an image file.
- A global `Ctrl+Alt+Q` that brings the window forward with the clipboard
  already rendered.

#### Tickets

- **Signed tickets.** Each carries an Ed25519 signature over its event and
  serial, verifiable offline from a public key alone, with no network, no guest
  list and no shared secret.
- **Events.** An event remembers its signing key, its ticket numbering, its
  template and its design. A second batch continues from where the first
  stopped, on the same key, so tickets issued weeks apart still verify against
  one public key at the door.
- **A ticket editor.** Text, images drawn from a column of paths, boxes and
  rules, and the code itself, each dragged on a preview of the real page or
  nudged with the arrow keys, with rotation, opacity and a stacking order. Text
  carries a font, a size, a colour and real left, centre or right alignment,
  measured with Adobe's published Helvetica, Helvetica-Bold and Courier metrics
  so a centred name sits on the centre line whatever its length.
- **One variable namespace.** Every text box on a ticket, the email subject,
  the email body and the filename take the same `{{variables}}`, filled from
  whatever columns your CSV happens to have. Add a `seat` column and
  `{{seat}}` works everywhere at once. Column headings are folded into tokens,
  so `First Name` becomes `{{first_name}}`, and the editor inserts them from a
  clickable list rather than asking anyone to guess the spelling.
- **The columns this event needs**, listed live from the design and the copy by
  the same scanner that fills them in, with an **Example CSV** button that
  writes a file carrying exactly those headings. Guessing a column name is the
  most likely way a first run goes wrong.
- **Buyer lists.** Import a CSV and issue one ticket per person. Any column at
  all works; only the address and quantity columns mean anything to the app,
  and both are chosen after import. Rows that cannot be used are listed with
  their line numbers rather than skipped.
- **One PDF per ticket**, named from a filename template so a run can be filed
  by surname or seat rather than only by number. A combined single file remains
  as an option, for printing a roll at a venue.
- **Multi-page templates** keep every page in every ticket, and the page
  carrying the code is selectable. The template's own artwork is referenced
  rather than rewritten, so it appears once in the file no matter how many
  tickets are issued.
- **Duplicate an event**, for the same festival a year later. The design, the
  copy, the filename and the template carry over. The signing key, the ledger
  and the admission log deliberately do not: the copy gets a fresh key and
  starts at number 1, because a shared key would mean last year's tickets
  opening this year's door.
- **Sending.** Tickets can be emailed through Resend, one message per ticket
  with its own PDF attached. Manual only: nothing is scheduled and nothing
  polls. A test send is offered first, because an unverified sending domain
  fails every message identically.
- **Write-back on send.** As each ticket goes out, its number and the time land
  in the manifest's `ticket_sent` and `sent_at` columns, so the file is current
  at any moment and a stopped run resumes with only what is still pending.
- **A Check tab.** Verifies a ticket against the event key and refuses it the
  second time, reporting when it was first used. Input comes from a USB barcode
  scanner, which acts as a keyboard and is what most small venues already own.
  The admission log exports as CSV.

### Security

- **One feature reaches the network, and only on an explicit press.** Sending a
  ticket transmits the buyer's address, the message and their PDF to Resend.
  Signing keys, ledgers and redemption logs never leave the machine, and with
  no Resend key configured no network code runs at all.
- Ticket private keys are generated from the operating system's randomness and
  stored per event under the app data directory. The exported key file contains
  only the public half, and says so in a comment.
- The Resend API key is stored in plain text in the app config directory. See
  [SECURITY.md](SECURITY.md).
- State files that fail to parse are quarantined rather than overwritten, so a
  corrupt ticket ledger or admission log is recoverable by hand instead of
  being replaced with an empty one.
- Colours, sizes and file paths arriving from the webview are clamped or parsed
  before anything is allocated or written from them.
- The app declares only the permissions it uses. Rendering, verification, PDF
  work and every file write happen in Rust, so the webview is granted no
  filesystem or shell access at all.

[0.1.0]: https://github.com/Ramonvdo/qr-code-generator/releases/tag/v0.1.0
