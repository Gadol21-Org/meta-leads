//! The Facebook Pages this app reads leads from, each with its own access token.
//!
//! Tokens come from `META_PAGE_TOKENS` / `META_PAGE_ID`+`META_PAGE_ACCESS_TOKEN`, and/or are
//! derived at startup from one `META_USER_TOKEN` (a system user that has the Pages assigned).

use std::collections::BTreeMap;

use tokio::sync::RwLock;

use crate::{config::Config, meta::Graph};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageAuth {
    pub id: String,
    pub name: Option<String>,
    pub token: String,
}

#[derive(Default)]
pub struct Pages {
    inner: RwLock<BTreeMap<String, PageAuth>>,
}

impl Pages {
    /// The Pages whose tokens are configured directly.
    pub fn from_config(config: &Config) -> Pages {
        let map = config
            .page_tokens
            .iter()
            .map(|p| {
                (
                    p.page_id.clone(),
                    PageAuth {
                        id: p.page_id.clone(),
                        name: None,
                        token: p.token.clone(),
                    },
                )
            })
            .collect();
        Pages {
            inner: RwLock::new(map),
        }
    }

    /// Derives tokens from `META_USER_TOKEN` and looks up the Pages' names. Best effort:
    /// failures are logged and leave that Page without a name (or without a token).
    pub async fn resolve(&self, graph: &Graph, config: &Config) {
        if !config.user_token.is_empty() {
            let found = if config.page_ids.is_empty() {
                graph
                    .list_pages(&config.user_token)
                    .await
                    .map_err(|e| tracing::warn!("could not list Pages: {e}"))
                    .unwrap_or_default()
            } else {
                let mut found = Vec::new();
                for id in &config.page_ids {
                    match graph.fetch_page(&config.user_token, id).await {
                        Ok(info) => found.push(info),
                        Err(e) => tracing::warn!("could not read Page {id}: {e}"),
                    }
                }
                found
            };
            let mut map = self.inner.write().await;
            for info in found {
                match info.access_token {
                    Some(token) => {
                        map.entry(info.id.clone()).or_insert(PageAuth { id: info.id, name: info.name, token });
                    }
                    None => tracing::warn!("no page token for Page {}: is the system user assigned to it with Manage Page?", info.id),
                }
            }
        }
        // Names for explicitly configured tokens.
        let unnamed: Vec<PageAuth> = self
            .list()
            .await
            .into_iter()
            .filter(|p| p.name.is_none())
            .collect();
        for page in unnamed {
            match graph.fetch_page(&page.token, &page.id).await {
                Ok(info) => {
                    if let Some(p) = self.inner.write().await.get_mut(&page.id) {
                        p.name = info.name;
                    }
                }
                Err(e) => tracing::warn!("could not read the name of Page {}: {e}", page.id),
            }
        }
    }

    pub async fn get(&self, page_id: &str) -> Option<PageAuth> {
        self.inner.read().await.get(page_id).cloned()
    }

    pub async fn list(&self) -> Vec<PageAuth> {
        self.inner.read().await.values().cloned().collect()
    }
}
