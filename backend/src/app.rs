use std::{collections::HashSet, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{
    assets,
    config::Config,
    meta::{self, Graph, LeadgenEvent, WebhookPayload},
    push::{BroadcastResult, Payload, Pusher},
    store::{Lead, LeadSource, Store, Subscription},
};

/// What happened when the app tried to subscribe itself to the Page's `leadgen` field.
#[derive(Debug, PartialEq, Eq)]
pub enum SubscribeOutcome {
    /// Nothing to do: no Page id or no page token is configured.
    Skipped,
    Subscribed,
    Failed(String),
}

pub struct AppState {
    pub config: Config,
    pub store: Store,
    pub graph: Graph,
    pub pusher: Pusher,
    /// Lead ids being processed right now, so a Meta retry can't notify twice.
    in_flight: Mutex<HashSet<String>>,
}

impl AppState {
    pub fn new(config: Config, store: Store) -> anyhow::Result<Arc<AppState>> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()?;
        let graph = Graph::new(&config.graph_base, &config.page_access_token, http.clone());
        let pusher = Pusher::new(
            &config.vapid_public_key,
            &config.vapid_private_key,
            &config.vapid_subject,
            http,
        )?;
        Ok(Arc::new(AppState {
            config,
            store,
            graph,
            pusher,
            in_flight: Mutex::new(HashSet::new()),
        }))
    }

    /// Subscribes the app to the Page's `leadgen` field, so the Page never has to be subscribed by
    /// hand. Meta treats a repeat subscription as a no-op, so this is safe on every start. The
    /// result is logged and returned, never fatal.
    pub async fn subscribe_page(&self) -> SubscribeOutcome {
        let page_id = &self.config.page_id;
        if page_id.is_empty() || !self.graph.configured() {
            tracing::info!("not subscribing to leadgen: META_PAGE_ID and META_PAGE_ACCESS_TOKEN are both needed");
            return SubscribeOutcome::Skipped;
        }
        match self.graph.subscribe_leadgen(page_id).await {
            Ok(()) => {
                tracing::info!("Page {page_id}: subscribed to leadgen webhooks");
                SubscribeOutcome::Subscribed
            }
            Err(e) => {
                tracing::warn!(
                    "Page {page_id}: could not subscribe to leadgen webhooks ({e}); subscribe it by hand or give the token pages_manage_metadata"
                );
                SubscribeOutcome::Failed(e.to_string())
            }
        }
    }

    /// The lead as Meta describes it now, merged with what we already know
    /// locally (form name, how and when it first arrived).
    async fn fetch_lead(&self, ev: &LeadgenEvent, source: LeadSource) -> anyhow::Result<Lead> {
        if !self.graph.configured() {
            anyhow::bail!(
                "META_PAGE_ACCESS_TOKEN is not set, so only the webhook data is available."
            );
        }
        let graph_lead = self.graph.fetch_lead(&ev.leadgen_id).await?;
        let mut lead = graph_lead.into_lead(source, ev.created_time.unwrap_or_else(Utc::now));
        lead.form_id = lead.form_id.or_else(|| ev.form_id.clone());
        Ok(lead)
    }

    /// Fetches a lead from Meta and stores it. Returns (lead, is_new).
    async fn ingest(&self, ev: &LeadgenEvent, source: LeadSource) -> anyhow::Result<(Lead, bool)> {
        let existing = self.store.get_lead(&ev.leadgen_id).await;
        let mut lead = match self.fetch_lead(ev, source).await {
            Ok(lead) => lead,
            Err(e) => {
                tracing::error!("could not fetch lead {}: {e}", ev.leadgen_id);
                // Keep the richer data we already had if a refresh fails.
                if let Some(old) = existing.filter(|l| l.fetch_error.is_none()) {
                    return Ok((old, false));
                }
                let mut bare = Lead::bare(
                    ev.leadgen_id.clone(),
                    ev.created_time.unwrap_or_else(Utc::now),
                    source,
                );
                bare.form_id = ev.form_id.clone();
                bare.ad_id = ev.ad_id.clone();
                bare.adset_id = ev.adgroup_id.clone();
                bare.fetch_error = Some(e.to_string());
                bare
            }
        };
        let old = self.store.get_lead(&ev.leadgen_id).await;
        lead.page_id = Some(ev.page_id.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| Some(self.config.page_id.clone()).filter(|s| !s.is_empty()));
        lead.form_name = old.as_ref().and_then(|l| l.form_name.clone());
        if let (None, Some(form_id)) = (&lead.form_name, &lead.form_id) {
            if self.graph.configured() {
                lead.form_name = self.graph.fetch_form_name(form_id).await;
            }
        }
        if let Some(old) = &old {
            lead.source = old.source;
            lead.received_at = old.received_at;
        }
        self.store.save_lead(lead.clone()).await?;
        Ok((lead, old.is_none()))
    }

    async fn notify_new_lead(&self, lead: &Lead) {
        let subtitle: Vec<&str> = [lead.form_name.as_deref(), lead.campaign_name.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let payload = Payload {
            title: format!("New lead: {}", lead.title),
            body: if subtitle.is_empty() {
                "Tap to see all details".to_string()
            } else {
                subtitle.join(" · ")
            },
            url: format!("/leads/{}", lead.id),
            lead_id: Some(lead.id.clone()),
            tag: format!("lead-{}", lead.id),
        };
        let r = self.pusher.broadcast(&self.store, &payload).await;
        tracing::info!(
            "lead {} notified: {} sent, {} failed",
            lead.id,
            r.sent,
            r.failed
        );
    }

    async fn process_event(&self, ev: LeadgenEvent) {
        let id = ev.leadgen_id.clone();
        if !self.config.page_id.is_empty() && ev.page_id != self.config.page_id {
            return;
        }
        // Meta retries and can deliver the same lead twice; notify only once.
        if self.store.has_lead(&id).await || !self.in_flight.lock().await.insert(id.clone()) {
            return;
        }
        match self.ingest(&ev, LeadSource::Webhook).await {
            Ok((lead, true)) => self.notify_new_lead(&lead).await,
            Ok(_) => {}
            Err(e) => tracing::error!("failed to process lead {id}: {e}"),
        }
        self.in_flight.lock().await.remove(&id);
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Public, but only accepts requests signed with the Meta app secret.
        .route("/webhook", get(webhook_verify).post(webhook_receive))
        // Everything below is expected to sit behind Cloudflare Access.
        .route("/api/config", get(api_config))
        .route("/api/subscriptions", post(subscribe).delete(unsubscribe))
        .route("/api/test-notification", post(test_notification))
        .route("/api/leads", get(list_leads))
        .route("/api/leads/:id", get(get_lead))
        .route("/api/sync", post(sync))
        .fallback(assets::static_handler)
        .with_state(state)
}

// ---------- responses ----------

/// Error body for every failing `/api` call.
#[derive(Serialize)]
struct ApiError {
    error: String,
}

fn fail(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ApiError { error: msg.into() })).into_response()
}

