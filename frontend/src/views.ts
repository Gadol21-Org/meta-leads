import { bidi, esc, fmtDate, linkify, platformBadge, relTime } from "./format";
import { t } from "./strings";
import type { AppConfig, Lead, LeadSummary } from "./types";

/** A visible back button. It steps back through the app's own history when there is some (so the
 * phone's back gesture and this button agree), and goes to the list otherwise. */
export function backLink(): string {
  return `<a href="/" data-link data-back class="back"><span class="arrow" aria-hidden="true">←</span> ${t.nav.backToLeads}</a>`;
}

export function listView(leads: LeadSummary[], config: AppConfig): string {
  const banner = config.metaConfigured
    ? ""
    : `<p class="banner">${t.list.noToken}</p>`;
  if (!leads.length) {
    return `${banner}<p class="empty">${t.list.empty}</p>`;
  }
  const rows = leads
    .map(
      (l) => `
      <li><a href="/leads/${encodeURIComponent(l.id)}" data-link class="lead-row">
        <div>
          <div class="lead-name">${bidi(l.title)}</div>
          <div class="muted small">${bidi([l.form_name, l.campaign_name].filter(Boolean).join(" · ") || l.id)}</div>
        </div>
        <div class="date muted small">${bidi(relTime(l.created_time))}${platformBadge(l.platform)}</div>
      </a></li>`,
    )
    .join("");
  return `${banner}<ul class="lead-list">${rows}</ul>`;
}

export function leadView(lead: Lead): string {
  const adRows: [string, string | undefined][] = [
    [t.lead.received, fmtDate(lead.created_time)],
    [t.lead.form, lead.form_name ?? lead.form_id],
    [t.lead.campaign, lead.campaign_name ?? undefined],
    [t.lead.adSet, lead.adset_name],
    [t.lead.ad, lead.ad_name ?? lead.ad_id],
    [t.lead.organic, lead.is_organic === undefined ? undefined : lead.is_organic ? t.lead.yes : t.lead.no],
    [t.lead.partner, lead.partner_name],
    [t.lead.leadId, lead.id],
  ];
  const disclaimers = (lead.disclaimers ?? []).map((d) => [d.key, d.checked ? t.lead.checked : t.lead.notChecked] as const);
  return `
    ${backLink()}
    <section class="card">
      <h1>${bidi(lead.title)} ${platformBadge(lead.platform)}</h1>
      ${lead.fetch_error ? `<p class="banner">${esc(lead.fetch_error)} <button class="link" id="refresh-btn">${t.lead.retry}</button></p>` : ""}
      <dl class="fields">
        ${lead.fields
          .map((f) => `<dt>${bidi(f.label)}</dt><dd>${linkify(f.name, f.value) || '<span class="muted">—</span>'}</dd>`)
          .join("")}
        ${disclaimers.map(([k, v]) => `<dt>${bidi(k)}</dt><dd>${bidi(v ?? "")}</dd>`).join("")}
      </dl>
    </section>
    <section class="card">
      <h2>${t.lead.adDetails}</h2>
      <dl class="fields">${adRows
        .filter(([, v]) => v)
        .map(([k, v]) => `<dt>${bidi(k)}</dt><dd>${bidi(v ?? "")}</dd>`)
        .join("")}</dl>
    </section>`;
}
