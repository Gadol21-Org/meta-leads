//! All configuration comes from environment variables (see `.env.example`).

use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
    /// JSON file holding leads and push subscriptions. Empty = in memory only.
    pub data_file: String,
    pub app_secret: String,
    pub verify_token: String,
    /// Pages with their own access token: `META_PAGE_TOKENS` plus the single `META_PAGE_ID`/`META_PAGE_ACCESS_TOKEN`.
    pub page_tokens: Vec<PageToken>,
    /// A system-user (or long-lived user) token that can read several Pages; each Page's own
    /// token is derived from it at startup.
    pub user_token: String,
    /// Pages to derive tokens for from `user_token`. Empty means every Page the token can list.
    pub page_ids: Vec<String>,
    pub graph_base: String,
    pub vapid_public_key: String,
    pub vapid_private_key: String,
    pub vapid_subject: String,
}

/// A Facebook Page and the access token that can read its leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageToken {
    pub page_id: String,
    pub token: String,
}

/// Parses `META_PAGE_TOKENS`: `pageId:token,pageId:token`.
pub fn parse_page_tokens(raw: &str) -> anyhow::Result<Vec<PageToken>> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|pair| match pair.split_once(':') {
            Some((id, token)) if !id.trim().is_empty() && !token.trim().is_empty() => {
                Ok(PageToken {
                    page_id: id.trim().to_string(),
                    token: token.trim().to_string(),
                })
            }
            _ => anyhow::bail!("META_PAGE_TOKENS entries must look like pageId:token"),
        })
        .collect()
}

fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
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
    pub fn from_env() -> anyhow::Result<Config> {
        let graph_version = var_or("META_GRAPH_VERSION", "v21.0");
        let graph_root = var_or("META_GRAPH_BASE", "https://graph.facebook.com");
        let mut page_tokens = parse_page_tokens(&var("META_PAGE_TOKENS"))?;
        let (single_id, single_token) = (var("META_PAGE_ID"), var("META_PAGE_ACCESS_TOKEN"));
        if !single_id.is_empty() && !single_token.is_empty() {
            page_tokens.push(PageToken {
                page_id: single_id,
                token: single_token,
            });
        }
        Ok(Config {
            bind_addr: var_or("BIND_ADDR", "0.0.0.0:3000"),
            data_file: var_or("DATA_FILE", "data/db.json"),
            app_secret: var("META_APP_SECRET"),
            verify_token: var("META_VERIFY_TOKEN"),
            page_tokens,
            user_token: var("META_USER_TOKEN"),
            page_ids: split_list(&var("META_PAGE_IDS")),
            graph_base: format!("{}/{}", graph_root.trim_end_matches('/'), graph_version),
            vapid_public_key: var("VAPID_PUBLIC_KEY"),
            vapid_private_key: var("VAPID_PRIVATE_KEY"),
            vapid_subject: var_or("VAPID_SUBJECT", "mailto:admin@example.com"),
        })
    }
}
