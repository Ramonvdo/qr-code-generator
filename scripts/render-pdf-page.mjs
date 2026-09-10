// Rasterises a page of a PDF so a human can look at it.
//
// Structural assertions cannot tell you whether a ticket looks right, and the
// PDF code path is the one place in this app where "compiles and passes" and
// "actually prints correctly" can diverge. This turns a generated PDF back
// into a picture, which is also what makes the end-to-end test in `pdf.rs`
// possible: render the page, then decode the picture and verify its signature.
//
// The canvas backend is deliberately not a saved dependency. It exists only
// for this check, so it is installed on demand:
//
//   npm install --no-save @napi-rs/canvas
//   node scripts/render-pdf-page.mjs in.pdf out.png [page] [scale]
//
// Then, from src-tauri:
//   cargo test --lib sample_ticket_pdf -- --ignored --nocapture
//   cargo test --lib every_rendered_ticket_verifies -- --ignored --nocapture

import { createCanvas } from "@napi-rs/canvas";
import { readFileSync, writeFileSync } from "node:fs";
import * as pdfjs from "pdfjs-dist/legacy/build/pdf.mjs";

const [, , src, out, pageNo = "1", scale = "2"] = process.argv;
if (!src || !out) {
  console.error("usage: node scripts/render-pdf-page.mjs in.pdf out.png [page] [scale]");
  process.exit(1);
}

const doc = await pdfjs.getDocument({
  data: new Uint8Array(readFileSync(src)),
  isEvalSupported: false,
}).promise;

const page = await doc.getPage(Number(pageNo));
const viewport = page.getViewport({ scale: Number(scale) });
const canvas = createCanvas(Math.ceil(viewport.width), Math.ceil(viewport.height));
const ctx = canvas.getContext("2d");
// The page itself may be transparent; paper is not.
ctx.fillStyle = "#ffffff";
ctx.fillRect(0, 0, canvas.width, canvas.height);

await page.render({ canvasContext: ctx, viewport, canvas: null }).promise;
writeFileSync(out, canvas.toBuffer("image/png"));
console.log(`wrote ${out} (${canvas.width}x${canvas.height}, page ${pageNo} of ${doc.numPages})`);
