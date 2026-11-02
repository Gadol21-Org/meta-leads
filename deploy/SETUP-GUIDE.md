# Setup guide: Meta credentials, GitHub secrets, first deploy

Do the steps in this order. The Meta webhook can only be saved once the app is deployed and
`/webhook` is reachable, so the first deploy happens before the Meta webhook setup.

## 1. Values you make up or generate yourself

| Value | How |
|---|---|
| `META_VERIFY_TOKEN` | Any long random string, e.g. `openssl rand -hex 24`. You paste the same string into Meta in step 4. |
| `VAPID_PUBLIC_KEY`, `VAPID_PRIVATE_KEY` | Web Push keys. Run `cargo run -p meta-leads-backend -- vapid` in this repo (needs Rust), or `docker run --rm --entrypoint meta-leads-backend <image> vapid` once an image exists. Keep the private key secret; generate them once and reuse them, because changing them drops every subscribed device. |
| `VAPID_SUBJECT` | `mailto:you@example.com` (a contact address for push services). |

## 2. Meta app and `META_APP_SECRET`

1. Go to [developers.facebook.com/apps](https://developers.facebook.com/apps) > **Create app**, attach the
   Business portfolio that owns the Facebook Page, and pick the use case **Capture & manage ad leads with
   Marketing API** (the name may differ slightly; it is the lead-ads one). Do **not** stop at the generic
   **Other** use case: the lead permissions below are only available through a lead-ads use case, and
   without them the Lead Ads Testing Tool says "required permissions are missing on the app".
2. **Add the permissions to the app.** App dashboard > **Use cases** > the lead-ads use case >
   **Customize** (or **Permissions and features**) and click **Add** next to each of:
   `leads_retrieval`, `pages_manage_ads`, `pages_read_engagement`, `pages_show_list` and
   `pages_manage_metadata`. While the app is in Development mode and you are an admin, developer or tester
   of the app, these work without App Review ("Standard access").
3. **App settings > Basic**: copy **App secret** (click Show) into `META_APP_SECRET`.
   Also fill in a **Privacy Policy URL**; Meta requires one before the app can go Live (step 6).

**See which permissions the app has right now:** App dashboard > **App Review > Permissions and features**
lists every permission with its access level (a permission that is not listed or says it must be added is
missing). For a token, open `https://graph.facebook.com/v21.0/me/permissions?access_token=<TOKEN>` or paste it
into the [Access Token Debugger](https://developers.facebook.com/tools/debug/accesstoken/) and read `scopes`.

## 3. GitHub settings and first deploy

In the repo: **Settings > Secrets and variables > Actions** (the `budget-tracking` repo already has
the VM values; if you want one copy, create them as organisation secrets instead and share them).

| Kind | Name | What goes in it |
|---|---|---|
| Secret | `ORACLE_SSH_KEY` | The private deploy key, the same one `budget-tracking` uses (`-----BEGIN OPENSSH PRIVATE KEY-----` block). |
| Secret | `ORACLE_KNOWN_HOSTS` | Optional: output of `ssh-keyscan <vm-ip>`. |
| Secret | `LEADS_ENV` | The app's settings, one `NAME=value` per line, no quotes. Contents below. |
| Variable | `ORACLE_HOST` | The VM's public IP. Deploys are skipped until it is set. |
| Variable | `ORACLE_USER` | Optional, default `ubuntu`. |
| Variable | `BUILD_RUNNER` | Optional, default `ubuntu-24.04-arm`; `ubuntu-24.04` for an AMD VM. |

`LEADS_ENV` for the first deploy (no page token yet):

```
META_APP_SECRET=<from step 2>
META_VERIFY_TOKEN=<from step 1>
VAPID_PUBLIC_KEY=<from step 1>
VAPID_PRIVATE_KEY=<from step 1>
VAPID_SUBJECT=mailto:you@example.com
```

Then do the tunnel and Cloudflare Access setup from [README.md](README.md) (public hostname to
`localhost:8084`, Access on the hostname, **Bypass** for the exact path `/webhook`) and run
**Actions > Deploy > Run workflow**. Check `https://<your-host>/webhook` in a browser: it must show
an empty 403 page (not a Cloudflare login page), which means Meta can reach it.

## 4. Webhook subscription

1. In the Meta app: **Add product > Webhooks**.
2. Choose the **Page** object > **Subscribe to this object**. Callback URL
   `https://<your-host>/webhook`, verify token = your `META_VERIFY_TOKEN`. Meta calls the URL
   right away; it saves only if the handshake succeeds.
3. In the Page object's field list, **subscribe to `leadgen`**.

## 5. Page access token and `META_PAGE_ID`

1. Open the [Graph API Explorer](https://developers.facebook.com/tools/explorer/), pick your app,
   **User or Page** token. Add the permissions `pages_show_list`, `pages_read_engagement`,
   `pages_manage_metadata`, `pages_manage_ads` and `leads_retrieval`, click **Generate Access
   Token**, and approve the dialog (select your Page). You must be an admin of the Page, and
   have "Leads access" for it (Page settings > Leads Access, or Business Suite).
2. Make it long-lived. Using that short-lived token (`SHORT`), your App ID and the app secret:
   ```
   GET https://graph.facebook.com/v21.0/oauth/access_token
       ?grant_type=fb_exchange_token&client_id=<APP_ID>&client_secret=<APP_SECRET>&fb_exchange_token=<SHORT>
   ```
   The `access_token` it returns (`LONG_USER`) lasts about 60 days.
3. Get the page token: `GET https://graph.facebook.com/v21.0/me/accounts?access_token=<LONG_USER>`.
   Find your Page in the list: its `id` is **`META_PAGE_ID`** and its `access_token` is
   **`META_PAGE_ACCESS_TOKEN`**. A page token obtained from a long-lived user token does not
   expire. Confirm in the [Access Token Debugger](https://developers.facebook.com/tools/debug/accesstoken/)
   (Expires: Never).
4. Add both to the `LEADS_ENV` secret:
   ```
   META_PAGE_ID=<page id>
   META_PAGE_ACCESS_TOKEN=<page token>
   ```
   then re-run **Actions > Deploy > Run workflow** (the container only reads the settings at deploy).
5. **Subscribing the app to the Page is automatic.** On every start, when both values above are set, the
   app subscribes itself to the Page's `leadgen` field (Meta treats a repeat as a no-op) and logs
   `Page <id>: subscribed to leadgen webhooks` in `docker logs meta-leads`, or a warning if Meta refused
   (usually a token without `pages_manage_metadata`). Without this subscription no leads are delivered.
   Doing it by hand is optional, for example to test the token before deploying:
   ```
   POST https://graph.facebook.com/v21.0/<PAGE_ID>/subscribed_apps
        ?subscribed_fields=leadgen&access_token=<PAGE_TOKEN>
   ```
   It returns `{"success": true}`.

Treat the app secret and page token like passwords. An alternative that avoids a personal user
token is a **system user** (Business settings > Users > System users) with the Page assigned and a
token generated with the same permissions.

## 6. Testing, then real leads

* Test now, in Development mode: [Lead Ads Testing Tool](https://developers.facebook.com/tools/lead-ads-testing),
  pick the Page and form, **Create lead**. It should appear in the app and as a notification
  within seconds.
* **Real leads need the app in Live mode.** In Development mode Meta only sends webhooks for leads
  created by people with a role on the app (the testing tool, or you). Switch **App mode** to
  **Live** (top bar of the app dashboard; needs the privacy policy URL from step 2).
* For an app that reads leads from a Page owned by your own business, Standard Access to the
  permissions normally works once the app is Live and connected to that business. If real leads
  still don't arrive, or Graph returns a permissions error, go to **App Review > Permissions and
  features** and request **Advanced Access** for `leads_retrieval`, `pages_manage_metadata`,
  `pages_read_engagement` and `pages_show_list` (this can need Business Verification and a short
  screencast of the use).
* Also make sure the app is allowed to access the Page's leads: Business Suite > Settings >
  Leads Access > **CRM integrations**, assign your app.

## 7. Phones

Open `https://<your-host>`, pass the Cloudflare Access login, tap **Enable notifications**.
On iPhone, first Share > **Add to Home Screen**, then open the app from the home screen.

## Troubleshooting: a test lead did not arrive

Work through these in order; the first one that fails is the cause. The server log (`docker logs meta-leads`)
now records every step, so you can tell them apart.

1. **The Lead Ads Testing Tool says "required permissions are missing on the app".** The app lacks lead
   permissions; see step 2 of section 2 and add them to the app's lead-ads use case. Also check that you are
   logged in to Facebook as an **admin of the Page** and have a **role on the app** (App roles).
2. **Is the webhook saved in Meta?** App dashboard > **Webhooks** > **Page**: callback URL and verify token are
   shown, and the `leadgen` row has **Subscribe** switched on. The log has
   `webhook verification handshake accepted` from when you saved it.
3. **Is the Page subscribed?** The startup log says `Page <id>: subscribed to leadgen webhooks`.
4. **Does Meta reach the server?** In the dashboard Webhooks > Page > `leadgen` row click **Test** (sends a
   sample event). The log must show `webhook POST received`.
   * Nothing logged: the request never arrived. Check the Cloudflare Access bypass for `/webhook` (it must
     cover **POST**, not just GET) and Cloudflare **Security > Events** / Zero Trust **Logs > Access** for blocks
     (bot protection or WAF rules can block Meta's servers).
   * `webhook POST rejected: the signature does not match META_APP_SECRET`: the secret in `LEADS_ENV` is wrong
     (use App settings > Basic > App secret of **this** app; no quotes or spaces) or the deploy did not pick it up.
5. **Is the app in Development mode?** In Development mode Meta delivers webhooks only for leads created by
   people with a role on the app. A test lead from the Testing Tool by an admin works; real leads from the
   public need the app set to **Live** (section 6).
6. **Did the app store the lead?** Log lines: `webhook POST accepted: 1 leadgen event(s)` then
   `lead <id> stored (...)`. `lead <id> ignored: it is for Page ...` means `META_PAGE_ID` differs from the
   Page the lead came from. A stored lead with "details could not be read" means the page token or its
   `leads_retrieval` permission is wrong (the error text is in the log and on the lead page).
7. **Did the phone get the notification?** The log says `lead <id> not notified: no device has enabled
   notifications yet` if no phone subscribed (open the app on the phone and tap **Enable notifications**; on
   iPhone add it to the Home Screen first) or `VAPID keys are not configured`.
