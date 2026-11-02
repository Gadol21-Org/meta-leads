use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use hmac::{Hmac, Mac};
use meta_leads_backend::{
    app::{router, AppState, SubscribeOutcome},
    config::Config,
    push::{generate_vapid, Payload, Pusher},
    store::{Store, Subscription},
};
use serde_json::{json, Value};
use sha2::Sha256;
use tower::ServiceExt;
use web_push_native::p256::{elliptic_curve::sec1::ToEncodedPoint, SecretKey};

/// (path, body length, has VAPID header) of each push request received.
type Hits = Arc<Mutex<Vec<(String, usize, bool)>>>;

const SECRET: &str = "app-secret";

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// A stand-in for graph.facebook.com that knows lead 111 and form 900.
async fn fake_graph(calls: Arc<Mutex<Vec<String>>>) -> String {
    async fn node(
        State(calls): State<Arc<Mutex<Vec<String>>>>,
        Path((_v, id)): Path<(String, String)>,
    ) -> (StatusCode, Json<Value>) {
        calls.lock().unwrap().push(id.clone());
        match id.as_str() {
            "111" => (
                StatusCode::OK,
                Json(json!({
                    "id": "111", "created_time": "2026-11-02T08:00:00+0000", "form_id": "900",
                    "campaign_name": "Autumn promo", "platform": "ig", "is_organic": false,
                    "field_data": [
                        {"name": "full_name", "values": ["Dana Levi"]},
                        {"name": "email", "values": ["dana@example.com"]},
                        {"name": "phone_number", "values": ["+972501234567"]},
                    ],
                })),
            ),
            "900" => (
                StatusCode::OK,
                Json(json!({"id": "900", "name": "Contact form"})),
            ),
            _ => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": {"message": "not found"}})),
            ),
        }
    }
    // Page 5's token is "token"; anything else is refused, as Meta does for a bad token.
    async fn subscribe(
        State(calls): State<Arc<Mutex<Vec<String>>>>,
        Path((_v, id)): Path<(String, String)>,
        Query(q): Query<HashMap<String, String>>,
    ) -> (StatusCode, Json<Value>) {
        let token = q.get("access_token").cloned().unwrap_or_default();
        let fields = q.get("subscribed_fields").cloned().unwrap_or_default();
        calls
            .lock()
            .unwrap()
            .push(format!("POST {id}/subscribed_apps {fields}@{token}"));
        if token == "token" && fields == "leadgen" {
            (StatusCode::OK, Json(json!({"success": true})))
        } else {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"message": "(#190) bad token"}})),
            )
        }
    }

    let app = Router::new()
        .route("/:v/:id/subscribed_apps", post(subscribe))
        .route("/:v/:id", get(node))
        .with_state(calls);
    format!("http://{}", serve(app).await)
}

struct Harness {
    state: Arc<AppState>,
    app: Router,
}

async fn harness(graph_root: &str, vapid: Option<(String, String)>) -> Harness {
    harness_with(graph_root, vapid, |_| {}).await
}

async fn harness_with(
    graph_root: &str,
    vapid: Option<(String, String)>,
    configure: impl FnOnce(&mut Config),
) -> Harness {
    // Not via environment variables: tests run in parallel and each has its own fake Graph API.
    let mut config = Config::from_env();
    config.graph_base = format!("{graph_root}/v1");
    config.app_secret = SECRET.into();
    config.verify_token = "verify-me".into();
    config.page_access_token = "token".into();
    config.data_file = String::new();
    if let Some((public, private)) = vapid {
        config.vapid_public_key = public;
        config.vapid_private_key = private;
    }
    configure(&mut config);
    let state = AppState::new(config, Store::open("").await.unwrap()).unwrap();
    Harness {
        app: router(state.clone()),
        state,
    }
}

