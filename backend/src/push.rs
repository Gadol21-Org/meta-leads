//! Web Push fan-out to every subscribed device.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Serialize;
use web_push_native::{
    jwt_simple::algorithms::{ECDSAP256KeyPairLike, ES256KeyPair},
    p256::PublicKey,
    Auth, WebPushBuilder,
};

use crate::store::{Store, Subscription};

#[derive(Serialize, Clone, Debug)]
pub struct Payload {
    pub title: String,
    pub body: String,
    pub url: String,
    #[serde(rename = "leadId", skip_serializing_if = "Option::is_none")]
    pub lead_id: Option<String>,
    pub tag: String,
}

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct BroadcastResult {
    pub sent: usize,
    pub failed: usize,
}

pub struct Pusher {
    key: Option<ES256KeyPair>,
    pub public_key: String,
    subject: String,
    http: reqwest::Client,
}

/// Generates a VAPID key pair: (public, private), both base64url without padding.
pub fn generate_vapid() -> (String, String) {
    let key = ES256KeyPair::generate();
    let public = key.key_pair().public_key().to_bytes_uncompressed();
    (
        URL_SAFE_NO_PAD.encode(public),
        URL_SAFE_NO_PAD.encode(key.to_bytes()),
    )
}

impl Pusher {
    pub fn new(
        public_key: &str,
        private_key: &str,
        subject: &str,
        http: reqwest::Client,
    ) -> anyhow::Result<Pusher> {
        let key = if private_key.is_empty() || public_key.is_empty() {
            None
        } else {
            let bytes = URL_SAFE_NO_PAD
                .decode(private_key.trim_end_matches('='))
                .map_err(|e| anyhow::anyhow!("VAPID_PRIVATE_KEY is not base64url: {e}"))?;
            Some(
                ES256KeyPair::from_bytes(&bytes)
                    .map_err(|e| anyhow::anyhow!("VAPID_PRIVATE_KEY is invalid: {e}"))?,
            )
        };
        Ok(Pusher {
            key,
            public_key: public_key.to_string(),
            subject: subject.to_string(),
            http,
        })
    }

    pub fn enabled(&self) -> bool {
        self.key.is_some()
    }

    async fn send_one(
        &self,
        key: &ES256KeyPair,
        sub: &Subscription,
        body: &[u8],
    ) -> Result<(), SendError> {
        let p256dh = URL_SAFE_NO_PAD
            .decode(sub.p256dh.trim_end_matches('='))
            .map_err(|_| SendError::Bad)?;
        let auth = URL_SAFE_NO_PAD
            .decode(sub.auth.trim_end_matches('='))
            .map_err(|_| SendError::Bad)?;
        if auth.len() != 16 {
            return Err(SendError::Bad);
        }
        let builder = WebPushBuilder::new(
            sub.endpoint.parse().map_err(|_| SendError::Bad)?,
            PublicKey::from_sec1_bytes(&p256dh).map_err(|_| SendError::Bad)?,
            Auth::clone_from_slice(&auth),
        )
        .with_valid_duration(std::time::Duration::from_secs(24 * 3600))
        .with_vapid(key, &self.subject);
        let req = builder.build(body.to_vec()).map_err(|_| SendError::Bad)?;

        let mut r = self
            .http
            .post(req.uri().to_string())
            .header("Urgency", "high");
        for (name, value) in req.headers() {
            r = r.header(name, value);
        }
        let res = r
            .body(req.into_body())
            .send()
            .await
            .map_err(|e| SendError::Network(e.to_string()))?;
        match res.status().as_u16() {
            200..=299 => Ok(()),
            404 | 410 => Err(SendError::Gone),
            s => Err(SendError::Network(format!("push service returned {s}"))),
        }
    }

    /// Sends one notification to every subscribed device; prunes dead ones.
    pub async fn broadcast(&self, store: &Store, payload: &Payload) -> BroadcastResult {
        let Some(key) = &self.key else {
            return BroadcastResult::default();
        };
        let body = serde_json::to_vec(payload).expect("payload serialises");
        let subs = store.list_subscriptions().await;
        let results =
            futures::future::join_all(subs.iter().map(|s| self.send_one(key, s, &body))).await;
        let mut out = BroadcastResult::default();
        for (sub, res) in subs.iter().zip(results) {
            match res {
                Ok(()) => out.sent += 1,
                Err(e) => {
                    out.failed += 1;
                    if matches!(e, SendError::Gone | SendError::Bad) {
                        let _ = store.remove_subscription(&sub.endpoint).await;
                    } else {
                        tracing::warn!(
                            "push to {}… failed: {e:?}",
                            sub.endpoint.chars().take(40).collect::<String>()
                        );
                    }
                }
            }
        }
        out
    }
}

#[derive(Debug)]
#[allow(dead_code)]
enum SendError {
    /// 404/410 from the push service: the subscription is gone.
    Gone,
    /// Malformed subscription.
    Bad,
    Network(String),
}
