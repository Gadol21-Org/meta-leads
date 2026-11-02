import type { AppConfig, Lead, LeadSummary } from "./types";

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
    throw new ApiError("Session expired. Reload the page to sign in again.", res.status || 401);
  }
  const json = (await res.json().catch(() => ({}))) as { error?: string } & T;
  if (!res.ok) throw new ApiError(json.error ?? `HTTP ${res.status}`, res.status);
  return json;
}

export const getConfig = () => request<AppConfig>("/api/config");
export const listLeads = () => request<{ leads: LeadSummary[] }>("/api/leads").then((r) => r.leads);
export const getLead = (id: string, refresh = false) =>
  request<{ lead: Lead }>(`/api/leads/${encodeURIComponent(id)}${refresh ? "?refresh=1" : ""}`).then((r) => r.lead);
export const syncLeads = () => request<{ added: number; errors: string[] }>("/api/sync", { method: "POST" });
export const sendTestPush = () => request<{ sent: number; failed: number }>("/api/test-notification", { method: "POST" });
export const saveSubscription = (sub: PushSubscriptionJSON) =>
  request<{ ok: boolean }>("/api/subscriptions", { method: "POST", body: sub });
export const deleteSubscription = (endpoint: string) =>
  request<{ ok: boolean }>("/api/subscriptions", { method: "DELETE", body: { endpoint } });