fn signed(body: &str) -> Request<Body> {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    Request::post("/webhook")
        .header(
            "x-hub-signature-256",
            format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
        )
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn webhook_body(id: &str) -> String {
    json!({"object": "page", "entry": [{"id": "5", "changes": [{"field": "leadgen",
        "value": {"leadgen_id": id, "page_id": "5", "form_id": "900", "created_time": 1793606400}}]}]})
    .to_string()
}

async fn wait_for_lead(st: &AppState, id: &str) {
    for _ in 0..100 {
        if st.store.has_lead(id).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("lead {id} never stored");
}

async fn json_of(res: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn webhook_handshake() {
    let h = harness("http://127.0.0.1:1", None).await;
    let ok = h
        .app
        .clone()
        .oneshot(
            Request::get("/webhook?hub.mode=subscribe&hub.verify_token=verify-me&hub.challenge=42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(ok.into_body(), 100)
            .await
            .unwrap()
            .as_ref(),
        b"42"
    );
    let bad = h
        .app
        .oneshot(
            Request::get("/webhook?hub.mode=subscribe&hub.verify_token=nope&hub.challenge=42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn webhook_rejects_bad_or_missing_signature() {
    let h = harness("http://127.0.0.1:1", None).await;
    let forged = Request::post("/webhook")
        .header("x-hub-signature-256", format!("sha256={}", "0".repeat(64)))
        .body(Body::from(webhook_body("111")))
        .unwrap();
    assert_eq!(
        h.app.clone().oneshot(forged).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let unsigned = Request::post("/webhook")
        .body(Body::from(webhook_body("111")))
        .unwrap();
    assert_eq!(
        h.app.oneshot(unsigned).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(h.state.store.list_leads().await.is_empty());
}

#[tokio::test]
async fn signed_webhook_stores_the_full_lead_once() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let graph = fake_graph(calls.clone()).await;
    let h = harness(&graph, None).await;

    let body = webhook_body("111");
    assert_eq!(
        h.app.clone().oneshot(signed(&body)).await.unwrap().status(),
        StatusCode::OK
    );
    wait_for_lead(&h.state, "111").await;
    // Meta retries the same delivery.
    assert_eq!(
        h.app.clone().oneshot(signed(&body)).await.unwrap().status(),
        StatusCode::OK
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    let lead = h.state.store.get_lead("111").await.unwrap();
    assert_eq!(lead.title, "Dana Levi");
    assert_eq!(lead.form_name.as_deref(), Some("Contact form"));
    assert_eq!(lead.campaign_name.as_deref(), Some("Autumn promo"));
    assert_eq!(lead.created_time.to_rfc3339(), "2026-11-02T08:00:00+00:00");
    let labels: Vec<_> = lead.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(labels, ["Full name", "Email", "Phone number"]);
    assert_eq!(
        calls.lock().unwrap().iter().filter(|c| *c == "111").count(),
        1,
        "duplicate delivery is ignored"
    );
}

#[tokio::test]
async fn lead_is_kept_with_webhook_data_when_graph_fails() {
    let graph = fake_graph(Default::default()).await;
    let h = harness(&graph, None).await;
    assert_eq!(
        h.app
            .clone()
            .oneshot(signed(&webhook_body("222")))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    wait_for_lead(&h.state, "222").await;
    let lead = h.state.store.get_lead("222").await.unwrap();
    assert!(lead.fetch_error.unwrap().contains("not found"));
    assert_eq!(lead.created_time.to_rfc3339(), "2026-11-02T08:00:00+00:00");
}

#[tokio::test]
async fn api_serves_leads_and_is_read_only() {
    let graph = fake_graph(Default::default()).await;
    let h = harness(&graph, None).await;
    h.app
        .clone()
        .oneshot(signed(&webhook_body("111")))
        .await
        .unwrap();
    wait_for_lead(&h.state, "111").await;

    let list = json_of(
        h.app
            .clone()
            .oneshot(Request::get("/api/leads").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(list["leads"].as_array().unwrap().len(), 1);
    let one = json_of(
        h.app
            .clone()
            .oneshot(Request::get("/api/leads/111").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    let email = one["lead"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "email")
        .unwrap();
    assert_eq!(email["value"], "dana@example.com");
    let missing = h
        .app
        .clone()
        .oneshot(Request::get("/api/leads/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    for (method, path) in [
        ("POST", "/api/leads/111"),
        ("PUT", "/api/leads/111"),
        ("DELETE", "/api/leads/111"),
        ("POST", "/api/leads/111/stage"),
    ] {
        let res = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            res.status() == StatusCode::NOT_FOUND || res.status() == StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path} must not modify anything, got {}",
            res.status()
        );
    }
    assert_eq!(
        h.state.store.get_lead("111").await.unwrap().title,
        "Dana Levi"
    );
}

#[tokio::test]
async fn notification_deep_link_is_served_by_the_pwa_shell() {
    let h = harness("http://127.0.0.1:1", None).await;
    let res = h
        .app
        .clone()
        .oneshot(Request::get("/leads/111").body(Body::empty()).unwrap())
        .await
        .unwrap();
    // 200 with index.html when the frontend is built; the webhook and API paths are never shadowed.
    assert!(res.status() == StatusCode::OK || res.status() == StatusCode::NOT_FOUND);
    let api = h
        .app
        .oneshot(Request::get("/api/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(api.status(), StatusCode::NOT_FOUND);
    assert_ne!(
        api.headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap()),
        Some("text/html")
    );
}

/// A push service that records requests and answers 410 for /gone.
async fn fake_push_service(hits: Hits) -> String {
    async fn handle(State(hits): State<Hits>, req: Request<Body>) -> StatusCode {
        let path = req.uri().path().to_string();
        let has_vapid = req
            .headers()
            .get(header::AUTHORIZATION)
            .map(|v| v.to_str().unwrap().starts_with("vapid "))
            .unwrap_or(false);
        let body = axum::body::to_bytes(req.into_body(), usize::MAX)
            .await
            .unwrap();
        hits.lock()
            .unwrap()
            .push((path.clone(), body.len(), has_vapid));
        if path == "/gone" {
            StatusCode::GONE
        } else {
            StatusCode::CREATED
        }
    }
    let app = Router::new().route("/*p", post(handle)).with_state(hits);
    format!("http://{}", serve(app).await)
}

fn subscription(endpoint: String) -> Subscription {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
    let secret = SecretKey::random(&mut web_push_native::p256::elliptic_curve::rand_core::OsRng);
    Subscription {
        endpoint,
        p256dh: B64.encode(secret.public_key().to_encoded_point(false).as_bytes()),
        auth: B64.encode([7u8; 16]),
        user_agent: String::new(),
        added_at: None,
    }
}

#[tokio::test]
async fn broadcast_reaches_every_device_and_prunes_expired_ones() {
    let hits = Arc::new(Mutex::new(Vec::new()));
    let service = fake_push_service(hits.clone()).await;
    let (public, private) = generate_vapid();
    let store = Store::open("").await.unwrap();
    for p in ["a", "b", "gone"] {
        store
            .add_subscription(subscription(format!("{service}/{p}")))
            .await
            .unwrap();
    }
    let pusher = Pusher::new(
        &public,
        &private,
        "mailto:t@example.com",
        reqwest::Client::new(),
    )
    .unwrap();
    let result = pusher
        .broadcast(
            &store,
            &Payload {
                title: "New lead: Dana".into(),
                body: "b".into(),
                url: "/leads/1".into(),
                lead_id: Some("1".into()),
                tag: "lead-1".into(),
            },
        )
        .await;

    assert_eq!((result.sent, result.failed), (2, 1));
    {
        let hits = hits.lock().unwrap();
        assert_eq!(hits.len(), 3);
        assert!(
            hits.iter().all(|(_, len, vapid)| *len > 0 && *vapid),
            "encrypted payload with a VAPID header"
        );
    }
    let mut left: Vec<_> = store
        .list_subscriptions()
        .await
        .into_iter()
        .map(|s| s.endpoint.rsplit('/').next().unwrap().to_string())
        .collect();
    left.sort();
    assert_eq!(left, ["a", "b"]);
}

#[tokio::test]
async fn new_lead_triggers_a_push_to_subscribers() {
    let hits = Arc::new(Mutex::new(Vec::new()));
    let service = fake_push_service(hits.clone()).await;
    let graph = fake_graph(Default::default()).await;
    let h = harness(&graph, Some(generate_vapid())).await;
    h.state
        .store
        .add_subscription(subscription(format!("{service}/phone")))
        .await
        .unwrap();

    h.app
        .clone()
        .oneshot(signed(&webhook_body("111")))
        .await
        .unwrap();
    for _ in 0..100 {
        if !hits.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(hits.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn push_is_a_no_op_without_vapid_keys() {
    let pusher = Pusher::new("", "", "mailto:t@example.com", reqwest::Client::new()).unwrap();
    assert!(!pusher.enabled());
    let store = Store::open("").await.unwrap();
    let r = pusher
        .broadcast(
            &store,
            &Payload {
                title: "x".into(),
                body: String::new(),
                url: "/".into(),
                lead_id: None,
                tag: "t".into(),
            },
        )
        .await;
    assert_eq!((r.sent, r.failed), (0, 0));
}

#[test]
fn meta_helpers() {
    use meta_leads_backend::{
        meta::*,
        store::{Disclaimer, Field, LeadSource},
    };
    assert!(!verify_signature(b"x", Some("sha256=zz"), SECRET));
    assert!(!verify_signature(b"x", None, SECRET));

    let not_a_page: WebhookPayload =
        serde_json::from_value(json!({"object": "user", "entry": []})).unwrap();
    assert!(not_a_page.leadgen_events().is_empty());

    // Ids may arrive as numbers; other change fields are ignored.
    let payload: WebhookPayload = serde_json::from_value(json!({"object": "page", "entry": [{"id": 5, "changes": [
        {"field": "feed", "value": {"anything": true}},
        {"field": "leadgen", "value": {"leadgen_id": 777, "form_id": "9", "created_time": 1793606400}},
    ]}]}))
    .unwrap();
    let events = payload.leadgen_events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].leadgen_id.as_str(), events[0].page_id.as_str()),
        ("777", "5")
    );
    assert_eq!(
        events[0].created_time.unwrap().to_rfc3339(),
        "2026-11-02T08:00:00+00:00"
    );

    let field = |name: &str, value: &str| Field {
        name: name.into(),
        label: String::new(),
        value: value.into(),
    };
    assert_eq!(
        lead_title(&[field("first_name", "A"), field("last_name", "B")]),
        "A B"
    );
    assert_eq!(lead_title(&[field("email", "a@b.co")]), "a@b.co");
    assert_eq!(lead_title(&[]), FALLBACK_TITLE);

    let graph: GraphLead = serde_json::from_value(json!({
        "id": "1", "created_time": "2026-11-02T10:00:00+0200",
        "field_data": [{"name": "what_are_you_interested_in?", "values": ["a", "b"]}],
        "custom_disclaimer_responses": [{"checkbox_key": "terms", "is_checked": "1"}, {"checkbox_key": "ads", "is_checked": false}],
    }))
    .unwrap();
    let lead = graph.into_lead(LeadSource::Sync, chrono::Utc::now());
    assert_eq!(lead.created_time.to_rfc3339(), "2026-11-02T08:00:00+00:00");
    assert_eq!(lead.fields[0].label, "What are you interested in?");
    assert_eq!(lead.fields[0].value, "a, b");
    assert_eq!(
        lead.disclaimers,
        [
            Disclaimer {
                key: "terms".into(),
                checked: true
            },
            Disclaimer {
                key: "ads".into(),
                checked: false
            }
        ]
    );
}

#[tokio::test]
async fn the_page_is_subscribed_to_leadgen_at_startup_and_failures_are_not_fatal() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let graph = fake_graph(calls.clone()).await;

    let h = harness_with(&graph, None, |c| c.page_id = "5".into()).await;
    assert_eq!(h.state.subscribe_page().await, SubscribeOutcome::Subscribed);
    assert!(calls
        .lock()
        .unwrap()
        .contains(&"POST 5/subscribed_apps leadgen@token".to_string()));

    // Meta refuses the token: reported, not a panic.
    let h = harness_with(&graph, None, |c| {
        c.page_id = "5".into();
        c.page_access_token = "expired".into();
    })
    .await;
    match h.state.subscribe_page().await {
        SubscribeOutcome::Failed(msg) => assert!(msg.contains("bad token")),
        other => panic!("expected a failure, got {other:?}"),
    }

    // Without a Page id there is nothing to subscribe.
    let h = harness(&graph, None).await;
    assert_eq!(h.state.subscribe_page().await, SubscribeOutcome::Skipped);
}

// ---------- import and reconciliation ----------

/// A Graph API whose form F1 returns `listed`; lookups answer per id from `lookups`.
#[derive(Default)]
struct FakeMeta {
    listed: Vec<Value>,
    /// How many extra leads to list, each with a "next" link (simulates a list cut at the cap).
    more_pages: bool,
    list_fails: bool,
    lookups: HashMap<String, (StatusCode, Value)>,
}

async fn fake_meta(meta: Arc<Mutex<FakeMeta>>) -> String {
    async fn forms() -> Json<Value> {
        Json(json!({"data": [{"id": "F1", "name": "Spring form"}]}))
    }
    async fn leads(State(m): State<Arc<Mutex<FakeMeta>>>) -> (StatusCode, Json<Value>) {
        let m = m.lock().unwrap();
        if m.list_fails {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"message": "boom", "code": 2}})),
            );
        }
        let mut body = json!({"data": m.listed});
        if m.more_pages {
            body["paging"] = json!({"next": "http://127.0.0.1:1/never"});
        }
        (StatusCode::OK, Json(body))
    }
    async fn node(
        State(m): State<Arc<Mutex<FakeMeta>>>,
        Path((_v, id)): Path<(String, String)>,
    ) -> (StatusCode, Json<Value>) {
        match m.lock().unwrap().lookups.get(&id) {
            Some((status, body)) => (*status, Json(body.clone())),
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": {"message": "?"}})),
            ),
        }
    }
    let app = Router::new()
        .route("/:v/:id/leadgen_forms", get(forms))
        .route("/:v/:id/leads", get(leads))
        .route("/:v/:id", get(node))
        .with_state(meta);
    format!("http://{}", serve(app).await)
}

fn graph_lead(id: &str, name: &str) -> Value {
    json!({
        "id": id, "created_time": "2026-11-02T08:00:00+0000", "form_id": "F1",
        "field_data": [{"name": "full_name", "values": [name]}],
    })
}

fn missing() -> (StatusCode, Value) {
    (
        StatusCode::BAD_REQUEST,
        json!({"error": {"message": "(#100) does not exist", "code": 100, "error_subcode": 33}}),
    )
}

async fn store_lead(h: &Harness, id: &str, age_days: i64) {
    let created = chrono::Utc::now() - chrono::Duration::days(age_days);
    let mut lead = meta_leads_backend::store::Lead::bare(
        id.into(),
        created,
        meta_leads_backend::store::LeadSource::Webhook,
    );
    lead.form_id = Some("F1".into());
    lead.title = format!("old {id}");
    h.state.store.save_lead(lead).await.unwrap();
}

async fn run_import(h: &Harness) -> (StatusCode, Value) {
    let res = h
        .app
        .clone()
        .oneshot(Request::post("/api/sync").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    (status, json_of(res).await)
}

async fn import_harness(meta: Arc<Mutex<FakeMeta>>) -> Harness {
    let graph = fake_meta(meta).await;
    harness_with(&graph, None, |c| c.page_id = "5".into()).await
}

#[tokio::test]
async fn import_refreshes_adds_and_removes_only_leads_meta_confirms_are_gone() {
    let meta = Arc::new(Mutex::new(FakeMeta::default()));
    {
        let mut m = meta.lock().unwrap();
        m.listed = vec![graph_lead("A", "Alice New"), graph_lead("N", "Newbie")];
        m.lookups.insert("B".into(), missing());
        m.lookups.insert(
            "C".into(),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": {"message": "boom"}}),
            ),
        );
    }
    let h = import_harness(meta).await;
    store_lead(&h, "A", 1).await; // still on Meta: refreshed
    store_lead(&h, "B", 1).await; // Meta says it does not exist: removed
    store_lead(&h, "C", 1).await; // the check failed: kept
    store_lead(&h, "D", 200).await; // older than Meta's retention: never checked, kept

    let (status, body) = run_import(&h).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"added": 1, "refreshed": 1, "removed": 1, "skipped_forms": 0, "unverified": 1})
    );
    let store = &h.state.store;
    assert_eq!(store.get_lead("A").await.unwrap().title, "Alice New");
    assert_eq!(
        store.get_lead("A").await.unwrap().source,
        meta_leads_backend::store::LeadSource::Webhook,
        "how a lead first arrived is kept"
    );
    assert!(store.get_lead("N").await.is_some());
    assert!(store.get_lead("B").await.is_none());
    assert!(store.get_lead("C").await.is_some());
    assert!(store.get_lead("D").await.is_some());
}

#[tokio::test]
async fn a_failed_form_removes_nothing() {
    let meta = Arc::new(Mutex::new(FakeMeta::default()));
    {
        let mut m = meta.lock().unwrap();
        m.list_fails = true;
        m.lookups.insert("B".into(), missing());
    }
    let h = import_harness(meta).await;
    store_lead(&h, "B", 1).await;
    let (status, body) = run_import(&h).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["skipped_forms"], 1);
    assert_eq!(body["removed"], 0);
    assert!(h.state.store.get_lead("B").await.is_some());
}

#[tokio::test]
async fn an_empty_or_truncated_list_removes_nothing() {
    // Empty list: more likely a problem than every lead being deleted.
    let meta = Arc::new(Mutex::new(FakeMeta::default()));
    meta.lock().unwrap().lookups.insert("B".into(), missing());
    let h = import_harness(meta).await;
    store_lead(&h, "B", 1).await;
    let (_, body) = run_import(&h).await;
    assert_eq!(body["removed"], 0);
    assert!(h.state.store.get_lead("B").await.is_some());

    // List cut at the 200-lead cap: a lead missing from it proves nothing.
    let meta = Arc::new(Mutex::new(FakeMeta::default()));
    {
        let mut m = meta.lock().unwrap();
        m.listed = (0..200)
            .map(|i| graph_lead(&format!("L{i}"), "x"))
            .collect();
        m.more_pages = true;
        m.lookups.insert("B".into(), missing());
    }
    let h = import_harness(meta).await;
    store_lead(&h, "B", 1).await;
    let (_, body) = run_import(&h).await;
    assert_eq!(body["added"], 200);
    assert_eq!(body["removed"], 0);
    assert_eq!(body["skipped_forms"], 1);
    assert!(h.state.store.get_lead("B").await.is_some());
}
