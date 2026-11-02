import express from 'express';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createAuth } from './auth.js';
import {
  createMetaClient,
  extractLeadgenEvents,
  leadTitle,
  normaliseFields,
  verifySignature,
} from './meta.js';
import { createPusher } from './push.js';

// Webhook times are unix seconds; Graph API times are ISO strings.
function toIso(t) {
  if (t == null || t === '') return new Date().toISOString();
  const d = /^\d+$/.test(String(t)) ? new Date(Number(t) * 1000) : new Date(t);
  return Number.isNaN(d.getTime()) ? new Date().toISOString() : d.toISOString();
}

const PUBLIC_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'public');

export function createApp({ config, store, fetchImpl, log = console }) {
  const meta = createMetaClient(config.meta, fetchImpl);
  const pusher = createPusher(config.vapid, store, log);
  const auth = createAuth(config);
  const app = express();
  app.set('trust proxy', true);

  // ---------- lead ingestion ----------

  function buildLead(graphLead, extra = {}) {
    const fields = normaliseFields(graphLead.field_data);
    const { field_data, ...meta_ } = graphLead;
    return {
      ...meta_,
      ...extra,
      fields,
      title: leadTitle(fields),
    };
  }

  /** Fetches a lead from Meta and stores it. Returns { lead, isNew }. */
  async function ingestLead(leadgenId, hint = {}, source = 'webhook') {
    const existing = store.getLead(leadgenId);
    let graphLead = null;
    let fetchError = null;
    if (meta.configured) {
      try {
        graphLead = await meta.fetchLead(leadgenId);
      } catch (err) {
        fetchError = err.message;
        log.error?.(`could not fetch lead ${leadgenId}: ${err.message}`);
      }
    } else {
      fetchError = 'META_PAGE_ACCESS_TOKEN is not set, so only the webhook data is available.';
    }

    const formId = graphLead?.form_id || hint.form_id;
    let formName = existing?.form_name || hint.form_name;
    if (!formName && formId && meta.configured) formName = (await meta.fetchForm(formId))?.name;

    const base = graphLead || {
      id: String(leadgenId),
      created_time: toIso(hint.created_time),
      form_id: hint.form_id,
      ad_id: hint.ad_id,
      adset_id: hint.adgroup_id,
    };
    const lead = buildLead(base, {
      page_id: hint.page_id || existing?.page_id || config.meta.pageId || undefined,
      form_name: formName,
      fetchError: graphLead ? undefined : fetchError,
      source: existing?.source || source,
      receivedAt: existing?.receivedAt || new Date().toISOString(),
    });
    // If a refresh failed, keep the richer data we already had.
    if (!graphLead && existing && !existing.fetchError) {
      return { lead: existing, isNew: false };
    }
    await store.saveLead(lead);
    return { lead, isNew: !existing };
  }

  async function notifyNewLead(lead) {
    const subtitle = [lead.form_name, lead.campaign_name].filter(Boolean).join(' · ');
    return pusher.broadcast({
      title: `New lead: ${lead.title}`,
      body: subtitle || 'Tap to see all details',
      url: `/leads/${encodeURIComponent(lead.id)}`,
      leadId: lead.id,
      tag: `lead-${lead.id}`,
    });
  }

  // ---------- Meta webhook (public, signature-checked) ----------

  const inFlight = new Set();

  app.get('/webhook', (req, res) => {
    const ok =
      req.query['hub.mode'] === 'subscribe' &&
      config.meta.verifyToken &&
      req.query['hub.verify_token'] === config.meta.verifyToken;
    if (!ok) return res.sendStatus(403);
    res.type('text/plain').send(String(req.query['hub.challenge'] ?? ''));
  });

  app.post('/webhook', express.raw({ type: '*/*', limit: '1mb' }), async (req, res) => {
    const raw = Buffer.isBuffer(req.body) ? req.body : Buffer.from('');
    if (!verifySignature(raw, req.get('x-hub-signature-256'), config.meta.appSecret)) {
      return res.sendStatus(401);
    }
    let payload;
    try {
      payload = JSON.parse(raw.toString('utf8'));
    } catch {
      return res.sendStatus(400);
    }
    // Meta wants a fast 200; it retries otherwise. Process after responding.
    res.sendStatus(200);
    for (const ev of extractLeadgenEvents(payload)) {
      const id = String(ev.leadgen_id);
      if (config.meta.pageId && String(ev.page_id) !== String(config.meta.pageId)) continue;
      // Meta retries and can deliver the same lead twice; notify only once.
      if (store.hasLead(id) || inFlight.has(id)) continue;
      inFlight.add(id);
      try {
        const { lead, isNew } = await ingestLead(String(ev.leadgen_id), ev);
        if (isNew) await notifyNewLead(lead);
      } catch (err) {
        log.error?.(`failed to process lead ${id}: ${err.message}`);
      } finally {
        inFlight.delete(id);
      }
    }
  });

  // ---------- auth ----------

  app.use('/api', express.json({ limit: '100kb' }));
  app.post('/api/login', (req, res) => auth.login(req, res));
  app.post('/api/logout', (req, res) => auth.logout(req, res));
  app.get('/api/session', (req, res) =>
    res.json({ authed: auth.isAuthed(req), passwordRequired: auth.enabled }),
  );
  app.use('/api', (req, res, next) => auth.require(req, res, next));

  // ---------- app API ----------

  app.get('/api/config', (_req, res) =>
    res.json({
      vapidPublicKey: pusher.publicKey || null,
      pushEnabled: pusher.enabled,
      metaConfigured: meta.configured,
    }),
  );

  app.post('/api/subscriptions', async (req, res) => {
    const sub = req.body;
    if (!sub?.endpoint || !sub?.keys?.p256dh || !sub?.keys?.auth) {
      return res.status(400).json({ error: 'invalid push subscription' });
    }
    await store.addSubscription({
      endpoint: sub.endpoint,
      keys: { p256dh: sub.keys.p256dh, auth: sub.keys.auth },
      userAgent: req.get('user-agent') || '',
    });
    res.status(201).json({ ok: true });
  });

  app.delete('/api/subscriptions', async (req, res) => {
    if (req.body?.endpoint) await store.removeSubscription(req.body.endpoint);
    res.json({ ok: true });
  });

  app.post('/api/test-notification', async (_req, res) => {
    const result = await pusher.broadcast({
      title: 'Meta Leads test',
      body: 'Notifications are working on this device.',
      url: '/',
      tag: 'test',
    });
    res.json(result);
  });

  app.get('/api/leads', (_req, res) => {
    const leads = store
      .listLeads()
      .map(({ id, title, created_time, form_name, campaign_name, platform }) => ({
        id,
        title,
        created_time,
        form_name,
        campaign_name,
        platform,
      }));
    res.json({ leads });
  });

  app.get('/api/leads/:id', async (req, res) => {
    let lead = store.getLead(req.params.id);
    if (!lead) return res.status(404).json({ error: 'lead not found' });
    if (req.query.refresh === '1' && meta.configured) {
      lead = (await ingestLead(lead.id, lead, lead.source)).lead;
    }
    res.json({ lead });
  });

  // Read-only import of existing leads from all of the page's forms
  // (no notifications). Nothing is written to Meta.
  app.post('/api/sync', async (_req, res) => {
    if (!meta.configured || !config.meta.pageId) {
      return res.status(400).json({ error: 'Set META_PAGE_ID and META_PAGE_ACCESS_TOKEN first.' });
    }
    let added = 0;
    try {
      for await (const { lead: g, form } of meta.iterateAllLeads()) {
        if (store.hasLead(g.id)) continue;
        await store.saveLead(
          buildLead(g, {
            page_id: config.meta.pageId,
            form_name: form.name,
            source: 'sync',
            receivedAt: new Date().toISOString(),
          }),
        );
        added++;
      }
    } catch (err) {
      return res.status(502).json({ error: err.message, added });
    }
    res.json({ added });
  });

  app.use('/api', (_req, res) => res.status(404).json({ error: 'not found' }));

  // ---------- PWA static files ----------

  app.get('/sw.js', (_req, res) => {
    res.set('Cache-Control', 'no-cache');
    res.sendFile(path.join(PUBLIC_DIR, 'sw.js'));
  });
  app.use(express.static(PUBLIC_DIR, { index: 'index.html' }));
  // Client-side routes, e.g. /leads/123 opened from a notification.
  app.get(['/leads', '/leads/:id'], (_req, res) => res.sendFile(path.join(PUBLIC_DIR, 'index.html')));

  return { app, ingestLead, notifyNewLead };
}
