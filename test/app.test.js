import { test } from 'node:test';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import { createApp } from '../src/server.js';
import { openStore } from '../src/store.js';
import { loadConfig } from '../src/config.js';
import { extractLeadgenEvents, leadTitle, normaliseFields, verifySignature } from '../src/meta.js';

const SECRET = 'app-secret';

function fakeGraph(calls) {
  return async (url) => {
    const u = new URL(url);
    calls.push(u.pathname);
    if (u.pathname.endsWith('/111')) {
      return Response.json({
        id: '111',
        created_time: '2026-11-02T08:00:00+0000',
        form_id: '900',
        campaign_name: 'Autumn promo',
        platform: 'ig',
        field_data: [
          { name: 'full_name', values: ['Dana Levi'] },
          { name: 'email', values: ['dana@example.com'] },
          { name: 'phone_number', values: ['+972501234567'] },
        ],
      });
    }
    if (u.pathname.endsWith('/900')) return Response.json({ id: '900', name: 'Contact form' });
    return Response.json({ error: { message: 'not found' } }, { status: 404 });
  };
}

async function start(env = {}) {
  const config = loadConfig({
    META_APP_SECRET: SECRET,
    META_VERIFY_TOKEN: 'verify-me',
    META_PAGE_ACCESS_TOKEN: 'token',
    ...env,
  });
  const store = await openStore(null);
  const calls = [];
  const { app } = createApp({ config, store, fetchImpl: fakeGraph(calls), log: { error() {}, warn() {} } });
  const server = app.listen(0);
  await new Promise((r) => server.once('listening', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  return { base, store, calls, close: () => server.close() };
}

const signed = (body) => ({
  method: 'POST',
  headers: {
    'content-type': 'application/json',
    'x-hub-signature-256': 'sha256=' + crypto.createHmac('sha256', SECRET).update(body).digest('hex'),
  },
  body,
});

const webhookBody = (id) =>
  JSON.stringify({
    object: 'page',
    entry: [{ id: '5', changes: [{ field: 'leadgen', value: { leadgen_id: id, page_id: '5', form_id: '900', created_time: 1793606400 } }] }],
  });

const waitFor = async (fn) => {
  for (let i = 0; i < 50; i++) {
    if (fn()) return;
    await new Promise((r) => setTimeout(r, 10));
  }
  throw new Error('timed out');
};

test('webhook verification handshake', async () => {
  const s = await start();
  const ok = await fetch(`${s.base}/webhook?hub.mode=subscribe&hub.verify_token=verify-me&hub.challenge=42`);
  assert.equal(ok.status, 200);
  assert.equal(await ok.text(), '42');
  const bad = await fetch(`${s.base}/webhook?hub.mode=subscribe&hub.verify_token=nope&hub.challenge=42`);
  assert.equal(bad.status, 403);
  s.close();
});

test('webhook rejects bad signatures', async () => {
  const s = await start();
  const res = await fetch(`${s.base}/webhook`, {
    method: 'POST',
    headers: { 'x-hub-signature-256': 'sha256=' + '0'.repeat(64) },
    body: webhookBody('111'),
  });
  assert.equal(res.status, 401);
  assert.equal(s.store.listLeads().length, 0);
  s.close();
});

test('signed webhook stores the full lead from the Graph API, once', async () => {
  const s = await start();
  const body = webhookBody('111');
  assert.equal((await fetch(`${s.base}/webhook`, signed(body))).status, 200);
  await waitFor(() => s.store.hasLead('111'));
  await fetch(`${s.base}/webhook`, signed(body)); // Meta retry
  await new Promise((r) => setTimeout(r, 50));

  const lead = s.store.getLead('111');
  assert.equal(lead.title, 'Dana Levi');
  assert.equal(lead.form_name, 'Contact form');
  assert.equal(lead.campaign_name, 'Autumn promo');
  assert.deepEqual(lead.fields.map((f) => f.label), ['Full name', 'Email', 'Phone number']);
  assert.equal(s.calls.filter((p) => p.endsWith('/111')).length, 1, 'duplicate delivery is ignored');
  s.close();
});

test('lead is still stored with webhook data when Graph fetch fails', async () => {
  const s = await start();
  await fetch(`${s.base}/webhook`, signed(webhookBody('222')));
  await waitFor(() => s.store.hasLead('222'));
  const lead = s.store.getLead('222');
  assert.match(lead.fetchError, /not found/);
  assert.equal(lead.created_time, new Date(1793606400 * 1000).toISOString());
  s.close();
});

test('API serves leads and is read-only', async () => {
  const s = await start();
  await fetch(`${s.base}/webhook`, signed(webhookBody('111')));
  await waitFor(() => s.store.hasLead('111'));

  const { leads } = await (await fetch(`${s.base}/api/leads`)).json();
  assert.equal(leads.length, 1);
  const { lead } = await (await fetch(`${s.base}/api/leads/111`)).json();
  assert.equal(lead.fields.find((f) => f.name === 'email').value, 'dana@example.com');

  for (const method of ['POST', 'PUT', 'PATCH', 'DELETE']) {
    const res = await fetch(`${s.base}/api/leads/111`, { method });
    assert.equal(res.status, 404, `${method} on a lead is not supported`);
  }
  assert.equal((await fetch(`${s.base}/api/leads/111/stage`, { method: 'POST' })).status, 404);
  s.close();
});

test('notification deep link is served by the PWA shell', async () => {
  const s = await start();
  const res = await fetch(`${s.base}/leads/111`);
  assert.equal(res.status, 200);
  assert.match(await res.text(), /<main id="view"/);
  s.close();
});

test('meta helpers', () => {
  assert.equal(verifySignature(Buffer.from('x'), 'sha256=zz', SECRET), false);
  assert.deepEqual(extractLeadgenEvents({ object: 'user', entry: [] }), []);
  const fields = normaliseFields([
    { name: 'first_name', values: ['A'] },
    { name: 'last_name', values: ['B'] },
  ]);
  assert.equal(leadTitle(fields), 'A B');
  assert.equal(leadTitle([]), 'New lead');
});
