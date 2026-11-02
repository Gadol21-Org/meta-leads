# Meta Leads

A small installable web app (PWA) for Meta Lead Ads. Every new lead from your
Facebook/Instagram lead forms triggers an instant push notification on every
phone or desktop that has turned notifications on. Tapping the notification
opens the lead's page with every form answer and the ad/campaign it came from.

**This first version is read-only.** It receives and displays leads and never
writes anything back to Meta.

## How it works

```
Meta lead form ──webhook (leadgen)──▶ /webhook ──Graph API──▶ full lead
                                          │
                                          ├─▶ saved to data/db.json
                                          └─▶ Web Push to every subscribed device
                                                   │
                                     tap ──────────▶ /leads/<id> in the PWA
```

* `backend/` (Rust, axum): `app.rs` webhook + read-only API, `meta.rs` signature check and Graph API,
  `push.rs` Web Push fan-out (expired subscriptions are removed automatically), `store.rs` JSON file store
  (single instance; swap for a database if you scale out). The built frontend is embedded in the binary.
* `frontend/` (TypeScript, Vite): the PWA. `public/sw.js` is the service worker (plain JS, as in budget-tracking).

## Setup

1. **Configure**
   ```sh
   cp .env.example .env
   cargo run -p meta-leads-backend -- vapid     # paste the two keys into .env
   ```
   Fill in the `META_*` values (below).

2. **Meta app** (developers.facebook.com)
   * Create a Business app and add the **Webhooks** product.
   * Subscribe the **Page** object to the `leadgen` field with callback URL
     `https://<your-host>/webhook` and the same verify token as `META_VERIFY_TOKEN`.
   * Subscribe the app to your page:
     `POST /{page-id}/subscribed_apps?subscribed_fields=leadgen` with a page token.
   * Get a long-lived page access token (or a system-user token) with
     `leads_retrieval`, `pages_manage_metadata`, `pages_show_list` and
     `pages_read_engagement`, and put it in `META_PAGE_ACCESS_TOKEN`.
   * In Meta Business Suite > Leads Access, make sure the app is allowed to
     access leads for the page.
   * Use Meta's [Lead Ads Testing Tool](https://developers.facebook.com/tools/lead-ads-testing)
     to send a test lead.

3. **Deploy**: pushes to `main` deploy automatically, see [deploy/README.md](deploy/README.md).
   To run it locally (needs Rust and Node.js; HTTPS is required for push and for Meta webhooks):
   ```sh
   cargo run -p meta-leads-backend       # builds the frontend too, serves on :3000
   ```
   or with Docker:
   `docker build -t meta-leads . && docker run --env-file .env -p 3000:3000 -v meta-leads-data:/data meta-leads`

4. **On each phone**: open the site, tap **Enable notifications**.
   On iPhone (iOS 16.4+), first use Share > **Add to Home Screen**, open the
   app from the home screen, then enable notifications. Apple only allows web
   push for installed web apps.

The ⋯ menu has **Import existing leads from Meta** (reads past leads from all
the page's forms without notifying), and **Send test notification**.

## Local testing

```sh
cargo test                      # backend (fake Graph API and push service)
cd frontend && npm test         # frontend
```

Without a real lead id the Graph lookup fails, so the lead is shown with only its id and a Retry button.

## Security notes

* Webhook POSTs are rejected unless `X-Hub-Signature-256` matches `META_APP_SECRET`.
* The app has no login of its own: put everything except `/webhook` behind Cloudflare Access (see `deploy/README.md`).
* Lead data is never cached by the service worker; only the app shell is.

## Which Meta credentials are needed

| Credential | Needed for |
|---|---|
| `META_VERIFY_TOKEN` | Meta's one-time webhook handshake. You pick any string. **Required.** |
| `META_APP_SECRET` | Checking the signature on every webhook POST. Without it every delivery is rejected. **Required.** |
| `META_PAGE_ACCESS_TOKEN` | Reading the lead's contents. The webhook only carries ids, so without the token the notification and lead page show just an id. Also needed once to subscribe the app to the page. **Required for a useful app.** |
| `META_PAGE_ID` | Optional: ignore webhooks for other pages and enable the import. |
