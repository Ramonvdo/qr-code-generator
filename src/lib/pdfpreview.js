// Rendering the first page of a template so the code can be positioned on it.
//
// pdf.js is the only thing in the bundle that can draw a PDF, and seeing the
// actual design is the difference between placing a code and guessing at
// coordinates. It is imported only from the tickets route, so it stays out of
// the initial chunk.

import * as pdfjsLib from "pdfjs-dist";
import workerSrc from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { invoke } from "@tauri-apps/api/core";

// Bundled and served from the app's own origin. Nothing is fetched remotely,
// which is why the CSP only needs `worker-src 'self' blob:`.
pdfjsLib.GlobalWorkerOptions.workerSrc = workerSrc;

/**
 * Draw page one of `path` into `canvas` at `displayWidth` CSS pixels.
 *
 * Rendered with `rotation: 0` on purpose. The backend refuses templates whose
 * pages carry a /Rotate value, because placing a code on a rotated page means
 * guessing at the mapping back to PDF user space. Previewing unrotated keeps
 * what is on screen identical to what gets stamped.
 *
 * @returns the page's size in points and its height/width ratio, so the caller
 *   can size the stage and scale point sizes onto it.
 */
export async function renderTemplatePage(path, canvas, displayWidth, pageNumber = 1) {
  const bytes = await invoke("template_bytes", { path });
  const doc = await pdfjsLib.getDocument({ data: new Uint8Array(bytes) }).promise;
  try {
    // Clamped rather than trusted: the stored page can outlive a switch to a
    // shorter template, and pdf.js throws on an out-of-range page.
    const page = await doc.getPage(Math.min(Math.max(1, pageNumber), doc.numPages));
    const base = page.getViewport({ scale: 1, rotation: 0 });

    // Render at device resolution and scale down in CSS, or the page comes
    // out soft on a high-DPI display.
    const dpr = window.devicePixelRatio || 1;
    const viewport = page.getViewport({
      scale: (displayWidth / base.width) * dpr,
      rotation: 0,
    });

    // Only the backing resolution is set. The displayed size is left to CSS,
    // so the stage can be fluid; pinning it in pixels here is what made the
    // page overflow its column in a narrow window.
    canvas.width = Math.round(viewport.width);
    canvas.height = Math.round(viewport.height);

    await page.render({ canvas, viewport }).promise;
    return { ratio: base.height / base.width, width: base.width, height: base.height };
  } finally {
    // Without this the worker holds the document and its buffers for the life
    // of the session, which adds up when trying several templates.
    await doc.destroy();
  }
}