fn server_error(e: anyhow::Error) -> Response {
    tracing::error!("{e}");
    fail(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigResponse {
    vapid_public_key: Option<String>,
    push_enabled: bool,
    meta_configured: bool,
}

#[derive(Serialize)]
struct LeadSummary {
    id: String,
    title: String,
    created_time: DateTime<Utc>,
    form_name: Option<String>,
    campaign_name: Option<String>,
    platform: Option<String>,
}

#[derive(Serialize)]
struct LeadsResponse {
    leads: Vec<LeadSummary>,
}

#[derive(Serialize)]
struct LeadResponse {
    lead: Lead,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Serialize)]
struct SyncResponse {
    added: usize,
}

// ---------- Meta webhook ----------

#[derive(Deserialize)]
struct VerifyQuery {
    #[serde(rename = "hub.mode")]
    mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    challenge: Option<String>,
}

async fn webhook_verify(State(st): State<Arc<AppState>>, Query(q): Query<VerifyQuery>) -> Response {
    let expected = &st.config.verify_token;
    let ok = q.mode.as_deref() == Some("subscribe")
        && !expected.is_empty()
        && q.verify_token.as_ref() == Some(expected);
    if ok {
        q.challenge.unwrap_or_default().into_response()
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

async fn webhook_receive(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok());
    if !meta::verify_signature(&body, sig, &st.config.app_secret) {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(payload) = serde_json::from_slice::<WebhookPayload>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    // Meta wants a fast 200 and retries otherwise: process after responding.
    let events = payload.leadgen_events();
    tokio::spawn(async move {
        for ev in events {
            st.process_event(ev).await;
        }
    });
    StatusCode::OK
}

// ---------- app API (read-only) ----------

async fn api_config(State(st): State<Arc<AppState>>) -> Json<ConfigResponse> {
    Json(ConfigResponse {
        vapid_public_key: Some(st.pusher.public_key.clone()).filter(|k| !k.is_empty()),
        push_enabled: st.pusher.enabled(),
        meta_configured: st.graph.configured(),
    })
}

#[derive(Deserialize)]
struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}

/// The browser's `PushSubscription.toJSON()`.
#[derive(Deserialize)]
struct SubscriptionBody {
    endpoint: Option<String>,
    keys: Option<SubscriptionKeys>,
}

async fn subscribe(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(b): Json<SubscriptionBody>,
) -> Response {
    let (Some(endpoint), Some(keys)) = (
        b.endpoint
            .filter(|e| e.starts_with("https://") || e.starts_with("http://")),
        b.keys,
    ) else {
        return fail(StatusCode::BAD_REQUEST, "invalid push subscription");
    };
    let sub = Subscription {
        endpoint,
        p256dh: keys.p256dh,
        auth: keys.auth,
        user_agent: headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string(),
        added_at: None,
    };
    match st.store.add_subscription(sub).await {
        Ok(()) => (StatusCode::CREATED, Json(OkResponse { ok: true })).into_response(),
        Err(e) => server_error(e),
    }
}

async fn unsubscribe(State(st): State<Arc<AppState>>, Json(b): Json<SubscriptionBody>) -> Response {
    if let Some(endpoint) = b.endpoint {
        if let Err(e) = st.store.remove_subscription(&endpoint).await {
            return server_error(e);
        }
    }
    Json(OkResponse { ok: true }).into_response()
}

async fn test_notification(State(st): State<Arc<AppState>>) -> Json<BroadcastResult> {
    let payload = Payload {
        title: "Meta Leads test".into(),
        body: "Notifications are working on this device.".into(),
        url: "/".into(),
        lead_id: None,
        tag: "test".into(),
    };
    Json(st.pusher.broadcast(&st.store, &payload).await)
}

async fn list_leads(State(st): State<Arc<AppState>>) -> Json<LeadsResponse> {
    let leads = st
        .store
        .list_leads()
        .await
        .into_iter()
        .map(|l| LeadSummary {
            id: l.id,
            title: l.title,
            created_time: l.created_time,
            form_name: l.form_name,
            campaign_name: l.campaign_name,
            platform: l.platform,
        })
        .collect();
    Json(LeadsResponse { leads })
}

#[derive(Deserialize)]
struct RefreshQuery {
    refresh: Option<String>,
}

async fn get_lead(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<RefreshQuery>,
) -> Response {
    let Some(mut lead) = st.store.get_lead(&id).await else {
        return fail(StatusCode::NOT_FOUND, "lead not found");
    };
    if q.refresh.as_deref() == Some("1") && st.graph.configured() {
        let ev = LeadgenEvent {
            leadgen_id: lead.id.clone(),
            page_id: lead.page_id.clone().unwrap_or_default(),
            form_id: lead.form_id.clone(),
            ..LeadgenEvent::default()
        };
        match st.ingest(&ev, lead.source).await {
            Ok((fresh, _)) => lead = fresh,
            Err(e) => return server_error(e),
        }
    }
    Json(LeadResponse { lead }).into_response()
}

/// Read-only import of existing leads from all of the page's forms (no
/// notifications). Nothing is written to Meta.
async fn sync(State(st): State<Arc<AppState>>) -> Response {
    if !st.graph.configured() || st.config.page_id.is_empty() {
        return fail(
            StatusCode::BAD_REQUEST,
            "Set META_PAGE_ID and META_PAGE_ACCESS_TOKEN first.",
        );
    }
    let all = match st.graph.all_leads(&st.config.page_id, 200).await {
        Ok(v) => v,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, e.to_string()),
    };
    let mut added = 0;
    for (graph_lead, form_name) in all {
        if st.store.has_lead(&graph_lead.id).await {
            continue;
        }
        let mut lead = graph_lead.into_lead(LeadSource::Sync, Utc::now());
        lead.page_id = Some(st.config.page_id.clone());
        lead.form_name = form_name;
        if let Err(e) = st.store.save_lead(lead).await {
            return server_error(e);
        }
        added += 1;
    }
    Json(SyncResponse { added }).into_response()
}
