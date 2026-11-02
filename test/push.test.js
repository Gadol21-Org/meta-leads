import { test, mock } from 'node:test';
import assert from 'node:assert/strict';
import webpush from 'web-push';
import { createPusher } from '../src/push.js';
import { openStore } from '../src/store.js';

const sub = (endpoint) => ({ endpoint, keys: { p256dh: 'p', auth: 'a' } });

test('broadcast reaches every subscription and prunes expired ones', async () => {
  const sent = [];
  mock.method(webpush, 'sendNotification', async (s, body) => {
    sent.push([s.endpoint, JSON.parse(body)]);
    if (s.endpoint.endsWith('/gone')) throw Object.assign(new Error('Gone'), { statusCode: 410 });
    return { statusCode: 201 };
  });

  const store = await openStore(null);
  for (const e of ['https://push/a', 'https://push/b', 'https://push/gone']) await store.addSubscription(sub(e));

  const vapid = webpush.generateVAPIDKeys();
  const pusher = createPusher({ ...vapid, subject: 'mailto:t@example.com' }, store, { warn() {} });
  const result = await pusher.broadcast({ title: 'New lead: Dana', url: '/leads/1' });

  assert.equal(sent.length, 3);
  assert.equal(sent[0][1].url, '/leads/1');
  assert.deepEqual(result, { sent: 2, failed: 1 });
  assert.deepEqual(store.listSubscriptions().map((s) => s.endpoint).sort(), ['https://push/a', 'https://push/b']);
  mock.restoreAll();
});

test('push is a no-op without VAPID keys', async () => {
  const store = await openStore(null);
  const pusher = createPusher({}, store);
  assert.equal(pusher.enabled, false);
  assert.deepEqual(await pusher.broadcast({ title: 'x' }), { sent: 0, failed: 0 });
});
