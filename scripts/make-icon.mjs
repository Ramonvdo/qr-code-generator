// Generates the 1024px source icon that `npm run tauri icon` expands into the
// full platform set (.ico, .icns, and the PNG sizes).
//
// It writes the PNG by hand rather than pulling in a graphics library. The
// icon is a dozen rounded rectangles, and a dependency that exists only to
// draw them would be a dependency to audit, update and explain forever. Node
// ships zlib, which is the only hard part of a PNG.
//
// Usage:  node scripts/make-icon.mjs [outfile]
// Then:   npm run tauri icon app-icon.png

import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";

const SIZE = 1024;
// Samples per axis. Rounded corners are the whole reason this exists; without
// supersampling they come out as visible staircases at the sizes that matter.
const SS = 3;

// Matches the app's dark surface and accent so the icon and the window agree.
const GROUND = [0x17, 0x17, 0x1a];
const MARK = [0xf2, 0xf2, 0xf5];
const ACCENT = [0x4c, 0xb1, 0xe8];

// The mark is laid out on a 9x9 module grid: three finder squares in the
// corners, which is the part people actually recognise at 16px, plus a sparse
// scatter suggesting data. Anything finer turns to mush in a taskbar.
const GRID = 9;
const FINDERS = [
  [0, 0],
  [6, 0],
  [0, 6],
];
const DATA = [
  [5, 4], [7, 4],
  [4, 5], [6, 5], [8, 5],
  [5, 6], [7, 6],
  [4, 7], [6, 7], [8, 7],
  [5, 8], [7, 8],
];

/** Coverage test for a rounded rectangle, in pixel units. */
function inRoundRect(px, py, x, y, w, h, r) {
  const dx = Math.max(x + r - px, 0, px - (x + w - r));
  const dy = Math.max(y + r - py, 0, py - (y + h - r));
  return dx * dx + dy * dy <= r * r;
}

function render() {
  // The mark sits inside a margin so it never touches the ground's corners.
  const margin = SIZE * 0.17;
  const inner = SIZE - margin * 2;
  const unit = inner / GRID;

  const shapes = [];
  for (const [gx, gy] of FINDERS) {
    shapes.push({
      color: MARK,
      x: margin + gx * unit,
      y: margin + gy * unit,
      w: unit * 3,
      h: unit * 3,
      r: unit * 0.85,
    });
  }
  for (const [gx, gy] of DATA) {
    shapes.push({
      color: ACCENT,
      x: margin + gx * unit,
      y: margin + gy * unit,
      w: unit,
      h: unit,
      r: unit * 0.3,
    });
  }

  const px = Buffer.alloc(SIZE * SIZE * 4);
  const groundR = SIZE * 0.22;
  const step = 1 / SS;
  const offset = step / 2;

  for (let y = 0; y < SIZE; y++) {
    for (let x = 0; x < SIZE; x++) {
      let gHits = 0;
      // Accumulated colour of whichever mark shape covered each sample.
      let mHits = 0;
      let mr = 0, mg = 0, mb = 0;

      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const fx = x + offset + sx * step;
          const fy = y + offset + sy * step;
          if (!inRoundRect(fx, fy, 0, 0, SIZE, SIZE, groundR)) continue;
          gHits++;
          for (const s of shapes) {
            if (inRoundRect(fx, fy, s.x, s.y, s.w, s.h, s.r)) {
              mHits++;
              mr += s.color[0];
              mg += s.color[1];
              mb += s.color[2];
              break;
            }
          }
        }
      }

      const total = SS * SS;
      const i = (y * SIZE + x) * 4;
      if (gHits === 0) continue;

      // Blend the marks over the ground, then let the ground's own coverage
      // drive alpha so the outer corners stay soft.
      const mFrac = mHits / total;
      const gFrac = gHits / total;
      const avgM = mHits ? [mr / mHits, mg / mHits, mb / mHits] : [0, 0, 0];
      for (let c = 0; c < 3; c++) {
        px[i + c] = Math.round(GROUND[c] * (1 - mFrac) + avgM[c] * mFrac);
      }
      px[i + 3] = Math.round(255 * gFrac);
    }
  }
  return px;
}

// -------------------------------------------------------------- png writer --

const CRC_TABLE = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();

function crc32(buf) {
  let c = -1;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function png(pixels, size) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type: RGBA
  // 10..12 stay zero: deflate, adaptive filtering, no interlace.

  // Every scanline carries a leading filter byte. Filter 0 (none) keeps this
  // simple and still compresses well on flat colour.
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    const src = y * size * 4;
    const dst = y * (size * 4 + 1);
    raw[dst] = 0;
    pixels.copy(raw, dst + 1, src, src + size * 4);
  }

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const out = process.argv[2] ?? "app-icon.png";
writeFileSync(out, png(render(), SIZE));
console.log(`Wrote ${out} (${SIZE}x${SIZE})`);
