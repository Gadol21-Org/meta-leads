// Everything that talks to Meta: webhook verification, the Graph API lead
// lookup and backfill import. Read-only: nothing here writes to Meta.

import crypto from 'node:crypto';

const LEAD_FIELDS = [
  'id',
  'created_time',
  'field_data',
  'ad_id',
  'ad_name',
  'adset_id',
  'adset_name',
  'campaign_id',
  'campaign_name',
  'form_id',
  'is_organic',
  'platform',
  'partner_name',
  'retailer_item_id',
  'custom_disclaimer_responses',
].join(',');

/** Checks the X-Hub-Signature-256 header Meta sends with every webhook POST. */
export function verifySignature(rawBody, header, appSecret) {
  if (!appSecret || !header || !header.startsWith('sha256=')) return false;
  const expected = crypto.createHmac('sha256', appSecret).update(rawBody).digest('hex');
  const given = header.slice('sha256='.length);
  if (given.length !== expected.length) return false;
  return crypto.timingSafeEqual(Buffer.from(given, 'hex'), Buffer.from(expected, 'hex'));
}

/** Pulls the leadgen change events out of a page webhook payload. */
export function extractLeadgenEvents(payload) {
  const events = [];
  if (!payload || payload.object !== 'page') return events;
  for (const entry of payload.entry || []) {
    for (const change of entry.changes || []) {
      if (change.field === 'leadgen' && change.value?.leadgen_id) {
        events.push({ ...change.value, page_id: change.value.page_id || entry.id });
      }
    }
  }
  return events;
}

export function createMetaClient(cfg, fetchImpl = globalThis.fetch) {
  const base = `https://graph.facebook.com/${cfg.graphVersion}`;

  async function graph(pathname, { params = {} } = {}) {
    const url = new URL(`${base}/${pathname}`);
    for (const [k, v] of Object.entries(params)) url.searchParams.set(k, v);
    url.searchParams.set('access_token', cfg.pageAccessToken);
    const res = await fetchImpl(url);
    const json = await res.json().catch(() => ({}));
    if (!res.ok || json.error) {
      const msg = json.error?.message || `HTTP ${res.status}`;
      const err = new Error(`Graph API ${pathname}: ${msg}`);
      err.status = res.status;
      throw err;
    }
    return json;
  }

  return {
    configured: Boolean(cfg.pageAccessToken),

    /** Full lead record: every form answer plus ad/campaign context. */
    fetchLead(leadgenId) {
      return graph(leadgenId, { params: { fields: LEAD_FIELDS } });
    },

    fetchForm(formId) {
      return graph(formId, { params: { fields: 'id,name' } }).catch(() => null);
    },

    /** Backfill: every lead from every form on the page, newest first per form. */
    async *iterateAllLeads({ maxPerForm = 200 } = {}) {
      let forms = await graph(`${cfg.pageId}/leadgen_forms`, {
        params: { fields: 'id,name', limit: '100' },
      });
      const allForms = [...(forms.data || [])];
      while (forms.paging?.next && allForms.length < 1000) {
        forms = await (await fetchImpl(forms.paging.next)).json();
        allForms.push(...(forms.data || []));
      }
      for (const form of allForms) {
        let seen = 0;
        let page = await graph(`${form.id}/leads`, {
          params: { fields: LEAD_FIELDS, limit: '100' },
        });
        while (page) {
          for (const lead of page.data || []) {
            if (seen++ >= maxPerForm) break;
            yield { lead, form };
          }
          if (seen >= maxPerForm || !page.paging?.next) break;
          page = await (await fetchImpl(page.paging.next)).json();
        }
      }
    },

  };
}

/** Turns Graph API field_data into a flat, display-friendly list. */
export function normaliseFields(fieldData = []) {
  return fieldData.map((f) => ({
    name: f.name,
    label: humanise(f.name),
    value: (f.values || []).join(', '),
  }));
}

function humanise(name = '') {
  return name
    .replace(/[_-]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/^\w/, (c) => c.toUpperCase());
}

/** A short "who is this" line for notifications and the list view. */
export function leadTitle(fields) {
  const get = (...names) =>
    fields.find((f) => names.includes(f.name.toLowerCase()))?.value || '';
  const name =
    get('full_name', 'name') || [get('first_name'), get('last_name')].filter(Boolean).join(' ');
  return name || get('email') || get('phone_number', 'phone') || 'New lead';
}
