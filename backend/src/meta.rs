//! Everything that talks to Meta: webhook signature check, the Graph API lead
//! lookup and the backfill import. Read-only: nothing here writes to Meta.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::store::{Field, Lead};

const LEAD_FIELDS: &str = "id,created_time,field_data,ad_id,ad_name,adset_id,adset_name,\
campaign_id,campaign_name,form_id,is_organic,platform,partner_name,custom_disclaimer_responses";

/// Checks the `X-Hub-Signature-256` header Meta sends with every webhook POST.
pub fn verify_signature(raw_body: &[u8], header: Option<&str>, app_secret: &str) -> bool {
    let Some(given) = header.and_then(|h| h.strip_prefix("sha256=")) else {
        return false;
    };
    if app_secret.is_empty() {
        return false;
    }
    let Ok(given) = hex::decode(given) else {
        return false;
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()).expect("any key length");
    mac.update(raw_body);
    mac.finalize().into_bytes().as_slice().ct_eq(&given).into()
}

/// A `leadgen` change from a page webhook payload.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeadgenEvent {
    pub leadgen_id: String,
    pub page_id: String,
    pub form_id: Option<String>,
    pub ad_id: Option<String>,
    pub adgroup_id: Option<String>,
    pub created_time: Option<i64>,
}

fn as_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn extract_leadgen_events(payload: &Value) -> Vec<LeadgenEvent> {
    let mut events = Vec::new();
    if payload["object"] != "page" {
        return events;
    }
    for entry in payload["entry"].as_array().into_iter().flatten() {
        for change in entry["changes"].as_array().into_iter().flatten() {
            let value = &change["value"];
            if change["field"] != "leadgen" {
                continue;
            }
            let Some(leadgen_id) = as_string(&value["leadgen_id"]) else {
                continue;
            };
            events.push(LeadgenEvent {
                leadgen_id,
                page_id: as_string(&value["page_id"])
                    .or_else(|| as_string(&entry["id"]))
                    .unwrap_or_default(),
                form_id: as_string(&value["form_id"]),
                ad_id: as_string(&value["ad_id"]),
                adgroup_id: as_string(&value["adgroup_id"]),
                created_time: value["created_time"].as_i64(),
            });
        }
    }
    events
}

/// Webhook times are unix seconds, Graph API times are ISO strings
/// (`2026-11-02T08:00:00+0000`). Always returns RFC 3339 in UTC.
pub fn normalise_time(raw: &Value) -> String {
    let parsed: Option<DateTime<Utc>> = match raw {
        Value::Number(n) => n.as_i64().and_then(|s| Utc.timestamp_opt(s, 0).single()),
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.with_timezone(&Utc))
            .or_else(|| {
                DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z")
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            })
            .or_else(|| {
                s.parse::<i64>()
                    .ok()
                    .and_then(|n| Utc.timestamp_opt(n, 0).single())
            })
            .or_else(|| {
                NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                    .ok()
                    .map(|n| n.and_utc())
            }),
        _ => None,
    };
    parsed
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn humanise(name: &str) -> String {
    let spaced: String = name
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Turns Graph API `field_data` into a flat, display-friendly list.
pub fn normalise_fields(field_data: &Value) -> Vec<Field> {
    field_data
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let name = f["name"].as_str()?.to_string();
            let value = f["values"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Some(Field {
                label: humanise(&name),
                name,
                value,
            })
        })
        .collect()
}

