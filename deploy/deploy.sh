#!/usr/bin/env bash
# Runs on the VM: replaces the running meta-leads container with the given image.
# Published on 127.0.0.1:8084 of the VM only; the cloudflared service forwards
# the public hostname to that port and keeps running across deploys.
# Usage: deploy.sh <image>
set -euo pipefail

image="$1"
dir="$HOME/meta-leads"
env_file="$dir/.env"
name=meta-leads
host_port=8084

if [ ! -f "$env_file" ]; then
  echo "$env_file is missing; the workflow writes it from the LEADS_ENV secret" >&2
  exit 1
fi

# Pull before stopping anything, so the app is down only for the swap.
docker pull "$image"

docker rm -f "$name" >/dev/null 2>&1 || true
docker run -d --name "$name" \
  --init \
  --restart unless-stopped \
  --log-opt max-size=10m --log-opt max-file=3 \
  -p "127.0.0.1:$host_port:3000" \
  -v meta-leads-data:/data \
  --env-file "$env_file" \
  "$image"

# Catch an app that exits at once and restarts in a loop (bad env, say).
sleep 10
if [ "$(docker inspect -f '{{.State.Running}} {{.RestartCount}}' "$name")" != "true 0" ]; then
  echo "$name did not stay up:" >&2
  docker logs --tail 50 "$name" >&2
  exit 1
fi
docker logs --tail 10 "$name"

systemctl is-active --quiet cloudflared 2>/dev/null \
  || echo "warning: the cloudflared service is not running on this VM" >&2

docker image prune -f >/dev/null
