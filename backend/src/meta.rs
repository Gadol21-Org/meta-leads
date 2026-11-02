//! Everything that talks to Meta: webhook signature check, the Graph API lead
//! lookup and the backfill import. Read-only: nothing here writes to Meta.
//!
//! Meta's JSON is parsed into typed structs at the edge, so the rest of the
//! backend never touches untyped `serde_json::Value`s.

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Deserializer};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::store::{Disclaimer, Field, Lead, LeadSource};

pub const FALLBACK_TITLE: &str = "ליד חדש";

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

/// Meta sends ids as strings, but sometimes as bare numbers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum RawId {
    Text(String),
    Number(u64),
}

impl fmt::Display for RawId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RawId::Text(s) => f.write_str(s),
            RawId::Number(n) => write!(f, "{n}"),
        }
    }
}

fn id_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    RawId::deserialize(d).map(|id| id.to_string())
}

fn opt_id_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<RawId>::deserialize(d)?.map(|id| id.to_string()))
}

// ---------- webhook payload ----------

#[derive(Debug, Deserialize)]
pub struct WebhookPayload {
    pub object: String,
    #[serde(default)]
    entry: Vec<WebhookEntry>,
}

#[derive(Debug, Deserialize)]
struct WebhookEntry {
    #[serde(deserialize_with = "id_string")]
    id: String,
    #[serde(default)]
    changes: Vec<WebhookChange>,
}

#[derive(Debug, Deserialize)]
struct WebhookChange {
    field: String,
    /// Shape depends on `field`, so it is only parsed for `leadgen`.
    value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct LeadgenValue {
    #[serde(deserialize_with = "id_string")]
    leadgen_id: String,
    #[serde(default, deserialize_with = "opt_id_string")]
    page_id: Option<String>,
    #[serde(default, deserialize_with = "opt_id_string")]
    form_id: Option<String>,
    #[serde(default, deserialize_with = "opt_id_string")]
    ad_id: Option<String>,
    #[serde(default, deserialize_with = "opt_id_string")]
    adgroup_id: Option<String>,
    /// Unix seconds.
    created_time: Option<i64>,
}

/// A `leadgen` change from a page webhook payload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeadgenEvent {
    pub leadgen_id: String,
    pub page_id: String,
    pub form_id: Option<String>,
    pub ad_id: Option<String>,
    pub adgroup_id: Option<String>,
    pub created_time: Option<DateTime<Utc>>,
}

impl WebhookPayload {
    /// The `leadgen` events in this payload; anything else is ignored.
    pub fn leadgen_events(&self) -> Vec<LeadgenEvent> {
        if self.object != "page" {
            return Vec::new();
        }
        self.entry
            .iter()
            .flat_map(|entry| entry.changes.iter().map(move |c| (entry, c)))
            .filter(|(_, change)| change.field == "leadgen")
            .filter_map(|(entry, change)| {
                let v: LeadgenValue = serde_json::from_value(change.value.clone()).ok()?;
                Some(LeadgenEvent {
                    leadgen_id: v.leadgen_id,
                    page_id: v.page_id.unwrap_or_else(|| entry.id.clone()),
                    form_id: v.form_id,
                    ad_id: v.ad_id,
                    adgroup_id: v.adgroup_id,
                    created_time: v.created_time.and_then(|s| DateTime::from_timestamp(s, 0)),
                })
            })
            .collect()
    }
}

// ---------- Graph API responses ----------

/// Graph times look like `2026-11-02T08:00:00+0000` (no colon in the offset).
fn graph_time<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
    let Some(raw) = Option::<String>::deserialize(d)? else {
        return Ok(None);
    };
    Ok(DateTime::parse_from_str(&raw, "%Y-%m-%dT%H:%M:%S%z")
        .or_else(|_| DateTime::parse_from_rfc3339(&raw))
        .ok()
        .map(|t| t.with_timezone(&Utc)))
}

#[derive(Debug, Deserialize)]
struct GraphField {
    name: String,
    #[serde(default)]
    values: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GraphDisclaimer {
    checkbox_key: Option<String>,
    is_checked: Option<CheckedFlag>,
}

/// The API sends `"1"` or `true` depending on the field.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CheckedFlag {
    Bool(bool),
    Text(String),
}

