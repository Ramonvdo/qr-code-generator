<div align="center">

<img src="app-icon.png" width="128" alt="QR Code Generator" />

# QR Code Generator

**Paste a link, get a QR code, export it.**

Static codes that never expire and never redirect through anyone's server.

*Free and open source · local by default, no account, no telemetry*

</div>

---

Plenty of free QR generators wrap your link in a redirect that stops working when their pricing changes, or watermark the download. Both are fixable by generating the code on your own machine, which is all this does.

The codes it produces are **static**. The URL is encoded directly into the pattern, so there is no service in the middle, nothing to expire, and nothing to track. The trade is that a printed code cannot be repointed later. That is the honest cost of not depending on anyone.

## What it does

**Make a code.** Links, plain text, Wi-Fi networks and contact cards. The input is focused on launch, and the preview updates as you type. `Ctrl+Alt+Q` from anywhere brings the window forward with whatever is on your clipboard already rendered.

**Knows a link from a sentence.** A bare `example.com` gets `https://` so phones open it instead of searching for it. When the encoded string differs from what you typed, it is shown to you rather than applied silently.

**Proves the code scans before you save it.** Every render is decoded back by a second, independent library and checked for contrast. Pick two colours that are too close, or a logo that covers too much, and it says so. Without that check you find out the code is unreadable after the stickers are printed.

**Export.** SVG for print, PNG at any size up to 8192px, or straight to the clipboard as an image. A logo can go in the middle, with error correction raised automatically to cover the modules it hides.

**Batch.** Paste a column straight out of a spreadsheet, one entry per line, and get a folder of files. Named from their contents or numbered in order, with every failure reported by line number rather than silently skipped.

