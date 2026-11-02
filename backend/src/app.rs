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
    pages::{PageAuth, Pages},
    push::{BroadcastResult, Payload, Pusher},
    store::{Lead, LeadSource, Store, Subscription},
};

pub struct AppState {
    pub config: Config,
    pub store: Store,
    pub graph: Graph,
    pub pages: Pages,
    pub pusher: Pusher,
    /// Lead ids being processed right now, so a Meta retry can't notify twice.
    in_flight: Mutex<HashSet<String>>,
}

impl AppState {
    pub fn new(config: Config, store: Store) -> anyhow::Result<Arc<AppState>> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()?;
        let graph = Graph::new(&config.graph_base, http.clone());
        let pages = Pages::from_config(&config);
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
            pages,
            pusher,
            in_flight: Mutex::new(HashSet::new()),
        }))
    }

    /// Looks up Page names and derives Page tokens from `META_USER_TOKEN`. Call once at startup.
    pub async fn resolve_pages(&self) {
        self.pages.resolve(&self.graph, &self.config).await;
    }

    /// The lead as Meta describes it now, read with the token of the Page it came from.
    async fn fetch_lead(
        &self,
        ev: &LeadgenEvent,
        page: Option<&PageAuth>,
        source: LeadSource,
    ) -> anyhow::Result<Lead> {
        let page = page.ok_or_else(|| {
            anyhow::anyhow!(
                "No access token is configured for Page {}, so only the webhook data is available.",
                ev.page_id
            )
        })?;
        let graph_lead = self.graph.fetch_lead(&page.token, &ev.leadgen_id).await?;
        let mut lead = graph_lead.into_lead(source, ev.created_time.unwrap_or_else(Utc::now));
        lead.form_id = lead.form_id.or_else(|| ev.form_id.clone());
        Ok(lead)
    }

    /// Fetches a lead from Meta and stores it. Returns (lead, is_new).
    async fn ingest(&self, ev: &LeadgenEvent, source: LeadSource) -> anyhow::Result<(Lead, bool)> {
        let existing = self.store.get_lead(&ev.leadgen_id).await;
        let page = self.pages.get(&ev.page_id).await;
        let mut lead = match self.fetch_lead(ev, page.as_ref(), source).await {
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
        lead.page_id = Some(ev.page_id.clone()).filter(|s| !s.is_empty());
        lead.page_name = page.as_ref().and_then(|p| p.name.clone());
        lead.form_name = old.as_ref().and_then(|l| l.form_name.clone());
        if let (None, Some(form_id), Some(page)) = (&lead.form_name, &lead.form_id, &page) {
            lead.form_name = self.graph.fetch_form_name(&page.token, form_id).await;
        }
        if let Some(old) = &old {
            lead.source = old.source;
            lead.received_at = old.received_at;
        }
        self.store.save_lead(lead.clone()).await?;
        Ok((lead, old.is_none()))
    }

    async fn notify_new_lead(&self, lead: &Lead) {
        // Only name the Page when there is more than one to tell apart.
        let page_name = if self.pages.list().await.len() > 1 {
            lead.page_name.as_deref()
        } else {
            None
        };
        let subtitle: Vec<&str> = [
            page_name,
            lead.form_name.as_deref(),
            lead.campaign_name.as_deref(),
        ]
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
    pages: Vec<PageView>,
}

#[derive(Serialize)]
struct PageView {
    id: String,
    name: Option<String>,
}

#[derive(Serialize)]
struct LeadSummary {
    id: String,
    title: String,
    created_time: DateTime<Utc>,
    page_name: Option<String>,
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
    /// One message per Page that could not be read; the other Pages are still imported.
    errors: Vec<String>,
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
    let pages: Vec<PageView> = st
        .pages
        .list()
        .await
        .into_iter()
        .map(|p| PageView {
            id: p.id,
            name: p.name,
        })
        .collect();
    Json(ConfigResponse {
        vapid_public_key: Some(st.pusher.public_key.clone()).filter(|k| !k.is_empty()),
        push_enabled: st.pusher.enabled(),
        meta_configured: !pages.is_empty(),
        pages,
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
            page_name: l.page_name,
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
    if q.refresh.as_deref() == Some("1") {
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

/// Read-only import of existing leads from all forms of every configured Page (no
/// notifications). Nothing is written to Meta.
async fn sync(State(st): State<Arc<AppState>>) -> Response {
    let pages = st.pages.list().await;
    if pages.is_empty() {
        return fail(
            StatusCode::BAD_REQUEST,
            "Configure at least one Page token first (see the setup guide).",
        );
    }
    let mut added = 0;
    let mut errors = Vec::new();
    for page in pages {
        let leads = match st.graph.all_leads(&page.token, &page.id, 200).await {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{}: {e}", page.name.as_deref().unwrap_or(&page.id)));
                continue;
            }
        };
        for (graph_lead, form_name) in leads {
            if st.store.has_lead(&graph_lead.id).await {
                continue;
            }
            let mut lead = graph_lead.into_lead(LeadSource::Sync, Utc::now());
            lead.page_id = Some(page.id.clone());
            lead.page_name = page.name.clone();
            lead.form_name = form_name;
            if let Err(e) = st.store.save_lead(lead).await {
                return server_error(e);
            }
            added += 1;
        }
    }
    Json(SyncResponse { added, errors }).into_response()
}