/// A short "who is this" line for notifications and the list view.
pub fn lead_title(fields: &[Field]) -> String {
    let get = |names: &[&str]| {
        fields
            .iter()
            .find(|f| names.contains(&f.name.to_lowercase().as_str()) && !f.value.is_empty())
            .map(|f| f.value.clone())
            .unwrap_or_default()
    };
    let mut name = get(&["full_name", "name"]);
    if name.is_empty() {
        name = [get(&["first_name"]), get(&["last_name"])]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    for fallback in [name, get(&["email"]), get(&["phone_number", "phone"])] {
        if !fallback.is_empty() {
            return fallback;
        }
    }
    "New lead".to_string()
}

fn opt(v: &Value) -> Option<String> {
    as_string(v).filter(|s| !s.is_empty())
}

/// Builds a stored lead from a Graph API lead object.
pub fn lead_from_graph(g: &Value, source: &str) -> Lead {
    let fields = normalise_fields(&g["field_data"]);
    Lead {
        id: as_string(&g["id"]).unwrap_or_default(),
        created_time: normalise_time(&g["created_time"]),
        title: lead_title(&fields),
        fields,
        form_id: opt(&g["form_id"]),
        ad_id: opt(&g["ad_id"]),
        ad_name: opt(&g["ad_name"]),
        adset_id: opt(&g["adset_id"]),
        adset_name: opt(&g["adset_name"]),
        campaign_id: opt(&g["campaign_id"]),
        campaign_name: opt(&g["campaign_name"]),
        platform: opt(&g["platform"]),
        is_organic: g["is_organic"].as_bool(),
        partner_name: opt(&g["partner_name"]),
        custom_disclaimer_responses: g["custom_disclaimer_responses"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        source: source.to_string(),
        received_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ..Lead::default()
    }
}

#[derive(Clone)]
pub struct Graph {
    base: String,
    token: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct GraphError {
    error: Option<GraphErrorBody>,
}
#[derive(Deserialize)]
struct GraphErrorBody {
    message: Option<String>,
}

impl Graph {
    pub fn new(base: &str, token: &str, http: reqwest::Client) -> Graph {
        Graph {
            base: base.to_string(),
            token: token.to_string(),
            http,
        }
    }

    pub fn configured(&self) -> bool {
        !self.token.is_empty()
    }

    async fn get(&self, path: &str, params: &[(&str, &str)]) -> anyhow::Result<Value> {
        let url = format!("{}/{}", self.base, path);
        self.send(
            self.http
                .get(url)
                .query(params)
                .query(&[("access_token", &self.token)]),
        )
        .await
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let res = req.send().await?;
        let status = res.status();
        let body: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() || body.get("error").is_some() {
            let msg = serde_json::from_value::<GraphError>(body)
                .ok()
                .and_then(|e| e.error)
                .and_then(|e| e.message)
                .unwrap_or_else(|| format!("HTTP {status}"));
            anyhow::bail!("Graph API: {msg}");
        }
        Ok(body)
    }

    /// Full lead record: every form answer plus ad/campaign context.
    pub async fn fetch_lead(&self, leadgen_id: &str) -> anyhow::Result<Value> {
        self.get(leadgen_id, &[("fields", LEAD_FIELDS)]).await
    }

    pub async fn fetch_form_name(&self, form_id: &str) -> Option<String> {
        let v = self.get(form_id, &[("fields", "id,name")]).await.ok()?;
        v["name"].as_str().map(str::to_string)
    }

    /// Follows `paging.next` links (which already carry the access token).
    async fn next_page(&self, page: &Value) -> anyhow::Result<Option<Value>> {
        match page["paging"]["next"].as_str() {
            Some(url) => Ok(Some(self.send(self.http.get(url)).await?)),
            None => Ok(None),
        }
    }

    /// Backfill: every lead of every form on the page, newest first per form.
    pub async fn all_leads(
        &self,
        page_id: &str,
        max_per_form: usize,
    ) -> anyhow::Result<Vec<(Value, Option<String>)>> {
        let mut forms = self
            .get(
                &format!("{page_id}/leadgen_forms"),
                &[("fields", "id,name"), ("limit", "100")],
            )
            .await?;
        let mut all_forms: Vec<Value> = Vec::new();
        loop {
            all_forms.extend(forms["data"].as_array().cloned().unwrap_or_default());
            match self.next_page(&forms).await? {
                Some(next) if all_forms.len() < 1000 => forms = next,
                _ => break,
            }
        }
        let mut out = Vec::new();
        for form in all_forms {
            let Some(form_id) = as_string(&form["id"]) else {
                continue;
            };
            let form_name = form["name"].as_str().map(str::to_string);
            let mut seen = 0;
            let mut page = self
                .get(
                    &format!("{form_id}/leads"),
                    &[("fields", LEAD_FIELDS), ("limit", "100")],
                )
                .await?;
            'pages: loop {
                for lead in page["data"].as_array().into_iter().flatten() {
                    if seen >= max_per_form {
                        break 'pages;
                    }
                    seen += 1;
                    out.push((lead.clone(), form_name.clone()));
                }
                match self.next_page(&page).await? {
                    Some(next) => page = next,
                    None => break,
                }
            }
        }
        Ok(out)
    }
}