**Tickets.** Numbered, signed tickets laid over a PDF template you supply, or onto a blank sheet. Text, images, rules and the code are placed in an editor and filled in from whatever columns your CSV happens to have, so a ticket can carry a name, a seat or a tier without changing anything here. One PDF per buyer, and they can be emailed from the app. See [Tickets](#tickets) below, which is worth reading before you rely on it.

**Check.** A door mode that verifies a ticket and refuses it the second time. Takes input from a USB barcode scanner, which is what most small venues already own.

**Read.** Point it at a screenshot or photo and it tells you what the code actually contains.

Light and dark follow the system. There is no theme setting because there does not need to be one.

## Tickets

Three different problems hide behind the phrase "working tickets", and it is worth being precise about how far each one is solved.

**Issuance** and **authenticity** are handled. Each ticket carries an Ed25519 signature over its event and serial number, so a scanner holding only the public key can verify it offline: no network, no guest list, no shared secret. Forging a ticket needs the private key, which never leaves your machine.

**Double-scan prevention is handled for one door.** The Check tab keeps a record on the machine it runs on, so a repeat scan is refused and told when the ticket was first used. Two doors checking at once cannot see each other's record and can both admit the same ticket. That limit is stated in the tab itself, and the log exports so two doors can be reconciled afterwards.

Issuing a run writes:

| File | What it is |
|---|---|
| `tickets/0001.pdf` | One file per ticket. The name comes from a template you set, so it can be a surname or a seat instead |
| `<event>-tickets.csv` | Your original columns, then `serial`, `ticket_code`, `ticket_sent`, `sent_at`, `event` and `file` |
| `<event>-all-tickets.pdf` | Optional. Every ticket in one file, for printing a roll |
| `<event>-public.key` | Give this to the door. It cannot make tickets, only check them |

An event remembers its own key, its numbering, its template and its design, so a second batch continues from where the first stopped on the same key.

The private key is stored per event under the app's data directory. Losing it means future runs need a new public key at the door; leaking it lets someone else mint tickets.

### Variables

There is one namespace, and everything that takes text takes the same variables: each text box on the ticket, the email subject, the email body, and the filename. Write `{{seat}}` and it is filled in from the `seat` column of your CSV.

Column headings are folded into tokens, so `First Name`, `first name` and `FIRST_NAME` all become `{{first_name}}`. The editor lists every available name as a chip you click to insert, so there is nothing to guess or spell.

These need no column, because the app knows them:

| Variable | What it holds |
|---|---|
| `{{serial}}` | The ticket number, padded: `0001` |
| `{{serial_plain}}` | The same number without padding: `1` |
| `{{ticket_code}}` | The full signed payload the code encodes |
| `{{event}}` | The event name |
| `{{issued_at}}` | When the ticket was issued |
| `{{quantity}}` | How many that buyer's row asked for |

Anything else comes from your file, and any column at all works. A variable with no matching column is left visible as written rather than blanked, so a missing column looks like a mistake instead of quietly printing nothing.

### Designing the ticket

The design is a list of elements laid over your PDF: text, a code, an image drawn from a column of paths, and boxes or rules. Each one can be dragged on the page or nudged with the arrow keys, turned, faded, and reordered, and the list is the stacking order. Text carries a font, a point size, a colour and real left, centre or right alignment, measured with Adobe's own metrics so a centred name is centred whatever its length.

Your uploaded PDF is never rewritten. Each ticket page is your artwork with an overlay on top, which is also why multi-page designs keep all their pages. Editing the text already inside a PDF is deliberately not attempted: it is placed glyph by glyph in a subset font that usually lacks the characters you would type, so edits either fail or visibly break the layout. [Stirling-PDF does not attempt it either](https://docs.stirlingpdf.com/Functionality/Content-Editing/), and its Add Text and Add Stamp tools are overlays for the same reason.

### What your CSV needs

The Tickets tab lists the columns the current design and copy actually refer to, worked out by the same code that fills them in, so the list cannot drift from what a run looks up. **Example CSV** writes a file with exactly those headings and two sample rows.

Only two columns mean anything to the app itself, and you choose which they are after importing:

- **an address column**, needed only to send mail
- **a quantity column**, optional, where a buyer of three gets three separate tickets and so three separate files

Every other column is just a variable. Add a `table` column, put `{{table}}` on the ticket, and it works with no other setup. Rows that cannot be used are listed with their line numbers rather than silently skipped.

### Sending

Put a [Resend](https://resend.com) API key and a verified from-address in **Settings**, once, for all events. The key is stored under the app's data directory and is never written into an event or a manifest.

The subject and the message are written per event and take the same variables as the ticket, so the copy can greet people by name and quote their seat. Each ticket is attached to its own message. As each one goes out, its number is written into the manifest's `ticket_sent` column along with the time, so the manifest is current at any moment and a run that stops part way resumes by sending only what is still pending.

Sending is manual by design. Scheduled delivery from a desktop app only works while the machine is awake, online and running it, and a buyer missing their ticket because a lid closed is worse than no automation at all. If you would rather use your own tooling, take the folder and the manifest into whatever you already send mail with.

### Running it again next year

**Duplicate** copies an event's design, its email copy, its filename template and its template reference under a new name. It deliberately does not copy the signing key, the ledger or the admission log: the copy gets a fresh key and starts at number 1. Sharing a key would mean last year's tickets opening this year's door, which is the exact failure the signature exists to prevent.

### Verifying a ticket

A payload looks like `TKT1.<base64url body>.<base64url signature>`, where the body is `<event>|<serial>`. Verification is three steps:

1. Split on `.`, check the prefix is `TKT1`.
2. Base64url-decode the body and the signature.
3. Ed25519-verify the signature against the body bytes, using the public key.

Only parse the body after the signature checks out. `verify` in [`src-tauri/src/tickets.rs`](src-tauri/src/tickets.rs) is the executable version of those three steps, and its tests cover the cases that matter, including a forged serial with a valid signature attached.

## Building from source

Requires [Rust](https://rustup.rs) and [Node 20+](https://nodejs.org).

```powershell
npm install
npm run app:dev      # run it
npm run app:build    # produce an installer
```

The app icon is generated, not drawn by hand. `npm run icon` rebuilds `app-icon.png` from `scripts/make-icon.mjs` and expands it into the platform set.

## Known limitations

- **A static code cannot be edited after it is printed.** If the destination might move, put a URL you control in the code and redirect on your own server.
- **Templates with rotated pages are refused.** A page carrying a `/Rotate` value is rejected rather than guessed at, because placing a code somewhere plausible but wrong on a thousand printed tickets is the worse failure. Save a flattened copy and try again.
- **The scannability check is a good signal, not a guarantee.** It reads a perfect synthetic image. Print quality, lighting, curved surfaces and cheap scanners are all real and none of them are simulated. Test a printed sample before ordering a thousand of them.
- **Recent codes are session only.** They are a convenience while you work, not a history, and they are gone when the app closes.
- **Ticket text is limited to what WinAnsi can encode.** That covers Western European languages, the currency symbols and the typographic quotes, using the three fonts every PDF reader has built in. Greek, Cyrillic, Hebrew and CJK are not in that encoding and come out as `?`, because the alternative is embedding fonts the app does not carry.
- **The text already inside your template cannot be edited.** Elements are placed over the page, never into it. See [Designing the ticket](#designing-the-ticket) for why.
- **Checking covers one door.** Two machines checking the same event at once cannot see each other's record, and both would admit the same ticket. Use one, or reconcile the exported logs afterwards.
- **Sending needs a verified domain.** Resend will refuse a from-address on a domain you have not verified with them, and it will refuse every message identically. Use the test send before a real run.
- **No code signing yet.** Windows SmartScreen will warn on first run. Checksums are published with each release, which is the honest substitute until there is a certificate.
- **No auto-updater.** New versions are downloaded manually.

## Architecture

- **Rust does the work.** Encoding, rendering, verification, PDF stamping, signing, sending and every file write. The webview collects options and displays what comes back.
- **One matrix, every format.** The preview, the saved SVG, the saved PNG and the scannability check all render from a single QR matrix, so they cannot disagree.
- **The encoder and the decoder are different libraries.** `qrcode` writes the code, `rqrr` reads it back. Agreement between two independent implementations is worth more than either one checked against itself.
- **Tickets are verified end to end, as pictures.** A test issues a run, stamps it into a PDF, rasterises that PDF with an independent renderer, decodes the resulting image and checks the signature. See the dev aids at the bottom of [`src-tauri/src/pdf.rs`](src-tauri/src/pdf.rs).
- **One template engine.** [`src-tauri/src/vars.rs`](src-tauri/src/vars.rs) is the only thing that turns `{{a_variable}}` into text, and the only thing that knows which variables a string mentions. The ticket layout, the email subject, the email body and the filename all go through it, which is what makes the required-columns list trustworthy: it is produced by the same scanner that does the filling in.
- **Stamping never rewrites your template.** Each ticket page is `[your content, our overlay]`, so the artwork appears once in the file and nothing in it is modified. Multi-page designs keep all their pages.
- **The logic does not know Tauri exists.** `payload`, `render`, `verify`, `export`, `pdf`, `tickets` and `decode` are plain modules with no framework imports, which is why the test suite runs without a webview.

## Licence

MIT. See [LICENSE](LICENSE).
