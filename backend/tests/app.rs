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
    app::{router, AppState},
    config::{parse_page_tokens, Config, PageToken},
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

/// "id@token" of each Graph API call received.
type Calls = Arc<Mutex<Vec<String>>>;

/// A stand-in for graph.facebook.com. Page 5 ("Facebook Page", token5) owns lead 111 and
/// page 6 ("Instagram Page", token6) owns lead 444; reading a lead with the wrong Page's
/// token fails, as it does on Meta. The token "system" can read both Pages' details.
async fn fake_graph(calls: Calls) -> String {
    const PAGES: [(&str, &str, &str); 2] = [
        ("5", "Facebook Page", "token5"),
        ("6", "Instagram Page", "token6"),
    ];

    fn page_json(id: &str, name: &str, token: &str, caller: &str) -> Value {
        if caller == "system" {
            json!({"id": id, "name": name, "access_token": token})
        } else {
            json!({"id": id, "name": name})
        }
    }

    fn wrong_token() -> (StatusCode, Json<Value>) {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "(#190) wrong page token"}})),
        )
    }

    async fn node(
        State(calls): State<Calls>,
        Path((_v, id)): Path<(String, String)>,
        Query(q): Query<HashMap<String, String>>,
    ) -> (StatusCode, Json<Value>) {
        let token = q.get("access_token").cloned().unwrap_or_default();
        calls.lock().unwrap().push(format!("{id}@{token}"));
        if let Some((pid, name, ptoken)) = PAGES.iter().find(|(pid, ..)| *pid == id) {
            return match token.as_str() {
                "system" => (StatusCode::OK, Json(page_json(pid, name, ptoken, &token))),
                t if t == *ptoken => (StatusCode::OK, Json(page_json(pid, name, ptoken, &token))),
                _ => wrong_token(),
            };
        }
        let lead_page_token = match id.as_str() {
            "111" => "token5",
            "444" => "token6",
            "900" => "",
            _ => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": {"message": "not found"}})),
                )
            }
        };
        if token.is_empty() || (!lead_page_token.is_empty() && token != lead_page_token) {
            return wrong_token();
        }
        if id == "900" {
            return (
                StatusCode::OK,
                Json(json!({"id": "900", "name": "Contact form"})),
            );
        }
        let name = if id == "111" { "Dana Levi" } else { "Noa Bar" };
        (
            StatusCode::OK,
            Json(json!({
                "id": id, "created_time": "2026-11-02T08:00:00+0000", "form_id": "900",
                "campaign_name": "Autumn promo", "platform": "ig", "is_organic": false,
                "field_data": [
                    {"name": "full_name", "values": [name]},
                    {"name": "email", "values": ["dana@example.com"]},
                    {"name": "phone_number", "values": ["+972501234567"]},
                ],
            })),
        )
    }

    async fn accounts(Query(q): Query<HashMap<String, String>>) -> (StatusCode, Json<Value>) {
        if q.get("access_token").map(String::as_str) != Some("system") {
            return wrong_token();
        }
        let data: Vec<Value> = PAGES
            .iter()
            .map(|(id, name, token)| page_json(id, name, token, "system"))
            .collect();
        (StatusCode::OK, Json(json!({ "data": data })))
    }

    async fn subscribe(
        State(calls): State<Calls>,
        Path((_v, id)): Path<(String, String)>,
        Query(q): Query<HashMap<String, String>>,
    ) -> (StatusCode, Json<Value>) {
        let token = q.get("access_token").cloned().unwrap_or_default();
        let fields = q.get("subscribed_fields").cloned().unwrap_or_default();
        calls
            .lock()
            .unwrap()
            .push(format!("POST {id}/subscribed_apps {fields}@{token}"));
        match PAGES.iter().find(|(pid, ..)| *pid == id) {
            Some((_, _, ptoken)) if *ptoken == token && fields == "leadgen" => {
                (StatusCode::OK, Json(json!({"success": true})))
            }
            _ => wrong_token(),
        }
    }

    let app = Router::new()
        .route("/:v/:id/subscribed_apps", post(subscribe))
        .route("/:v/me/accounts", get(accounts))
        .route("/:v/:id", get(node))
        .with_state(calls);
    format!("http://{}", serve(app).await)
}

struct Harness {
    state: Arc<AppState>,
    app: Router,
}

fn page_tokens() -> Vec<PageToken> {
    [("5", "token5"), ("6", "token6")]
        .into_iter()
        .map(|(page_id, token)| PageToken {
            page_id: page_id.into(),
            token: token.into(),
        })
        .collect()
}

async fn harness(graph_root: &str, vapid: Option<(String, String)>) -> Harness {
    harness_with(graph_root, vapid, |c| c.page_tokens = page_tokens()).await
}

async fn harness_with(
    graph_root: &str,
    vapid: Option<(String, String)>,
    configure: impl FnOnce(&mut Config),
) -> Harness {
    // Not via environment variables: tests run in parallel and each has its own fake Graph API.
    let mut config = Config::from_env().unwrap();
    config.graph_base = format!("{graph_root}/v1");
    config.app_secret = SECRET.into();
    config.verify_token = "verify-me".into();
    config.page_tokens = Vec::new();
    config.data_file = String::new();
    if let Some((public, private)) = vapid {
        config.vapid_public_key = public;
        config.vapid_private_key = private;
    }
    configure(&mut config);
    let state = AppState::new(config, Store::open("").await.unwrap()).unwrap();
    state.resolve_pages().await;
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
    webhook_body_for(id, "5")
}

