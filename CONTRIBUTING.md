# Contributing

## Prerequisites

[Rust](https://rustup.rs) and [Node 20+](https://nodejs.org).

## Development loop

```powershell
npm install
npm run app:dev
```

Before opening a pull request, run what CI runs:

```powershell
npm run build
cd src-tauri
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

## Project map

```
src/                    the webview: options in, rendered SVG out
  app.css               the whole design system, tokens and UI kit
  lib/api.js            the only file that talks to Tauri
  lib/Preview.svelte    live code, scannability verdict, export row
  lib/Layout.svelte     the ticket element editor: stage, list, properties
  lib/pdfpreview.js     pdf.js, loaded on demand, draws the template page
  lib/modes/            one component per tab
src-tauri/src/
  lib.rs                commands. Thin: parse, delegate, serialise
  payload.rs            what the user typed becomes the encoded string
  render.rs             matrix in, SVG or PNG or greyscale out
  verify.rs             reads our own output back and judges it
  export.rs             atomic writes and safe filenames
  storage.rs            JSON that is quarantined, never overwritten
  vars.rs               the only template engine. {{name}} in, text out
  fonts.rs              character widths and WinAnsi, so text lands right
  events.rs             the event record, its ledger and its serials
  buyers.rs             a spreadsheet becomes rows of arbitrary columns
  batch.rs              parsing a pasted list
  pdf.rs                laying elements over a template
  tickets.rs            Ed25519 issuance and verification
  checkin.rs            what a scan at the door means
  email.rs              the one thing here that touches the network
  decode.rs             reading a code out of an image
  b64.rs                base64, both alphabets
scripts/                standalone tools, each explaining why it exists
```

## Conventions worth knowing

- **The logic does not import Tauri.** Everything except `lib.rs` is a plain
  module. That is what keeps the tests fast and runnable without a webview.
- **One matrix, every output.** Preview, SVG, PNG and the scannability check
  all render from the same `Qr`. Never add a second path that produces pixels.
- **One template engine.** `vars.rs` is the only place that expands
  `{{a_variable}}`, and the only place that reports which variables a string
  mentions. The layout, the email subject, the email body and the filename all
  call it. A second expander would let the "columns this event needs" panel
  disagree with what a run actually looks up, and the operator would only find
  out from a printed ticket.
- **Anything writable is measurable.** `fonts.rs` owns both halves: the
  WinAnsi byte a character is written as, and its width. They have to cover
  exactly the same set of characters, and a test asserts it. A character that
  can be drawn but not measured comes out fine on a left-aligned line and
  visibly crooked on a centred one.
- **PDF strings are bytes.** The content stream is assembled as Rust text, so
  anything above ASCII goes in as an octal escape of its WinAnsi byte. Pushing
  the character directly writes UTF-8 into a stream the reader decodes as
  WinAnsi, which printed `café` as `cafÃ©`.
- **Elements are laid over the template, never into it.** The uploaded PDF's
  own content stream is shared by reference and never rewritten.
- **Expected values in tests come from somewhere else.** RFC 4648 vectors for
  base64, the WCAG formula for contrast, the ZXing documentation for Wi-Fi
  strings, a different library for decoding. Never assert that the code agrees
  with itself.
- **The webview is a trust boundary.** Colours, sizes and paths coming from it
  are parsed or clamped before anything is allocated or written.
- **Refuse rather than guess.** A rotated PDF page is rejected with an
  explanation. Silently placing a code somewhere plausible but wrong is worse
  than not placing it.
- **Never overwrite state you could not read.** A ticket ledger or an
  admission log that fails to parse is renamed aside, not replaced. See
  `storage.rs`.
- **Say what leaves the machine.** Sending is the only network call, and it
  only happens on an explicit press. If that ever changes, README and
  SECURITY change in the same commit.
- **Comments say why, not what.** Record the reason, and where there was one,
  the failure that motivated the line.

## Testing

There are no automated UI tests. Describe the manual steps you took in the pull
request. For anything touching the door, scan the same ticket twice and confirm the
second is refused with the first scan's time. Any keyboard typing a payload
followed by Enter exercises the same path a USB scanner does.

For anything touching the PDF path, generate and look at a real file:

```powershell
cd src-tauri
cargo test --lib sample_ticket_pdf -- --ignored --nocapture
cd ..
npm install --no-save @napi-rs/canvas
node scripts/render-pdf-page.mjs "$env:TEMP\qrgen-sample-tickets.pdf" "$env:TEMP\qrgen-ticket-p1.png" 1 3
```

## Releases

Bump the version in `package.json`, `src-tauri/Cargo.toml` and
`src-tauri/tauri.conf.json` together, commit as `Release 0.1.0`, then push a
matching `v0.1.0` tag. The release workflow builds and drafts the release.
