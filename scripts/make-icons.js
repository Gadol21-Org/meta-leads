// Generates the PWA PNG icons with no image dependencies: a blue tile with a
// white bell-ish dot. Re-run with `npm run icons` after changing colours.
import fs from 'node:fs';
import zlib from 'node:zlib';

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
};

function png(size, { rounded, padding = 0, mono = false }) {
  const rows = [];
  const r = size * 0.22;
  const c = size / 2;
  for (let y = 0; y < size; y++) {
    const row = [0];
    for (let x = 0; x < size; x++) {
      // rounded-square mask
      let inside = true;
      if (rounded) {
        const dx = Math.max(r - x, 0, x - (size - 1 - r));
        const dy = Math.max(r - y, 0, y - (size - 1 - r));
        inside = dx * dx + dy * dy <= r * r;
      }
      // "lead" glyph: head circle + shoulders arc
      const s = size - padding * 2;
      const head = Math.hypot(x - c, y - (padding + s * 0.38)) <= s * 0.14;
      const bodyD = Math.hypot(x - c, y - (padding + s * 0.86));
      const body = bodyD <= s * 0.3 && y <= padding + s * 0.76;
      const glyph = head || body;
      if (mono) row.push(255, 255, 255, glyph ? 255 : 0);
      else if (!inside) row.push(0, 0, 0, 0);
      else if (glyph) row.push(255, 255, 255, 255);
      else row.push(0x1d, 0x4e, 0xd8, 255);
    }
    rows.push(Buffer.from(row));
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0); ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; ihdr[9] = 6; // 8-bit RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(Buffer.concat(rows))),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

const out = new URL('../public/icons/', import.meta.url);
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(new URL('icon-192.png', out), png(192, { rounded: true, padding: 20 }));
fs.writeFileSync(new URL('icon-512.png', out), png(512, { rounded: true, padding: 52 }));
fs.writeFileSync(new URL('maskable-512.png', out), png(512, { rounded: false, padding: 110 }));
fs.writeFileSync(new URL('apple-touch-icon.png', out), png(180, { rounded: false, padding: 24 }));
fs.writeFileSync(new URL('badge-96.png', out), png(96, { mono: true, padding: 8 }));
console.log('icons written');
