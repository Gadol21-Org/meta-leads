//! Import and reconciliation: keeps the local store a cache of what Meta holds.
//!
//! Leads Meta still returns are added or refreshed. A stored lead is removed only when Meta
//! itself confirms it no longer exists, and never on weak evidence (see `deletion_candidates`).

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::meta::{FormFetch, Graph, GraphLead, LeadLookup};
use crate::store::{Lead, LeadSource, Store};

/// Meta only serves a lead for about 90 days. Older leads vanish from Meta's lists without
/// having been deleted, so only leads younger than this are ever considered for removal.
const SAFE_AGE: Duration = Duration::days(80);
/// Leads read per form. A form with more is "incomplete" and is never used to remove anything.
const MAX_PER_FORM: usize = 200;
/// Individual existence checks per import, to bound the number of Graph calls.
const MAX_CHECKS: usize = 100;

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct Report {
    /// Leads Meta has that were not stored yet.
    pub added: usize,
    /// Stored leads whose data was re-read from Meta.
    pub refreshed: usize,
    /// Stored leads Meta confirmed are gone.
    pub removed: usize,
    /// Forms that could not be read, or had more leads than the cap; their leads were not
    /// checked for deletion.
    pub skipped_forms: usize,
    /// Leads missing from a list whose existence check was inconclusive; they were kept.
    pub unverified: usize,
}

/// Stored leads of `form_id` that are missing from Meta's list and recent enough that
/// Meta should still have them: the ones worth asking Meta about individually.
pub fn deletion_candidates(
    stored: &[Lead],
    form_id: &str,
    listed: &HashSet<&str>,
    now: DateTime<Utc>,
) -> Vec<String> {
    stored
        .iter()
        .filter(|l| l.form_id.as_deref() == Some(form_id))
        .filter(|l| !listed.contains(l.id.as_str()))
        .filter(|l| l.created_time > now - SAFE_AGE)
        .map(|l| l.id.clone())
        .collect()
}

pub async fn import(
    graph: &Graph,
    store: &Store,
    page_id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Report> {
    let forms = graph.all_leads(page_id, MAX_PER_FORM).await?;
    let mut report = Report::default();
    let mut checks_left = MAX_CHECKS;
    for fetch in forms {
        let form = match fetch {
            FormFetch::Read(form) => form,
            FormFetch::Failed { .. } => {
                report.skipped_forms += 1;
                continue;
            }
        };
        let listed: HashSet<String> = form.leads.iter().map(|l| l.id.clone()).collect();
        for lead in form.leads {
            store_lead(
                store,
                page_id,
                &form.form_id,
                form.form_name.clone(),
                lead,
                &mut report,
            )
            .await?;
        }
        if !form.complete {
            report.skipped_forms += 1;
            continue;
        }
        // An empty list for a form we hold leads for is more likely a problem than a mass
        // deletion, so nothing is removed on its strength.
        if listed.is_empty() {
            continue;
        }
        let listed: HashSet<&str> = listed.iter().map(String::as_str).collect();
        let stored = store.list_leads().await;
        for id in deletion_candidates(&stored, &form.form_id, &listed, now) {
            if checks_left == 0 {
                report.unverified += 1;
                continue;
            }
            checks_left -= 1;
            match graph.lookup_lead(&id).await {
                LeadLookup::Gone => {
                    store.remove_lead(&id).await?;
                    report.removed += 1;
                }
                LeadLookup::Found(lead) => {
                    let name = form.form_name.clone();
                    store_lead(store, page_id, &form.form_id, name, *lead, &mut report).await?;
                }
                LeadLookup::Unknown => report.unverified += 1,
            }
        }
    }
    Ok(report)
}

/// Saves a lead as Meta describes it now, keeping how and when it first arrived here.
async fn store_lead(
    store: &Store,
    page_id: &str,
    form_id: &str,
    form_name: Option<String>,
    graph_lead: GraphLead,
    report: &mut Report,
) -> anyhow::Result<()> {
    let mut lead = graph_lead.into_lead(LeadSource::Sync, Utc::now());
    lead.form_id = lead.form_id.or_else(|| Some(form_id.to_string()));
    lead.page_id = Some(page_id.to_string());
    lead.form_name = form_name;
    match store.get_lead(&lead.id).await {
        Some(old) => {
            lead.source = old.source;
            lead.received_at = old.received_at;
            report.refreshed += 1;
        }
        None => report.added += 1,
    }
    store.save_lead(lead).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lead(id: &str, form: Option<&str>, age_days: i64, now: DateTime<Utc>) -> Lead {
        let mut l = Lead::bare(id.into(), now - Duration::days(age_days), LeadSource::Sync);
        l.form_id = form.map(str::to_string);
        l
    }

    #[test]
    fn only_recent_unlisted_leads_of_the_form_are_candidates() {
        let now = Utc::now();
        let stored = vec![
            lead("listed", Some("f1"), 1, now),
            lead("gone", Some("f1"), 1, now),
            lead("old", Some("f1"), 200, now),
            lead("other-form", Some("f2"), 1, now),
            lead("no-form", None, 1, now),
        ];
        let listed: HashSet<&str> = ["listed"].into();
        assert_eq!(deletion_candidates(&stored, "f1", &listed, now), ["gone"]);
    }
}
