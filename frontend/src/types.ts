export interface Field {
  name: string;
  label: string;
  value: string;
}

export interface LeadSummary {
  id: string;
  title: string;
  created_time: string;
  form_name?: string | null;
  campaign_name?: string | null;
  platform?: string | null;
  /** The ad's name, or its id when the name is unknown. */
  ad?: string | null;
  is_organic?: boolean | null;
}

export interface Disclaimer {
  key: string;
  checked: boolean;
}

export type LeadSource = "webhook" | "sync";

export interface Lead extends LeadSummary {
  fields: Field[];
  form_id?: string;
  ad_id?: string;
  ad_name?: string;
  adset_id?: string;
  adset_name?: string;
  campaign_id?: string;
  partner_name?: string;
  disclaimers?: Disclaimer[];
  fetch_error?: string;
  source: LeadSource;
  received_at: string;
}

export interface AppConfig {
  vapidPublicKey: string | null;
  pushEnabled: boolean;
  metaConfigured: boolean;
}

/** Messages the service worker posts to open pages. */
export type WorkerMessage = { type: "open"; url: string } | { type: "new-lead"; leadId?: string };

export function isWorkerMessage(data: unknown): data is WorkerMessage {
  if (typeof data !== "object" || data === null) return false;
  const m = data as Record<string, unknown>;
  return (m.type === "open" && typeof m.url === "string") || m.type === "new-lead";
}

/** What an import did; mirrors the backend's `Report`. */
export interface ImportReport {
  added: number;
  refreshed: number;
  removed: number;
  skipped_forms: number;
  unverified: number;
}
