// The only file that talks to Tauri. Every backend call goes through here so
// the components stay declarative and the IPC surface is greppable in one
// place.

import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog, open as openDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { writeText, readText } from "@tauri-apps/plugin-clipboard-manager";

// ── rendering ──────────────────────────────────────────────────────────────

/** Guess link vs text. Lives in Rust so there is only one copy of the rules. */
export const detectKind = (input) => invoke("detect_kind", { input });

/** Render, verify, and return the SVG plus everything the UI reports. */
export const preview = (req) => invoke("preview", { req });

/** A filename stem derived from the payload, for the save dialog. */
export const suggestFilename = (req) => invoke("suggest_filename", { req });

// ── export ─────────────────────────────────────────────────────────────────

/**
 * Ask for a destination, then render and write it.
 *
 * Returns the written path, or null if the user cancelled the dialog. The
 * bytes never cross IPC: Rust renders straight to disk.
 */
export async function saveAs(req, format) {
  const stem = await suggestFilename(req);
  const path = await saveDialog({
    defaultPath: `${stem}.${format}`,
    filters: [{ name: format.toUpperCase(), extensions: [format] }],
  });
  if (!path) return null;
  return invoke("save", { req, format, path });
}

export const reveal = (path) => revealItemInDir(path);

// ── logo ───────────────────────────────────────────────────────────────────

/**
 * Choose a logo and decode it into the backend's cache.
 *
 * Returns its name and dimensions, or null if the dialog was cancelled. The
 * image itself stays in Rust: only a fraction is sent per render, not the file.
 */
export async function pickLogo() {
  const path = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "gif", "webp"] }],
  });
  if (!path) return null;
  return invoke("load_logo", { path });
}

export const clearLogo = () => invoke("clear_logo");

// ── batch ──────────────────────────────────────────────────────────────────

/**
 * Ask for a destination folder, then write one file per line.
 *
 * Returns the outcome, or null if the folder picker was cancelled. The base
 * request is spread flat because the Rust side flattens it too.
 */
export async function batchExport(base, { text, format, naming }) {
  const directory = await openDialog({ directory: true, multiple: false });
  if (!directory) return null;
  return invoke("batch_export", {
    req: { ...base, text, directory, format, naming },
  });
}

// ── clipboard ──────────────────────────────────────────────────────────────

/**
 * Put the QR on the clipboard as an image.
 *
 * Tauri's clipboard plugin is text-only, so this goes through the webview's
 * own Clipboard API. `png_bytes` returns a raw binary response, which arrives
 * here as an ArrayBuffer rather than a JSON array of 200,000 numbers.
 */
export async function copyImage(req) {
  const buf = await invoke("png_bytes", { req });
  const blob = new Blob([buf], { type: "image/png" });
  await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
}

export const copyText = (text) => writeText(text);
export const readClipboardText = () => readText();

// ── events ─────────────────────────────────────────────────────────────────

export const listEvents = () => invoke("list_events");
export const loadEvent = (id) => invoke("load_event", { id });
export const createEvent = (name) => invoke("create_event", { name });
export const saveEvent = (event) => invoke("save_event", { event });
export const eventTickets = (id) => invoke("event_tickets", { id });
export const eventPublicKey = (id) => invoke("event_public_key", { id });

/** Copy an event for another run. The copy gets its own signing key. */
export const duplicateEvent = (id, name) => invoke("duplicate_event", { id, name });

/** The CSV columns this event's design and copy refer to. */
export const requiredColumns = (id) => invoke("required_columns", { id });

/** Write an example CSV with exactly the columns this event needs. */
export async function writeExampleCsv(id, suggested) {
  const path = await saveDialog({
    defaultPath: suggested,
    filters: [{ name: "CSV", extensions: ["csv"] }],
  });
  if (!path) return null;
  return invoke("write_example_csv", { id, path });
}

/**
 * Choose a PDF template and validate it.
 *
 * Validation happens now rather than at issue time, so a rotated or unreadable
 * template is refused before anything has been positioned on it.
 */
export async function pickTemplate() {
  const path = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (!path) return null;
  const info = await invoke("load_template", { path });
  return { path, ...info };
}

// ── buyers ─────────────────────────────────────────────────────────────────

/** Choose a buyer CSV, guess its columns, and parse it. */
export async function pickBuyers() {
  const path = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: "CSV", extensions: ["csv", "txt"] }],
  });
  if (!path) return null;
  return invoke("load_buyers", { path });
}

/** Re-read the same file after the user corrected a column. */
export const remapBuyers = (path, mapping) => invoke("remap_buyers", { path, mapping });

// ── issuing ────────────────────────────────────────────────────────────────

/** Pick a folder, then write one PDF per ticket into it. */
export async function issue(req) {
  const directory = await openDialog({ directory: true, multiple: false });
  if (!directory) return null;
  return invoke("issue", { req: { ...req, directory } });
}

/** A fresh element for the layout editor, with sensible starting values. */
export function newElement(kind) {
  const id = `${kind}-${Math.random().toString(36).slice(2, 8)}`;
  const base = { id, kind, x: 0.1, y: 0.2, rotation: 0, opacity: 1 };
  if (kind === "text") {
    return {
      ...base,
      template: "{{first_name}}",
      points: 14,
      font: "helvetica",
      colour: "#101014",
      align: "left",
    };
  }
  if (kind === "image") {
    return { ...base, width: 0.15, height: 0.08, column: "logo" };
  }
  if (kind === "shape") {
    return {
      ...base,
      width: 0.3,
      height: 0.002,
      fill: "#101014",
      stroke: null,
      strokeWidth: 1,
    };
  }
  return { ...base, size: 0.26 };
}

// ── the door ───────────────────────────────────────────────────────────────

export const checkTicket = (eventId, scanned) =>
  invoke("check_ticket", { eventId, scanned });

export const doorState = (eventId) => invoke("door_state", { eventId });

export async function exportRedemptions(eventId, suggested) {
  const path = await saveDialog({
    defaultPath: suggested,
    filters: [{ name: "CSV", extensions: ["csv"] }],
  });
  if (!path) return null;
  return invoke("export_redemptions", { eventId, path });
}

// ── events from the backend ────────────────────────────────────────────────

/**
 * Fires when the global shortcut is pressed, carrying the clipboard text.
 *
 * Returns the unlisten function, which the caller is responsible for.
 */
export const onClipboardCode = (fn) => listen("clipboard-code", (e) => fn(e.payload));

// ── settings and sending ───────────────────────────────────────────────────

export const loadSettings = () => invoke("load_settings");
export const saveSettings = (config) => invoke("save_settings", { config });

/** One message to an address you choose, to prove the setup works. */
export const sendTest = (to) => invoke("send_test", { to });

/**
 * Start emailing every ticket that has not gone out.
 *
 * Returns how many are queued. Progress arrives as events, because a run of
 * five hundred takes minutes and a blocked window looks like a crash.
 */
export const sendTickets = (eventId, retryFailed) =>
  invoke("send_tickets", { eventId, retryFailed });

export const cancelSend = () => invoke("cancel_send");

export const onSendProgress = (fn) => listen("send-progress", (e) => fn(e.payload));
export const onSendDone = (fn) => listen("send-done", (e) => fn(e.payload));
