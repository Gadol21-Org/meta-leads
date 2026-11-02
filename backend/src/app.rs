use std::{collections::HashSet, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::{
    assets,
    config::Config,
    meta::{self, Graph, LeadgenEvent},
    push::{Payload, Pusher},
    store::{Lead, Store, Subscription},
};

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

    /// Fetches a lead from Meta and stores it. Returns (lead, is_new).
    async fn ingest(&self, ev: &LeadgenEvent, source: &str) -> anyhow::Result<(Lead, bool)> {
        let existing = self.store.get_lead(&ev.leadgen_id).await;
        let mut fetch_error = None;
        let mut graph_lead = None;
        if self.graph.configured() {
            match self.graph.fetch_lead(&ev.leadgen_id).await {
                Ok(g) => graph_lead = Some(g),
                Err(e) => {
                    tracing::error!("could not fetch lead {}: {e}", ev.leadgen_id);
                    fetch_error = Some(e.to_string());
                }
            }
        } else {
            fetch_error = Some(
                "META_PAGE_ACCESS_TOKEN is not set, so only the webhook data is available."
                    .to_string(),
            );
        }

        let mut lead = match &graph_lead {
            Some(g) => meta::lead_from_graph(g, source),
            None => {
                // Keep the richer data we already had if a refresh fails.
                if let Some(old) = existing.filter(|l| l.fetch_error.is_none()) {
                    return Ok((old, false));
                }
                Lead {
                    id: ev.leadgen_id.clone(),
                    created_time: meta::normalise_time(
                        &ev.created_time.map(Value::from).unwrap_or(Value::Null),
                    ),
                    title: "New lead".to_string(),
                    form_id: ev.form_id.clone(),
                    ad_id: ev.ad_id.clone(),
                    adset_id: ev.adgroup_id.clone(),
                    fetch_error,
                    source: source.to_string(),
                    received_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    ..Lead::default()
                }
            }
        };
        let old = self.store.get_lead(&ev.leadgen_id).await;
        lead.page_id = Some(ev.page_id.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| Some(self.config.page_id.clone()).filter(|s| !s.is_empty()));
        lead.form_id = lead.form_id.or_else(|| ev.form_id.clone());
        lead.form_name = old.as_ref().and_then(|l| l.form_name.clone());
        if lead.form_name.is_none() {
            if let Some(form_id) = &lead.form_id {
                if self.graph.configured() {
                    lead.form_name = self.graph.fetch_form_name(form_id).await;
                }
            }
        }
        if let Some(old) = &old {
            lead.source = old.source.clone();
            lead.received_at = old.received_at.clone();
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
        match self.ingest(&ev, "webhook").await {
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

// ---------- Meta webhook ----------

async fn webhook_verify(
    State(st): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let ok = q.get("hub.mode").map(String::as_str) == Some("subscribe")
        && !st.config.verify_token.is_empty()
        && q.get("hub.verify_token") == Some(&st.config.verify_token);
    if !ok {
        return StatusCode::FORBIDDEN.into_response();
    }
    q.get("hub.challenge")
        .cloned()
        .unwrap_or_default()
        .into_response()
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
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    // Meta wants a fast 200 and retries otherwise: process after responding.
    let events = meta::extract_leadgen_events(&payload);
    tokio::spawn(async move {
        for ev in events {
            st.process_event(ev).await;
        }
    });
    StatusCode::OK
}

// ---------- app API (read-only) ----------

async fn api_config(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "vapidPublicKey": Some(&st.pusher.public_key).filter(|k| !k.is_empty()),
        "pushEnabled": st.pusher.enabled(),
        "metaConfigured": st.graph.configured(),
    }))
}

#[derive(Deserialize)]
struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}
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
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid push subscription"})),
        )
            .into_response();
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
        added_at: String::new(),
    };
    match st.store.add_subscription(sub).await {
        Ok(()) => (StatusCode::CREATED, Json(json!({"ok": true}))).into_response(),
        Err(e) => server_error(e),
    }
}

async fn unsubscribe(State(st): State<Arc<AppState>>, Json(b): Json<SubscriptionBody>) -> Response {
    if let Some(endpoint) = b.endpoint {
        if let Err(e) = st.store.remove_subscription(&endpoint).await {
            return server_error(e);
        }
    }
    Json(json!({"ok": true})).into_response()
}

async fn test_notification(State(st): State<Arc<AppState>>) -> Json<Value> {
    let r = st
        .pusher
        .broadcast(
            &st.store,
            &Payload {
                title: "Meta Leads test".into(),
                body: "Notifications are working on this device.".into(),
                url: "/".into(),
                lead_id: None,
                tag: "test".into(),
            },
        )
        .await;
    Json(json!(r))
}

async fn list_leads(State(st): State<Arc<AppState>>) -> Json<Value> {
    let leads: Vec<Value> = st
        .store
        .list_leads()
        .await
        .into_iter()
        .map(|l| {
            json!({
                "id": l.id, "title": l.title, "created_time": l.created_time,
                "form_name": l.form_name, "campaign_name": l.campaign_name, "platform": l.platform,
            })
        })
        .collect();
    Json(json!({ "leads": leads }))
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
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "lead not found"})),
        )
            .into_response();
    };
    if q.refresh.as_deref() == Some("1") && st.graph.configured() {
        let ev = LeadgenEvent {
            leadgen_id: lead.id.clone(),
            page_id: lead.page_id.clone().unwrap_or_default(),
            form_id: lead.form_id.clone(),
            ..LeadgenEvent::default()
        };
        match st.ingest(&ev, &lead.source.clone()).await {
            Ok((fresh, _)) => lead = fresh,
            Err(e) => return server_error(e),
        }
    }
    Json(json!({ "lead": lead })).into_response()
}

/// Read-only import of existing leads from all of the page's forms (no
/// notifications). Nothing is written to Meta.
async fn sync(State(st): State<Arc<AppState>>) -> Response {
    if !st.graph.configured() || st.config.page_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Set META_PAGE_ID and META_PAGE_ACCESS_TOKEN first."})),
        )
            .into_response();
    }
    let all = match st.graph.all_leads(&st.config.page_id, 200).await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": e.to_string(), "added": 0})),
            )
                .into_response()
        }
    };
    let mut added = 0;
    for (g, form_name) in all {
        let mut lead = meta::lead_from_graph(&g, "sync");
        if lead.id.is_empty() || st.store.has_lead(&lead.id).await {
            continue;
        }
        lead.page_id = Some(st.config.page_id.clone());
        lead.form_name = form_name;
        if let Err(e) = st.store.save_lead(lead).await {
            return server_error(e);
        }
        added += 1;
    }
    Json(json!({ "added": added })).into_response()
}

fn server_error(e: anyhow::Error) -> Response {
    tracing::error!("{e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal error"})),
    )
        .into_response()
}
