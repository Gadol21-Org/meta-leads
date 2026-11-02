// Service worker: shows push notifications, opens the lead page on tap,
// and caches the app shell so the PWA opens instantly (and offline).

const CACHE = 'meta-leads-v4';
const SHELL = ['/', '/manifest.webmanifest', '/icons/logo-192.png'];

self.addEventListener('install', (event) => {
  event.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

// Network first for everything; the cache is only a fallback for the shell.
// API responses (lead data) are never cached.
self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  if (event.request.method !== 'GET' || url.origin !== location.origin) return;
  if (url.pathname.startsWith('/api/') || url.pathname === '/webhook') return;
    event.respondWith(
    fetch(event.request)
      .then((res) => {
        // Hashed build assets and the shell; lead data (/api/) never gets here.
        if (res.ok) {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(event.request, copy));
        }
        return res;
      })
      .catch(() => caches.match(event.request).then((r) => r ?? caches.match('/'))),
  );
});

self.addEventListener('push', (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    data = { title: 'ליד חדש', body: event.data?.text() };
  }
  const title = data.title || 'ליד חדש';
  event.waitUntil(
    Promise.all([
      self.registration.showNotification(title, {
        body: data.body || '',
        tag: data.tag,
        renotify: Boolean(data.tag),
        icon: '/icons/logo-192.png',
        badge: '/icons/badge-96.png',
        data: { url: data.url || '/' },
      }),
      // Let open windows refresh their list.
      self.clients
        .matchAll({ type: 'window' })
        .then((cs) => cs.forEach((c) => c.postMessage({ type: 'new-lead', leadId: data.leadId }))),
    ]),
  );
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const url = new URL(event.notification.data?.url || '/', self.location.origin).href;
  event.waitUntil(
    self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then(async (clients) => {
      const client = clients.find((c) => new URL(c.url).origin === self.location.origin);
      if (client) {
        await client.focus();
        if ('navigate' in client) return client.navigate(url);
        return client.postMessage({ type: 'open', url: new URL(url).pathname });
      }
      return self.clients.openWindow(url);
    }),
  );
});