impl CheckedFlag {
    fn is_checked(&self) -> bool {
        match self {
            CheckedFlag::Bool(b) => *b,
            CheckedFlag::Text(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GraphLead {
    #[serde(deserialize_with = "id_string")]
    pub id: String,
    #[serde(default, deserialize_with = "graph_time")]
    created_time: Option<DateTime<Utc>>,
    #[serde(default)]
    field_data: Vec<GraphField>,
    form_id: Option<String>,
    ad_id: Option<String>,
    ad_name: Option<String>,
    adset_id: Option<String>,
    adset_name: Option<String>,
    campaign_id: Option<String>,
    campaign_name: Option<String>,
    platform: Option<String>,
    is_organic: Option<bool>,
    partner_name: Option<String>,
    #[serde(default)]
    custom_disclaimer_responses: Vec<GraphDisclaimer>,
}

#[derive(Debug, Deserialize)]
struct GraphForm {
    #[serde(deserialize_with = "id_string")]
    id: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Paging {
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
    paging: Option<Paging>,
}

impl<T> Page<T> {
    fn next_url(&self) -> Option<&str> {
        self.paging.as_ref()?.next.as_deref()
    }
}

#[derive(Debug, Deserialize)]
struct SuccessResponse {
    success: bool,
}

#[derive(Debug, Deserialize)]
struct GraphErrorEnvelope {
    error: GraphErrorBody,
}

#[derive(Debug, Deserialize)]
struct GraphErrorBody {
    message: Option<String>,
    code: Option<i64>,
    error_subcode: Option<i64>,
}

/// A failed Graph API call, with Meta's codes kept so callers can tell "gone" from "failed".
#[derive(Debug)]
pub struct GraphError {
    message: String,
    code: Option<i64>,
    subcode: Option<i64>,
    pub rate_limited: bool,
}

impl GraphError {
    /// Meta's answer for an id that does not exist (or is no longer readable with this token):
    /// code 100, subcode 33.
    fn is_object_missing(&self) -> bool {
        self.code == Some(100) && self.subcode == Some(33)
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GraphError {}

/// Meta error codes that mean "slow down": app (4), user (17), page/feature (32, 613)
/// and Business Use Case (80000 to 80014) call limits.
fn is_rate_limit_code(code: i64) -> bool {
    matches!(code, 4 | 17 | 32 | 613) || (80000..=80014).contains(&code)
}

const MAX_ATTEMPTS: u32 = 3;
const BASE_DELAY: Duration = Duration::from_secs(2);
const MAX_DELAY: Duration = Duration::from_secs(30);
/// Usage (percent of the hourly allowance) at which a warning is logged.
const USAGE_WARN_PERCENT: u64 = 75;

/// Exponential backoff for the retry after `attempt` (1-based), capped at `MAX_DELAY`.
fn backoff_delay(attempt: u32) -> Duration {
    BASE_DELAY
        .saturating_mul(1 << (attempt - 1).min(8))
        .min(MAX_DELAY)
}

/// The highest percentage found in a usage header such as
/// `{"call_count":28,"total_time":25,"total_cputime":25}` or the nested
/// `X-Business-Use-Case-Usage` form (`{"<id>":[{"call_count":3,...}]}`).
fn max_usage_percent(value: &serde_json::Value) -> u64 {
    match value {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0),
        serde_json::Value::Object(map) => map
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "type" | "estimated_time_to_regain_access"))
            .map(|(_, v)| max_usage_percent(v))
            .max()
            .unwrap_or(0),
        serde_json::Value::Array(items) => items.iter().map(max_usage_percent).max().unwrap_or(0),
        _ => 0,
    }
}

fn log_usage(headers: &reqwest::header::HeaderMap) {
    for name in ["x-app-usage", "x-business-use-case-usage"] {
        let Some(value) = headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        else {
            continue;
        };
        let percent = max_usage_percent(&value);
        if percent >= USAGE_WARN_PERCENT {
            tracing::warn!("Graph API usage is at {percent}% of the hourly limit ({name})");
        }
    }
}

// ---------- normalising ----------

fn humanise(name: &str) -> String {
    let spaced = name
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
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
    let full_name = [get(&["first_name"]), get(&["last_name"])]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    [
        get(&["full_name", "name"]),
        full_name,
        get(&["email"]),
        get(&["phone_number", "phone"]),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .unwrap_or_else(|| FALLBACK_TITLE.to_string())
}

impl GraphLead {
    /// Builds a stored lead, using `fallback_time` if Meta sent no usable time.
    pub fn into_lead(self, source: LeadSource, fallback_time: DateTime<Utc>) -> Lead {
        let fields: Vec<Field> = self
            .field_data
            .into_iter()
            .map(|f| Field {
                label: humanise(&f.name),
                value: f.values.join(", "),
                name: f.name,
            })
            .collect();
        Lead {
            title: lead_title(&fields),
            fields,
            form_id: self.form_id,
            ad_id: self.ad_id,
            ad_name: self.ad_name,
            adset_id: self.adset_id,
            adset_name: self.adset_name,
            campaign_id: self.campaign_id,
            campaign_name: self.campaign_name,
            platform: self.platform,
            is_organic: self.is_organic,
            partner_name: self.partner_name,
            disclaimers: self
                .custom_disclaimer_responses
                .into_iter()
                .map(|d| Disclaimer {
                    key: d.checkbox_key.unwrap_or_default(),
                    checked: d.is_checked.is_some_and(|c| c.is_checked()),
                })
                .collect(),
            ..Lead::bare(self.id, self.created_time.unwrap_or(fallback_time), source)
        }
    }
}

// ---------- Graph client ----------

#[derive(Clone)]
pub struct Graph {
    base: String,
    token: String,
    http: reqwest::Client,
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

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> anyhow::Result<T> {
        let url = format!("{}/{}", self.base, path);
        self.send(
            self.http
                .get(url)
                .query(params)
                .query(&[("access_token", &self.token)]),
        )
        .await
    }

    /// Sends the request, retrying only rate-limit errors (at most `MAX_ATTEMPTS` tries, with
    /// capped exponential backoff, or Meta's `Retry-After` when given). Other errors fail at once.
    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
    ) -> anyhow::Result<T> {
        let mut attempt = 1;
        loop {
            let this_try = req
                .try_clone()
                .ok_or_else(|| anyhow::anyhow!("Graph API: לא ניתן לנסות שוב את הבקשה"))?;
            let res = this_try.send().await?;
            let status = res.status();
            log_usage(res.headers());
            let retry_after = res
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            let body = res.bytes().await?;
            if status.is_success() {
                return serde_json::from_slice(&body)
                    .map_err(|e| anyhow::anyhow!("Graph API: תגובה לא צפויה: {e}"));
            }
            let error = serde_json::from_slice::<GraphErrorEnvelope>(&body)
                .ok()
                .map(|e| e.error);
            let code = error.as_ref().and_then(|e| e.code);
            let subcode = error.as_ref().and_then(|e| e.error_subcode);
            let rate_limited = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || code.is_some_and(is_rate_limit_code);
            let msg = error
                .and_then(|e| e.message)
                .unwrap_or_else(|| format!("HTTP {status}"));
            if rate_limited && attempt < MAX_ATTEMPTS {
                let delay = retry_after
                    .unwrap_or_else(|| backoff_delay(attempt))
                    .min(MAX_DELAY);
                tracing::warn!(
                    "Graph API rate limit ({msg}); retry {attempt}/{} in {delay:?}",
                    MAX_ATTEMPTS - 1
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }
            let message = if rate_limited {
                format!("הגעתם למגבלת הקריאות של Graph API, נסו שוב מאוחר יותר: {msg}")
            } else {
                format!("Graph API: {msg}")
            };
            return Err(GraphError {
                message,
                code,
                subcode,
                rate_limited,
            }
            .into());
        }
    }

    /// Subscribes the app that issued the page token to the Page's `leadgen` webhook field.
    /// Idempotent: Meta answers success whether or not it was already subscribed.
    pub async fn subscribe_leadgen(&self, page_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/{page_id}/subscribed_apps", self.base);
        let req = self.http.post(url).query(&[
            ("subscribed_fields", "leadgen"),
            ("access_token", &self.token),
        ]);
        let res: SuccessResponse = self.send(req).await?;
        anyhow::ensure!(res.success, "Graph API: החיבור של האפליקציה לעמוד נדחה");
        Ok(())
    }

    /// Full lead record: every form answer plus ad/campaign context.
    pub async fn fetch_lead(&self, leadgen_id: &str) -> anyhow::Result<GraphLead> {
        self.get(leadgen_id, &[("fields", LEAD_FIELDS)]).await
    }

    pub async fn fetch_form_name(&self, form_id: &str) -> Option<String> {
        self.get::<GraphForm>(form_id, &[("fields", "id,name")])
            .await
            .ok()?
            .name
    }

    /// Pages of a paginated list, following `paging.next` (which already carries the token), up to
    /// `limit` items. The flag says whether the list was read to its end, which matters because
    /// a lead missing from a truncated list proves nothing.
    async fn collect_pages<T: serde::de::DeserializeOwned>(
        &self,
        first: Page<T>,
        limit: usize,
    ) -> anyhow::Result<(Vec<T>, bool)> {
        let mut items = Vec::new();
        let mut page = first;
        let complete = loop {
            let next = page.next_url().map(str::to_string);
            items.extend(page.data);
            match next {
                None => break items.len() <= limit,
                Some(_) if items.len() >= limit => break false,
                Some(url) => page = self.send(self.http.get(url)).await?,
            }
        };
        items.truncate(limit);
        Ok((items, complete))
    }

    /// Backfill and reconciliation source: up to `max_per_form` leads from every form on the page.
    /// A form that fails is reported as such (never as "has no leads"); a rate limit aborts
    /// the whole import, since the remaining calls would fail too.
    pub async fn all_leads(
        &self,
        page_id: &str,
        max_per_form: usize,
    ) -> anyhow::Result<Vec<FormFetch>> {
        let forms: Page<GraphForm> = self
            .get(
                &format!("{page_id}/leadgen_forms"),
                &[("fields", "id,name"), ("limit", "100")],
            )
            .await?;
        let (forms, _) = self.collect_pages(forms, 1000).await?;
        let mut out = Vec::new();
        for form in forms {
            match self.form_leads(&form, max_per_form).await {
                Ok((leads, complete)) => out.push(FormFetch::Read(FormLeads {
                    form_id: form.id,
                    form_name: form.name,
                    leads,
                    complete,
                })),
                Err(e)
                    if e.downcast_ref::<GraphError>()
                        .is_some_and(|g| g.rate_limited) =>
                {
                    return Err(e)
                }
                Err(e) => {
                    tracing::warn!("import: form {} failed: {e}", form.id);
                    out.push(FormFetch::Failed {
                        form_id: form.id,
                        error: e.to_string(),
                    });
                }
            }
        }
        Ok(out)
    }

    async fn form_leads(
        &self,
        form: &GraphForm,
        max: usize,
    ) -> anyhow::Result<(Vec<GraphLead>, bool)> {
        let first: Page<GraphLead> = self
            .get(
                &format!("{}/leads", form.id),
                &[("fields", LEAD_FIELDS), ("limit", "100")],
            )
            .await?;
        self.collect_pages(first, max).await
    }

    /// Asks Meta about one lead by id, to confirm it is really gone before it is removed locally.
    pub async fn lookup_lead(&self, id: &str) -> LeadLookup {
        match self.fetch_lead(id).await {
            Ok(lead) => LeadLookup::Found(Box::new(lead)),
            Err(e)
                if e.downcast_ref::<GraphError>()
                    .is_some_and(GraphError::is_object_missing) =>
            {
                LeadLookup::Gone
            }
            Err(e) => {
                tracing::warn!("import: could not verify lead {id}: {e}");
                LeadLookup::Unknown
            }
        }
    }
}

/// What reading one form's leads gave.
pub enum FormFetch {
    Read(FormLeads),
    Failed { form_id: String, error: String },
}

pub struct FormLeads {
    pub form_id: String,
    pub form_name: Option<String>,
    pub leads: Vec<GraphLead>,
    /// False when the list was cut at the cap: absent leads are then not evidence of deletion.
    pub complete: bool,
}

pub enum LeadLookup {
    Found(Box<GraphLead>),
    /// Meta says the lead does not exist (any more).
    Gone,
    /// The check failed for another reason (network, permissions, rate limit): keep the lead.
    Unknown,
}

#[cfg(test)]
mod rate_limit_tests {
    use super::*;

    #[test]
    fn recognises_rate_limit_codes() {
        for code in [4, 17, 32, 613, 80001, 80004] {
            assert!(is_rate_limit_code(code), "{code}");
        }
        for code in [1, 100, 190, 200] {
            assert!(!is_rate_limit_code(code), "{code}");
        }
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(2), Duration::from_secs(4));
        assert_eq!(backoff_delay(20), MAX_DELAY);
    }

    #[test]
    fn reads_usage_from_both_header_shapes() {
        let app: serde_json::Value =
            serde_json::from_str(r#"{"call_count":28,"total_time":80,"total_cputime":25}"#)
                .unwrap();
        assert_eq!(max_usage_percent(&app), 80);
        let buc: serde_json::Value = serde_json::from_str(
            r#"{"123":[{"type":"ads_management","call_count":91,"estimated_time_to_regain_access":0}]}"#,
        )
        .unwrap();
        assert_eq!(max_usage_percent(&buc), 91);
    }
}
