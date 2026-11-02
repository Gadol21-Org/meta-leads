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
/// Most stored leads checked for removal when Meta lists nothing for them (see `import`).
const MAX_UNLISTED_REMOVALS: usize = 20;
/// Individual existence checks per import, to bound the number of Graph calls.
const MAX_CHECKS: usize = 100;

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
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

/// The date the 80-day rule is measured from. For a lead that came by webhook, `created_time` is
/// whatever the event said (Meta's dashboard test payload says 2020), so the time it arrived here
/// is used; for a synced lead Meta's own `created_time` is the reliable one.
fn age_reference(lead: &Lead) -> DateTime<Utc> {
    match lead.source {
        LeadSource::Webhook => lead.received_at,
        LeadSource::Sync => lead.created_time,
    }
}

fn recent_enough(lead: &Lead, now: DateTime<Utc>) -> bool {
    age_reference(lead) > now - SAFE_AGE
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
        .filter(|l| recent_enough(l, now))
        .map(|l| l.id.clone())
        .collect()
}

/// Stored leads whose form is not one of the Page's forms (or that have no form at all), such as
/// the made-up sample Meta's dashboard sends. No form list can vouch for them, so they are
/// worth the same individual check. Forms that failed to read still count as the Page's forms.
pub fn orphan_candidates(
    stored: &[Lead],
    page_forms: &HashSet<&str>,
    listed: &HashSet<&str>,
    now: DateTime<Utc>,
) -> Vec<String> {
    stored
        .iter()
        .filter(|l| l.form_id.as_deref().is_none_or(|f| !page_forms.contains(f)))
        .filter(|l| !listed.contains(l.id.as_str()))
        .filter(|l| recent_enough(l, now))
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
    let mut page_forms: HashSet<String> = HashSet::new();
    let mut listed_all: HashSet<String> = HashSet::new();
    for fetch in forms {
        let form = match fetch {
            FormFetch::Read(form) => form,
            FormFetch::Failed { form_id, .. } => {
                page_forms.insert(form_id);
                report.skipped_forms += 1;
                continue;
            }
        };
        page_forms.insert(form.form_id.clone());
        let listed: HashSet<String> = form.leads.iter().map(|l| l.id.clone()).collect();
        listed_all.extend(listed.iter().cloned());
        for lead in form.leads {
            let name = form.form_name.clone();
            store_lead(store, page_id, Some(&form.form_id), name, lead, &mut report).await?;
        }
        if !form.complete {
            tracing::info!(
                "import: form {} has more than {MAX_PER_FORM} leads; not checking it for deletions",
                form.form_id
            );
            report.skipped_forms += 1;
            continue;
        }
        let stored = store.list_leads().await;
        let listed: HashSet<&str> = listed.iter().map(String::as_str).collect();
        let candidates = deletion_candidates(&stored, &form.form_id, &listed, now);
        tracing::info!(
            "import: form {}: {} listed by Meta, {} stored leads to verify",
            form.form_id,
            listed.len(),
            candidates.len()
        );
        // An empty list can be the truth (the form's only leads were deleted), and each removal is
        // still confirmed by Meta lead by lead. But a long run of "missing" leads looks like a
        // Meta-side problem, so then nothing is removed.
        if candidates.len() > MAX_UNLISTED_REMOVALS && listed.is_empty() {
            tracing::warn!(
                "import: Meta returned no leads for form {} but {} are stored; keeping them",
                form.form_id,
                candidates.len()
            );
            continue;
        }
        let name = form.form_name.clone();
        check_leads(
            graph,
            store,
            page_id,
            name,
            candidates,
            &mut checks_left,
            &mut report,
        )
        .await?;
    }

    let stored = store.list_leads().await;
    let page_forms: HashSet<&str> = page_forms.iter().map(String::as_str).collect();
    let listed_all: HashSet<&str> = listed_all.iter().map(String::as_str).collect();
    let orphans = orphan_candidates(&stored, &page_forms, &listed_all, now);
    if orphans.len() > MAX_UNLISTED_REMOVALS {
        tracing::warn!(
            "import: {} stored leads belong to no form on the Page; keeping them",
            orphans.len()
        );
    } else {
        check_leads(
            graph,
            store,
            page_id,
            None,
            orphans,
            &mut checks_left,
            &mut report,
        )
        .await?;
    }
    Ok(report)
}

/// Asks Meta about each lead by id: removes the ones it says do not exist, refreshes the ones it
/// still has, and keeps (and counts) the ones the check could not settle.
async fn check_leads(
    graph: &Graph,
    store: &Store,
    page_id: &str,
    form_name: Option<String>,
    ids: Vec<String>,
    checks_left: &mut usize,
    report: &mut Report,
) -> anyhow::Result<()> {
    for id in ids {
        if *checks_left == 0 {
            report.unverified += 1;
            continue;
        }
        *checks_left -= 1;
        match graph.lookup_lead(&id).await {
            LeadLookup::Gone => {
                tracing::info!("import: removing lead {id}: Meta says it no longer exists");
                store.remove_lead(&id).await?;
                report.removed += 1;
            }
            LeadLookup::Found(lead) => {
                let name = form_name.clone();
                store_lead(store, page_id, None, name, *lead, report).await?;
            }
            LeadLookup::Unknown => report.unverified += 1,
        }
    }
    Ok(())
}

/// Saves a lead as Meta describes it now, keeping how and when it first arrived here.
async fn store_lead(
    store: &Store,
    page_id: &str,
    form_id: Option<&str>,
    form_name: Option<String>,
    graph_lead: GraphLead,
    report: &mut Report,
) -> anyhow::Result<()> {
    let mut lead = graph_lead.into_lead(LeadSource::Sync, Utc::now());
    lead.form_id = lead.form_id.or_else(|| form_id.map(str::to_string));
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

    #[test]
    fn webhook_leads_are_aged_from_when_they_arrived_not_from_the_event_time() {
        let now = Utc::now();
        // Meta's dashboard sample says the lead was created in 2020; it arrived a minute ago.
        let mut sample = lead("sample", Some("4444"), 2000, now);
        sample.source = LeadSource::Webhook;
        sample.received_at = now - Duration::minutes(1);
        let mut stale = lead("stale", Some("4444"), 1, now);
        stale.source = LeadSource::Webhook;
        stale.received_at = now - Duration::days(100);
        let stored = vec![
            sample,
            stale,
            lead("on-page", Some("f1"), 1, now),
            lead("no-form", None, 1, now),
        ];
        let page_forms: HashSet<&str> = ["f1"].into();
        let listed: HashSet<&str> = HashSet::new();
        assert_eq!(
            orphan_candidates(&stored, &page_forms, &listed, now),
            ["sample", "no-form"]
        );
    }
}
