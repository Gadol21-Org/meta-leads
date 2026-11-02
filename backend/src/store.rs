//! Tiny JSON-file store. Writes are serialised and atomic (write + rename),
//! which is plenty for a single instance with a few thousand leads.
//! Swap this module for a real database if you run more than one instance.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Field {
    pub name: String,
    pub label: String,
    pub value: String,
}

/// A custom disclaimer checkbox on the lead form.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Disclaimer {
    pub key: String,
    pub checked: bool,
}

/// How a lead reached us.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LeadSource {
    #[default]
    Webhook,
    /// Read-only import of existing leads from the page's forms.
    Sync,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lead {
    pub id: String,
    pub created_time: DateTime<Utc>,
    pub title: String,
    #[serde(default)]
    pub fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ad_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ad_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adset_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_organic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partner_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disclaimers: Vec<Disclaimer>,
    /// Set when the Graph API lookup failed and only webhook data is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_error: Option<String>,
    pub source: LeadSource,
    pub received_at: DateTime<Utc>,
}

impl Lead {
    /// A lead built from webhook data alone, for when the Graph lookup fails.
    pub fn bare(id: String, created_time: DateTime<Utc>, source: LeadSource) -> Lead {
        Lead {
            id,
            created_time,
            title: crate::meta::FALLBACK_TITLE.to_string(),
            fields: Vec::new(),
            form_id: None,
            form_name: None,
            page_id: None,
            ad_id: None,
            ad_name: None,
            adset_id: None,
            adset_name: None,
            campaign_id: None,
            campaign_name: None,
            platform: None,
            is_organic: None,
            partner_name: None,
            disclaimers: Vec::new(),
            fetch_error: None,
            source,
            received_at: Utc::now(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub added_at: Option<DateTime<Utc>>,
}

#[derive(Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    leads: BTreeMap<String, Lead>,
    #[serde(default)]
    subscriptions: BTreeMap<String, Subscription>,
}

pub struct Store {
    file: Option<PathBuf>,
    data: Mutex<Data>,
}

impl Store {
    pub async fn open(file: &str) -> anyhow::Result<Store> {
        if file.is_empty() {
            return Ok(Store {
                file: None,
                data: Mutex::new(Data::default()),
            });
        }
        let path = PathBuf::from(file);
        let data = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Store {
            file: Some(path),
            data: Mutex::new(data),
        })
    }

    async fn persist(&self, data: &Data) -> anyhow::Result<()> {
        let Some(path) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, serde_json::to_vec_pretty(data)?).await?;
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }

    /// Newest first.
    pub async fn list_leads(&self) -> Vec<Lead> {
        let d = self.data.lock().await;
        let mut v: Vec<Lead> = d.leads.values().cloned().collect();
        v.sort_by_key(|l| std::cmp::Reverse(l.created_time));
        v
    }

    pub async fn get_lead(&self, id: &str) -> Option<Lead> {
        self.data.lock().await.leads.get(id).cloned()
    }

    pub async fn has_lead(&self, id: &str) -> bool {
        self.data.lock().await.leads.contains_key(id)
    }

    pub async fn save_lead(&self, lead: Lead) -> anyhow::Result<()> {
        let mut d = self.data.lock().await;
        d.leads.insert(lead.id.clone(), lead);
        self.persist(&d).await
    }

    pub async fn list_subscriptions(&self) -> Vec<Subscription> {
        self.data
            .lock()
            .await
            .subscriptions
            .values()
            .cloned()
            .collect()
    }

    pub async fn add_subscription(&self, mut sub: Subscription) -> anyhow::Result<()> {
        sub.added_at = Some(Utc::now());
        let mut d = self.data.lock().await;
        d.subscriptions.insert(sub.endpoint.clone(), sub);
        self.persist(&d).await
    }

    pub async fn remove_subscription(&self, endpoint: &str) -> anyhow::Result<()> {
        let mut d = self.data.lock().await;
        if d.subscriptions.remove(endpoint).is_some() {
            self.persist(&d).await?;
        }
        Ok(())
    }
}
