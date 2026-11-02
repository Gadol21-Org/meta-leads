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
}

export interface Lead extends LeadSummary {
  fields: Field[];
  form_id?: string;
  ad_id?: string;
  ad_name?: string;
  adset_id?: string;
  adset_name?: string;
  campaign_id?: string;
  is_organic?: boolean;
  partner_name?: string;
  custom_disclaimer_responses?: { checkbox_key?: string; is_checked?: string | boolean }[];
  fetch_error?: string;
  source: string;
  received_at: string;
}

export interface AppConfig {
  vapidPublicKey: string | null;
  pushEnabled: boolean;
  metaConfigured: boolean;
}
