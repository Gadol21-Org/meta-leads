import { t } from "./strings";
import type { AppConfig, ImportReport, Lead, LeadSummary } from "./types";

export class ApiError extends Error {
  constructor(message: string, public status: number) {
    super(message);
  }
}

async function request<T>(path: string, init: { method?: string; body?: unknown } = {}): Promise<T> {
  const res = await fetch(path, {
    method: init.method,
    credentials: "same-origin",
    headers: init.body ? { "content-type": "application/json" } : undefined,
    body: init.body ? JSON.stringify(init.body) : undefined,
  });
  // Cloudflare Access answers with a redirect or 401/403 once its session expires.
  if (res.status === 401 || res.status === 403 || res.redirected) {
    throw new ApiError(t.api.sessionExpired, res.status || 401);
  }
  const json = (await res.json().catch(() => ({}))) as { error?: string } & T;
  if (!res.ok) {
    // Our own errors carry Meta's message. Anything else came from a proxy (restart, timeout, crash).
    throw new ApiError(
      json.error ?? t.api.badResponse(res.status),
      res.status,
    );
  }
  return json;
}

export const getConfig = () => request<AppConfig>("/api/config");
export const listLeads = () => request<{ leads: LeadSummary[] }>("/api/leads").then((r) => r.leads);
export const getLead = (id: string, refresh = false) =>
  request<{ lead: Lead }>(`/api/leads/${encodeURIComponent(id)}${refresh ? "?refresh=1" : ""}`).then((r) => r.lead);
export const syncLeads = () => request<ImportReport>("/api/sync", { method: "POST" });
export const saveSubscription = (sub: PushSubscriptionJSON) =>
  request<{ ok: boolean }>("/api/subscriptions", { method: "POST", body: sub });
export const deleteSubscription = (endpoint: string) =>
  request<{ ok: boolean }>("/api/subscriptions", { method: "DELETE", body: { endpoint } });
