// Meta Leads PWA client. No framework, no build step.

const $ = (sel, root = document) => root.querySelector(sel);
const view = $('#view');
let config = null;

// ---------- helpers ----------

const esc = (s) =>
  String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

async function api(path, opts = {}) {
  const res = await fetch(path, {
    credentials: 'same-origin',
    headers: opts.body ? { 'content-type': 'application/json' } : undefined,
    ...opts,
    body: opts.body ? JSON.stringify(opts.body) : undefined,
  });
  const json = await res.json().catch(() => ({}));
  if (res.status === 401 && path !== '/api/login') {
    renderLogin();
    throw Object.assign(new Error('login required'), { handled: true });
  }
  if (!res.ok) throw Object.assign(new Error(json.error || `HTTP ${res.status}`), { status: res.status, body: json });
  return json;
}

function toast(msg, ms = 3000) {
  const t = $('#toast');
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.hidden = true), ms);
}

const fmtDate = (iso) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '' : d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
};

function relTime(iso) {
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (!Number.isFinite(s)) return '';
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return new Date(iso).toLocaleDateString();
}

function linkify(name, value) {
  const v = esc(value);
  if (/email/i.test(name) && /^[^\s@]+@[^\s@]+$/.test(value)) return `<a href="mailto:${v}">${v}</a>`;
  if (/phone/i.test(name) && /^[+\d][\d\s().-]{5,}$/.test(value)) {
    const tel = value.replace(/[^\d+]/g, '');
    return `<a href="tel:${esc(tel)}">${v}</a> · <a href="https://wa.me/${esc(tel.replace('+', ''))}" target="_blank" rel="noopener">WhatsApp</a>`;
  }
  if (/^https?:\/\/\S+$/.test(value)) return `<a href="${v}" target="_blank" rel="noopener">${v}</a>`;
  return v;
}

// ---------- routing ----------

function navigate(url, replace = false) {
  history[replace ? 'replaceState' : 'pushState']({}, '', url);
  route();
}

document.addEventListener('click', (e) => {
  const a = e.target.closest('a[data-link]');
  if (a && a.origin === location.origin && !e.metaKey && !e.ctrlKey) {
    e.preventDefault();
    navigate(a.pathname + a.search);
  }
});
window.addEventListener('popstate', route);

async function route() {
  $('#menu').hidden = true;
  const m = location.pathname.match(/^\/leads\/([^/]+)$/);
  try {
    if (m) await renderLead(decodeURIComponent(m[1]));
    else await renderList();
  } catch (err) {
    if (!err.handled) view.innerHTML = `<p class="empty">Something went wrong: ${esc(err.message)}</p>`;
  }
}

// ---------- views ----------

function renderLogin() {
  $('#menu-btn').hidden = true;
  $('#notify-btn').hidden = true;
  view.innerHTML = `
    <form class="card login" id="login-form">
      <h1>Sign in</h1>
      <label>Password <input type="password" name="password" autocomplete="current-password" required autofocus></label>
      <button class="btn primary">Sign in</button>
      <p class="error" hidden></p>
    </form>`;
  $('#login-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
      await api('/api/login', { method: 'POST', body: { password: e.target.password.value } });
      await boot();
    } catch (err) {
      const p = $('.error', e.target);
      p.textContent = err.message;
      p.hidden = false;
    }
  });
}

async function renderList() {
  const { leads } = await api('/api/leads');
  view.innerHTML = `
    ${!config.metaConfigured ? `<p class="banner">Meta page token isn't configured, so leads only show their ids. See README.</p>` : ''}
    ${
      leads.length
        ? `<ul class="lead-list">${leads
            .map(
              (l) => `
          <li><a href="/leads/${encodeURIComponent(l.id)}" data-link class="lead-row">
            <div>
              <div class="lead-name">${esc(l.title)}</div>
              <div class="muted small">${esc([l.form_name, l.campaign_name].filter(Boolean).join(' · ') || l.id)}</div>
            </div>
            <div class="right muted small">${esc(relTime(l.created_time))}</div>
          </a></li>`,
            )
            .join('')}</ul>`
        : `<p class="empty">No leads yet. New leads appear here and as notifications.</p>`
    }`;
}

async function renderLead(id) {
  view.innerHTML = `<p class="empty">Loading…</p>`;
  let lead;
  try {
    ({ lead } = await api(`/api/leads/${encodeURIComponent(id)}`));
  } catch (err) {
    if (err.status === 404) {
      view.innerHTML = `<p class="empty">This lead isn't here yet. <a href="/" data-link>Back to leads</a></p>`;
      return;
    }
    throw err;
  }

  const metaRows = [
    ['Received', fmtDate(lead.created_time)],
    ['Form', lead.form_name || lead.form_id],
    ['Campaign', lead.campaign_name],
    ['Ad set', lead.adset_name],
    ['Ad', lead.ad_name || lead.ad_id],
    ['Platform', lead.platform && lead.platform.toUpperCase()],
    ['Organic', lead.is_organic === undefined ? '' : lead.is_organic ? 'Yes' : 'No'],
    ['Partner', lead.partner_name],
    ['Lead id', lead.id],
  ].filter(([, v]) => v);

  const disclaimers = (lead.custom_disclaimer_responses || [])
    .map((d) => [d.checkbox_key, d.is_checked === '1' || d.is_checked === true ? 'Checked' : 'Not checked']);

  view.innerHTML = `
    <a href="/" data-link class="back">← All leads</a>
    <section class="card">
      <h1>${esc(lead.title)}</h1>
      ${lead.fetchError ? `<p class="banner">${esc(lead.fetchError)} <button class="link" id="refresh-btn">Retry</button></p>` : ''}
      <dl class="fields">
        ${(lead.fields || []).map((f) => `<dt>${esc(f.label)}</dt><dd>${linkify(f.name, f.value) || '<span class="muted">—</span>'}</dd>`).join('')}
        ${disclaimers.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('')}
      </dl>
    </section>

    <section class="card">
      <h2>Ad details</h2>
      <dl class="fields">${metaRows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('')}</dl>
    </section>`;

  $('#refresh-btn')?.addEventListener('click', async () => {
    await api(`/api/leads/${encodeURIComponent(id)}?refresh=1`);
    renderLead(id);
  });
}

