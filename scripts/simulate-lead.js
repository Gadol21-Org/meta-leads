// Sends a correctly signed fake leadgen webhook to a running server, so you
// can check notifications end to end without a real ad.
//   META_APP_SECRET=... node scripts/simulate-lead.js [http://localhost:3000] [leadgen_id]
import crypto from 'node:crypto';

const base = process.argv[2] || 'http://localhost:3000';
const leadgenId = process.argv[3] || String(Date.now());
const secret = process.env.META_APP_SECRET;
if (!secret) throw new Error('Set META_APP_SECRET to the same value the server uses.');

const body = JSON.stringify({
  object: 'page',
  entry: [
    {
      id: process.env.META_PAGE_ID || '0',
      time: Math.floor(Date.now() / 1000),
      changes: [
        {
          field: 'leadgen',
          value: {
            leadgen_id: leadgenId,
            page_id: process.env.META_PAGE_ID || '0',
            form_id: '0',
            created_time: Math.floor(Date.now() / 1000),
          },
        },
      ],
    },
  ],
});
const sig = 'sha256=' + crypto.createHmac('sha256', secret).update(body).digest('hex');
const res = await fetch(`${base}/webhook`, {
  method: 'POST',
  headers: { 'content-type': 'application/json', 'x-hub-signature-256': sig },
  body,
});
console.log(`POST /webhook -> ${res.status} (leadgen_id ${leadgenId})`);
