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

2. **Meta app, credentials and GitHub secrets**: follow the step-by-step
   [deploy/SETUP-GUIDE.md](deploy/SETUP-GUIDE.md). In short: create a Business app, subscribe the Page object
   to `leadgen` with callback `https://<your-host>/webhook`, generate a never-expiring page token, and put everything
   in the `LEADS_ENV` secret. The app subscribes itself to the Page's `leadgen` field at startup.

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

The ⋯ menu has **Turn off notifications on this device**.

Meta is the source of truth and the local file is a cache. **Pull down to refresh** shows the stored leads at once,
then syncs with Meta (a sync asked for within 30 seconds of the last one reuses its result, so pulling often costs
nothing). The sync reads the leads of every form on the Page (up to 200 per form, without notifying), adds new
ones and refreshes stored ones. A stored lead is removed only when it is missing from that list and Meta also
answers "does not exist" when asked for that one lead by id. Nothing is removed when a form could not be read
completely (error, rate limit, or more than 200 leads), when the lead is older than 80 days (Meta stops
serving old leads; for a lead that came by webhook this counts from when it arrived), when Meta lists nothing for a form
but more than 20 of its leads are stored, or when the check itself failed. A stored lead whose form is not on the Page
at all, such as the made-up sample Meta's dashboard Test button sends, gets the same check by id and is removed
when Meta says it does not exist (at most 20 at once). Meta has no webhook for deleted leads, so a deletion
shows up at the next pull. If Meta cannot be reached the old content stays and a message says why. Nothing is
ever written to Meta. The server log has `import:` lines saying, per form, why leads were kept or removed.

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
| `META_PAGE_ACCESS_TOKEN` | Reading the lead's contents. The webhook only carries ids, so without the token the notification and lead page show just an id. Also used at startup to subscribe the app to the Page (together with `META_PAGE_ID`). **Required for a useful app.** |
| `META_PAGE_ID` | Optional: ignore webhooks for other pages and enable the import. |
