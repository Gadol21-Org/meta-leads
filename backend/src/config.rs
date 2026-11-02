//! All configuration comes from environment variables (see `.env.example`).

use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
    /// JSON file holding leads and push subscriptions. Empty = in memory only.
    pub data_file: String,
    pub app_secret: String,
    pub verify_token: String,
    pub page_id: String,
    pub page_access_token: String,
    pub graph_base: String,
    pub vapid_public_key: String,
    pub vapid_private_key: String,
    pub vapid_subject: String,
}

fn var(name: &str) -> String {
    env::var(name).unwrap_or_default()
}

fn var_or(name: &str, default: &str) -> String {
    match env::var(name) {
        Ok(v) if !v.is_empty() => v,
        _ => default.to_string(),
    }
}

impl Config {
    pub fn from_env() -> Config {
        let graph_version = var_or("META_GRAPH_VERSION", "v21.0");
        let graph_root = var_or("META_GRAPH_BASE", "https://graph.facebook.com");
        Config {
            bind_addr: var_or("BIND_ADDR", "0.0.0.0:3000"),
            data_file: var_or("DATA_FILE", "data/db.json"),
            app_secret: var("META_APP_SECRET"),
            verify_token: var("META_VERIFY_TOKEN"),
            page_id: var("META_PAGE_ID"),
            page_access_token: var("META_PAGE_ACCESS_TOKEN"),
            graph_base: format!("{}/{}", graph_root.trim_end_matches('/'), graph_version),
            vapid_public_key: var("VAPID_PUBLIC_KEY"),
            vapid_private_key: var("VAPID_PRIVATE_KEY"),
            vapid_subject: var_or("VAPID_SUBJECT", "mailto:admin@example.com"),
        }
    }
}
