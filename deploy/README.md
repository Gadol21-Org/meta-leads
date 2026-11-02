# Deploying to the Oracle Cloud VM

Same pipeline as `budget-tracking`. Every push to `main` runs
[`.github/workflows/deploy.yml`](../.github/workflows/deploy.yml):

1. The [`Dockerfile`](../Dockerfile) builds the app and pushes it to the repo's
   private container registry, `ghcr.io/gadol21-org/meta-leads`, tagged with the commit.
2. GitHub Actions connects to the VM over SSH and runs [`deploy.sh`](deploy.sh), which pulls the
   image and replaces the `meta-leads` container (`--restart unless-stopped`, published on
   `127.0.0.1:8084` of the VM only, data in the `meta-leads-data` Docker volume). The run fails,
   with the last log lines, if the container doesn't stay up for 10 seconds.

Pull requests build the image without pushing or deploying. Deploy by hand from
**Actions > Deploy > Run workflow**. The deploy step is skipped until the `ORACLE_HOST` variable is set.

### Ports on this VM

| Port | Service |
|---|---|
| 8080 | (taken by another service) |
| 8082 | `budget-tracking` |
| 8083 | `budget-staging` |
| 8084 | `meta-leads` (`host_port` in `deploy.sh`) |

## One-time setup

Reuse the VM, Docker, deploy key and `cloudflared` tunnel from `budget-tracking`.

1. **Tunnel hostname.** Add a public hostname to the tunnel, e.g. `leads.example.com`,
   service HTTP, URL `localhost:8084`.
2. **Cloudflare Access.** Zero Trust > Access > Applications > add a self-hosted application for
   `leads.example.com` with an **Allow** policy for your email. The app has no login of its own.
   Then add a second application (or a path rule) for `leads.example.com/webhook` with a
   **Bypass** policy for *Everyone*. Meta's servers can't log in, so the webhook is reachable
   without Access and protected only by the request signature (`X-Hub-Signature-256`, checked
   against `META_APP_SECRET`) and the verify token used in Meta's one-time handshake.
   Make sure nothing else is bypassed: only the exact path `/webhook`.
3. **GitHub settings** (repo > Settings > Secrets and variables > Actions):

| Kind | Name | Value |
|---|---|---|
| Secret | `ORACLE_SSH_KEY` | The private deploy key. |
| Secret | `ORACLE_KNOWN_HOSTS` | Optional: the `ssh-keyscan` output. |
| Secret | `LEADS_ENV` | The app's `.env`, see below. |
| Variable | `ORACLE_HOST` | The VM's public IP. Deploys are skipped until it is set. |
| Variable | `ORACLE_USER` | Optional, default `ubuntu`. |
| Variable | `BUILD_RUNNER` | Optional, default `ubuntu-24.04-arm`. Use `ubuntu-24.04` for an AMD VM. |

`LEADS_ENV` holds one `NAME=value` per line, no quotes (see `.env.example`):

```
META_APP_SECRET=...
META_VERIFY_TOKEN=any-long-random-string
META_PAGE_ID=...
META_PAGE_ACCESS_TOKEN=...
VAPID_PUBLIC_KEY=...      # from `npm run vapid`
VAPID_PRIVATE_KEY=...
VAPID_SUBJECT=mailto:you@example.com
```

To change a setting, update the secret and run the workflow again; restarting the container
doesn't re-read the file.

## Running it

```bash
docker logs -f meta-leads          # the app's logs
journalctl -u cloudflared -f       # the tunnel's logs
```