// ---------- push notifications ----------

function urlBase64ToUint8Array(base64) {
  const pad = '='.repeat((4 - (base64.length % 4)) % 4);
  const raw = atob((base64 + pad).replace(/-/g, '+').replace(/_/g, '/'));
  return Uint8Array.from(raw, (c) => c.charCodeAt(0));
}

async function currentSubscription() {
  if (!('serviceWorker' in navigator) || !('PushManager' in window)) return null;
  const reg = await navigator.serviceWorker.ready;
  return reg.pushManager.getSubscription();
}

async function updateNotifyButton() {
  const btn = $('#notify-btn');
  const supported = 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window;
  if (!config.pushEnabled) return (btn.hidden = true);
  if (!supported) {
    // iOS only exposes Web Push once the app is added to the Home Screen.
    btn.hidden = false;
    btn.disabled = true;
    btn.textContent = /iPhone|iPad/.test(navigator.userAgent) ? 'Add to Home Screen for alerts' : 'Notifications unsupported';
    return;
  }
  const sub = await currentSubscription();
  if (sub) {
    // Re-register so the server always has this device (cheap and idempotent).
    api('/api/subscriptions', { method: 'POST', body: sub.toJSON() }).catch(() => {});
  }
  btn.hidden = Boolean(sub) || Notification.permission === 'denied';
  btn.disabled = false;
  btn.textContent = 'Enable notifications';
}

async function subscribe() {
  const perm = await Notification.requestPermission();
  if (perm !== 'granted') return toast('Notifications were not allowed.');
  const reg = await navigator.serviceWorker.ready;
  const sub = await reg.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: urlBase64ToUint8Array(config.vapidPublicKey),
  });
  await api('/api/subscriptions', { method: 'POST', body: sub.toJSON() });
  toast('Notifications are on for this device.');
  updateNotifyButton();
}

async function unsubscribe() {
  const sub = await currentSubscription();
  if (!sub) return toast('This device is not subscribed.');
  await api('/api/subscriptions', { method: 'DELETE', body: { endpoint: sub.endpoint } });
  await sub.unsubscribe();
  toast('Notifications are off for this device.');
  updateNotifyButton();
}

// ---------- menu ----------

$('#notify-btn').addEventListener('click', () => subscribe().catch((e) => toast(e.message)));
$('#menu-btn').addEventListener('click', () => ($('#menu').hidden = !$('#menu').hidden));
$('#menu').addEventListener('click', async (e) => {
  const action = e.target.dataset.action;
  if (!action) return;
  $('#menu').hidden = true;
  try {
    if (action === 'sync') {
      toast('Importing leads from Meta…', 10000);
      const { added } = await api('/api/sync', { method: 'POST' });
      toast(`Imported ${added} lead${added === 1 ? '' : 's'}.`);
      route();
    } else if (action === 'test-push') {
      const { sent } = await api('/api/test-notification', { method: 'POST' });
      toast(`Sent to ${sent} device${sent === 1 ? '' : 's'}.`);
    } else if (action === 'unsubscribe') {
      await unsubscribe();
    } else if (action === 'logout') {
      await api('/api/logout', { method: 'POST' });
      renderLogin();
    }
  } catch (err) {
    if (!err.handled) toast(err.message);
  }
});

// The service worker asks us to navigate when a notification is tapped and
// the app is already open, and tells us when a new lead arrives.
navigator.serviceWorker?.addEventListener('message', (e) => {
  if (e.data?.type === 'open' && e.data.url) navigate(e.data.url);
  if (e.data?.type === 'new-lead' && location.pathname === '/') route();
});

// ---------- boot ----------

async function boot() {
  const session = await api('/api/session');
  if (!session.authed) return renderLogin();
  config = await api('/api/config');
  $('#menu-btn').hidden = false;
  if ('serviceWorker' in navigator) {
    await navigator.serviceWorker.register('/sw.js');
    updateNotifyButton().catch(() => {});
  }
  route();
}

document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible' && config && location.pathname === '/') route();
});

boot().catch((err) => {
  if (!err.handled) view.innerHTML = `<p class="empty">Could not start: ${esc(err.message)}</p>`;
});
