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

* `src/server.js`: Express app with the Meta webhook, login, read-only API and the static PWA.
* `src/meta.js`: webhook signature check, Graph API lead lookup, backfill import.
* `src/push.js`: Web Push fan-out; expired subscriptions are removed automatically.
* `src/store.js`: JSON file store (single instance). Swap for a database if you scale out.
* `public/`: the PWA (`app.js`, service worker `sw.js`, manifest and icons).

## Setup

1. **Install and configure**
   ```sh
   npm install
   cp .env.example .env
   npm run vapid          # paste the two keys into .env
   ```
   Fill in `APP_PASSWORD`, `SESSION_SECRET` and the `META_*` values (below).

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

3. **Run** (HTTPS is required for push and for Meta webhooks)
   ```sh
   node --env-file=.env src/index.js
   ```
   or with Docker:
   `docker build -t meta-leads . && docker run --env-file .env -p 3000:3000 -v $PWD/data:/app/data meta-leads`

4. **On each phone**: open the site, sign in, tap **Enable notifications**.
   On iPhone (iOS 16.4+), first use Share > **Add to Home Screen**, open the
   app from the home screen, then enable notifications. Apple only allows web
   push for installed web apps.

The ⋯ menu has **Import existing leads from Meta** (reads past leads from all
the page's forms without notifying), **Send test notification**, and log out.

## Local testing

```sh
npm test
META_APP_SECRET=... node scripts/simulate-lead.js http://localhost:3000
```

`simulate-lead.js` posts a correctly signed fake webhook. Without a real lead
id the Graph lookup fails, so the lead is shown with only its id and a Retry button.

## Security notes

* Webhook POSTs are rejected unless `X-Hub-Signature-256` matches `META_APP_SECRET`.
* The app and API are behind `APP_PASSWORD` (HttpOnly signed cookie, 180 days).
* Lead data is never cached by the service worker; only the app shell is.
