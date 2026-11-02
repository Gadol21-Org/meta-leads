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

1. Go to [developers.facebook.com/apps](https://developers.facebook.com/apps) > **Create app**.
   Choose the use case **Other** and the app type **Business**, and attach your Business portfolio
   (the one that owns the Facebook Page).
2. **App settings > Basic**: copy **App secret** (click Show) into `META_APP_SECRET`.
   Also fill in a **Privacy Policy URL**; Meta requires one before the app can go Live (step 6).

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

## 4. Webhook subscription (click path)

Meta's dashboard changes its layout from time to time; if a name below differs, look for the same word.

1. Go to [developers.facebook.com/apps](https://developers.facebook.com/apps) and open your app.
2. In the left menu open **Webhooks**. If you don't see it: **Add product** (or **Use cases**) > **Webhooks** > **Set up**.
3. In the dropdown at the top of the Webhooks page (it says *User* by default) choose **Page**, then click **Subscribe to this object**.
4. In the dialog enter:
   * **Callback URL**: `https://<your-host>/webhook`
   * **Verify token**: exactly the value of `META_VERIFY_TOKEN`
   Click **Verify and save**. Meta now calls your server, so the app must already be deployed with that token and `/webhook` must bypass Cloudflare Access. If it fails, open `https://<your-host>/webhook` in a browser: an empty 403 page is correct (the server is reachable); a Cloudflare login page means the bypass rule is missing.
5. In the table of Page fields, find the **`leadgen`** row and switch its **Subscribe** toggle on. (Subscribing the *object* alone is not enough; the field must be on.)

## 5. Page access token and `META_PAGE_ID`

You get these from the [Graph API Explorer](https://developers.facebook.com/tools/explorer/); the token is not shown anywhere in the app dashboard.

1. In the Explorer, top right, pick **Meta App** = your app. Under **User or Page** choose **Get User Access Token**.
2. In **Permissions** add `pages_show_list`, `pages_read_engagement`, `pages_manage_metadata`, `pages_manage_ads` and `leads_retrieval`, click **Generate Access Token** and approve the dialog, selecting your Page. You must be an admin of the Page and have Leads access for it (Page settings > Leads Access, or Business Suite).
3. Make it long-lived. Using that short-lived token (`SHORT`), your **App ID** and the app secret (both on App settings > Basic), open in a browser:
   ```
   https://graph.facebook.com/v21.0/oauth/access_token?grant_type=fb_exchange_token&client_id=<APP_ID>&client_secret=<APP_SECRET>&fb_exchange_token=<SHORT>
   ```
   The `access_token` in the answer (`LONG_USER`) lasts about 60 days.
4. Get the page token: open `https://graph.facebook.com/v21.0/me/accounts?access_token=<LONG_USER>`
   (or paste `me/accounts` in the Explorer with `LONG_USER` as its token). Find your Page in the list: its
   **`id`** is **`META_PAGE_ID`** and its **`access_token`** is **`META_PAGE_ACCESS_TOKEN`**.
   A page token obtained from a long-lived user token does not expire.
5. **Check the token works**:
   * Paste it into the [Access Token Debugger](https://developers.facebook.com/tools/debug/accesstoken/): it should show your app, **Expires: Never**, and `leads_retrieval` in the scopes.
   * `GET https://graph.facebook.com/v21.0/<PAGE_ID>/leadgen_forms?access_token=<PAGE_TOKEN>` should list your lead forms (an empty list is fine if you have none; an error means a missing permission).
6. **Subscribing the app to the Page is automatic.** On every start the app subscribes itself to the
   `leadgen` field of each configured Page (Meta treats a repeat as a no-op), and logs
   `Page <name>: subscribed to leadgen webhooks` (or a warning if Meta refused, usually a missing
   `pages_manage_metadata` permission). You can still do it by hand, for example to test the token before
   deploying:
   ```
   POST https://graph.facebook.com/v21.0/<PAGE_ID>/subscribed_apps?subscribed_fields=leadgen&access_token=<PAGE_TOKEN>
   ```
   It returns `{"success": true}`; `GET <PAGE_ID>/subscribed_apps` lists the subscribed apps.
7. Add both values to the `LEADS_ENV` secret:
   ```
   META_PAGE_ID=<page id>
   META_PAGE_ACCESS_TOKEN=<page token>
   ```
   then re-run **Actions > Deploy > Run workflow** (the container only reads the settings at deploy).

### Does the app need to log in again every 60 days?

No. Only the *user* token from step 3 lasts about 60 days. The **page** token you get from it in step 4
does not expire, and that page token is the only token the app uses. It stops working only if you
change your Facebook password, remove the app, lose admin access to the Page, or revoke it by hand.
The sturdier option is a system-user token (below), which is not tied to your personal account at all.

### `me/accounts` returns an empty `data` list

A token can be valid and still see no Pages. Check in this order:

1. **Which permissions and Pages did the token get?** Open
   `https://graph.facebook.com/v21.0/me/permissions?access_token=<LONG_USER>`. You need
   `pages_show_list` with `status: granted`. If it is missing or `declined`, generate the token again.
2. **Was the Page selected in the permission dialog?** When you click Generate Access Token, Meta shows
   "Choose the Pages you want to use"; if you picked none, the list is empty. Run
   `https://graph.facebook.com/v21.0/debug_token?input_token=<LONG_USER>&access_token=<APP_ID>|<APP_SECRET>`
   and look at `granular_scopes`: `pages_show_list` must have your Page's id in `target_ids`. If it is
   empty, generate the token again and tick the Page (or "Opt in to all current and future Pages").
3. **Is the token from the right app?** `debug_token` shows the `app_id`; it must be your lead app.
4. **Is the Page owned through a Business portfolio?** Then a personal token often lists nothing. Either add
   the `business_management` permission when generating the user token and read
   `<BUSINESS_ID>/owned_pages` instead, or skip user tokens and use a system user (below).
5. **Are you an admin of the Page?** In the Page's settings you need full control (admin), or Business Suite
   access with "Manage Page" for the Page.

You can also skip `me/accounts`: if you already know `<PAGE_ID>` (Page > About > Page transparency, or in
Business Suite settings), call `<PAGE_ID>?fields=access_token&access_token=<LONG_USER>`.

### System user token (recommended, not tied to a person)

1. [business.facebook.com/settings](https://business.facebook.com/settings) > **Users > System users** > **Add**,
   name it `meta-leads`, role **Admin** (or Employee).
2. **Assign assets**: select the system user > **Add assets** > **Pages** > pick your Page > turn on
   **Manage Page** (full control). Also assign your **app** (Apps tab) with full control.
3. **Generate new token**: choose your app, set token expiration to **Never**, tick `pages_show_list`,
   `pages_read_engagement`, `pages_manage_metadata`, `pages_manage_ads` and `leads_retrieval`, and copy it.
4. Get the page token from it: `https://graph.facebook.com/v21.0/<PAGE_ID>?fields=access_token&access_token=<SYSTEM_USER_TOKEN>`.
   Use the returned `access_token` as `META_PAGE_ACCESS_TOKEN` (the system user token itself also works
   for reading leads, but the page token is what step 6 above expects). Your Page id is in the Page's
   settings (Business Suite > Settings > Page > the number under the name).
5. Continue with step 7 (`LEADS_ENV`); the app subscribes itself to the Page.

## Two or more Pages (e.g. a Facebook Page and an Instagram-linked Page)

First, one fact about Instagram: leads from ads that run on Instagram are collected through the
**Facebook Page** that the Instagram account is linked to. An Instagram profile has no leadgen webhook of
its own. So if your Instagram account is linked to the same Facebook Page you already use, you have **one**
Page and nothing extra to configure. You only need the steps below when your ads run from **two different
Facebook Pages** (a Page "behind" each account). The notification and lead page then show which Page a lead
came from.

Do step 4 (webhook) once for the app. The app subscribes itself to every configured Page at startup, so
you only need to give it the tokens, in one of two ways:

* **One token per Page** (simplest if you already have them): in `LEADS_ENV`

  ```
  META_PAGE_TOKENS=<PAGE_ID_1>:<PAGE_TOKEN_1>,<PAGE_ID_2>:<PAGE_TOKEN_2>
  ```

  (no spaces, a colon between id and token, a comma between Pages; use this *instead of* `META_PAGE_ID` and
  `META_PAGE_ACCESS_TOKEN`, though those still work for one Page).
* **One system-user token for all Pages** (recommended): create the system user as described below, assign
  **both** Pages to it with Manage Page, generate one token (expiration Never), and put in `LEADS_ENV`

  ```
  META_USER_TOKEN=<SYSTEM_USER_TOKEN>
  META_PAGE_IDS=<PAGE_ID_1>,<PAGE_ID_2>
  ```

  The app fetches each Page's own token from that one token when it starts, so adding a third Page later
  is: assign it to the system user, add its id to `META_PAGE_IDS`, redeploy.
  (Leave `META_PAGE_IDS` empty to use every Page the system user can see.)

Check the startup log (`docker logs meta-leads`): it prints one line per Page it will read leads from and
whether the subscription worked, and a warning if a Page has no token (usually: the system user isn't
assigned to it with Manage Page).
The ⋯ > Import existing leads action imports from every Page and reports any Page it could not read.

## 6. Testing, then real leads

* Test now, in Development mode: [Lead Ads Testing Tool](https://developers.facebook.com/tools/lead-ads-testing),
  pick the Page and form, **Create lead**. It should appear in the app and as a notification
  within seconds. (You can also use the app's ⋯ menu > **Send test notification** to check push alone.)
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