fn webhook_body_for(id: &str, page: &str) -> String {
    json!({"object": "page", "entry": [{"id": page, "changes": [{"field": "leadgen",
        "value": {"leadgen_id": id, "page_id": page, "form_id": "900", "created_time": 1793606400}}]}]})
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
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "111@token5")
            .count(),
        1,
        "duplicate delivery is ignored, and the lead is read with its own Page's token"
    );
    assert_eq!(lead.page_id.as_deref(), Some("5"));
    assert_eq!(lead.page_name.as_deref(), Some("Facebook Page"));
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

async fn ingest_via_webhook(h: &Harness, lead_id: &str, page: &str) {
    let res = h
        .app
        .clone()
        .oneshot(signed(&webhook_body_for(lead_id, page)))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    wait_for_lead(&h.state, lead_id).await;
}

#[tokio::test]
async fn each_page_reads_its_leads_with_its_own_token() {
    let graph = fake_graph(Default::default()).await;
    let h = harness(&graph, None).await;
    ingest_via_webhook(&h, "111", "5").await;
    ingest_via_webhook(&h, "444", "6").await;

    let fb = h.state.store.get_lead("111").await.unwrap();
    let ig = h.state.store.get_lead("444").await.unwrap();
    assert_eq!(
        (fb.title.as_str(), fb.page_name.as_deref()),
        ("Dana Levi", Some("Facebook Page"))
    );
    assert_eq!(
        (ig.title.as_str(), ig.page_name.as_deref()),
        ("Noa Bar", Some("Instagram Page"))
    );
    assert!(fb.fetch_error.is_none() && ig.fetch_error.is_none());

    let list = json_of(
        h.app
            .clone()
            .oneshot(Request::get("/api/leads").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    let names: Vec<_> = list["leads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["page_name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"Facebook Page".into()) && names.contains(&"Instagram Page".into()));

    let config = json_of(
        h.app
            .clone()
            .oneshot(Request::get("/api/config").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(config["metaConfigured"], true);
    assert_eq!(config["pages"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn lead_from_an_unconfigured_page_is_kept_with_a_clear_error() {
    let graph = fake_graph(Default::default()).await;
    let h = harness(&graph, None).await;
    ingest_via_webhook(&h, "777", "7").await;
    let lead = h.state.store.get_lead("777").await.unwrap();
    assert!(lead.fetch_error.unwrap().contains("Page 7"));
    assert_eq!(lead.page_id.as_deref(), Some("7"));
}

#[tokio::test]
async fn one_system_user_token_covers_several_pages() {
    let graph = fake_graph(Default::default()).await;
    // Explicit list of Pages...
    let h = harness_with(&graph, None, |c| {
        c.user_token = "system".into();
        c.page_ids = vec!["5".into(), "6".into()];
    })
    .await;
    ingest_via_webhook(&h, "111", "5").await;
    ingest_via_webhook(&h, "444", "6").await;
    assert_eq!(
        h.state
            .store
            .get_lead("444")
            .await
            .unwrap()
            .page_name
            .as_deref(),
        Some("Instagram Page")
    );

    // ...or every Page the token can list.
    let h = harness_with(&graph, None, |c| c.user_token = "system".into()).await;
    assert_eq!(h.state.pages.list().await.len(), 2);
    ingest_via_webhook(&h, "111", "5").await;
    assert!(h
        .state
        .store
        .get_lead("111")
        .await
        .unwrap()
        .fetch_error
        .is_none());
}

#[tokio::test]
async fn import_reads_every_page_and_reports_failures() {
    let graph = fake_graph(Default::default()).await;
    let h = harness(&graph, None).await;
    // The fake has no leadgen_forms listing, so each Page's import fails; it must say which.
    let res = h
        .app
        .clone()
        .oneshot(Request::post("/api/sync").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    assert_eq!(body["added"], 0);
    assert_eq!(body["errors"].as_array().unwrap().len(), 2);

    let none = harness_with(&graph, None, |_| {}).await;
    let res = none
        .app
        .clone()
        .oneshot(Request::post("/api/sync").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn page_tokens_are_parsed_from_the_environment_format() {
    let parsed = parse_page_tokens("111:aaa, 222:bbb ,").unwrap();
    assert_eq!(
        parsed,
        [
            PageToken {
                page_id: "111".into(),
                token: "aaa".into()
            },
            PageToken {
                page_id: "222".into(),
                token: "bbb".into()
            }
        ]
    );
    assert!(parse_page_tokens("").unwrap().is_empty());
    assert!(parse_page_tokens("111").is_err());
    assert!(parse_page_tokens("111:").is_err());
}

#[tokio::test]
async fn every_page_is_subscribed_to_leadgen_and_failures_are_not_fatal() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let graph = fake_graph(calls.clone()).await;
    let h = harness(&graph, None).await;
    let outcomes = h.state.subscribe_pages().await;
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|(_, r)| r.is_ok()));
    {
        let calls = calls.lock().unwrap();
        assert!(calls.contains(&"POST 5/subscribed_apps leadgen@token5".to_string()));
        assert!(calls.contains(&"POST 6/subscribed_apps leadgen@token6".to_string()));
    }

    // A Page whose token Meta rejects is reported, the other Page still gets subscribed.
    let h = harness_with(&graph, None, |c| {
        c.page_tokens = vec![
            PageToken {
                page_id: "5".into(),
                token: "expired".into(),
            },
            PageToken {
                page_id: "6".into(),
                token: "token6".into(),
            },
        ]
    })
    .await;
    let outcomes = h.state.subscribe_pages().await;
    let by_page: HashMap<_, _> = outcomes.into_iter().collect();
    assert!(by_page["5"]
        .as_ref()
        .unwrap_err()
        .contains("wrong page token"));
    assert!(by_page["6"].is_ok());
}
