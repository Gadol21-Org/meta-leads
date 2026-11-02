import { esc, fmtDate, linkify, relTime } from "./format";
import type { AppConfig, Lead, LeadSummary } from "./types";

export function listView(leads: LeadSummary[], config: AppConfig): string {
  const banner = config.metaConfigured
    ? ""
    : `<p class="banner">Meta page token isn't configured, so leads only show their ids. See README.</p>`;
  if (!leads.length) {
    return `${banner}<p class="empty">No leads yet. New leads appear here and as notifications.</p>`;
  }
  const rows = leads
    .map(
      (l) => `
      <li><a href="/leads/${encodeURIComponent(l.id)}" data-link class="lead-row">
        <div>
          <div class="lead-name">${esc(l.title)}</div>
          <div class="muted small">${esc([l.form_name, l.campaign_name].filter(Boolean).join(" · ") || l.id)}</div>
        </div>
        <div class="right muted small">${esc(relTime(l.created_time))}</div>
      </a></li>`,
    )
    .join("");
  return `${banner}<ul class="lead-list">${rows}</ul>`;
}

export function leadView(lead: Lead): string {
  const adRows: [string, string | undefined][] = [
    ["Received", fmtDate(lead.created_time)],
    ["Form", lead.form_name ?? lead.form_id],
    ["Campaign", lead.campaign_name ?? undefined],
    ["Ad set", lead.adset_name],
    ["Ad", lead.ad_name ?? lead.ad_id],
    ["Platform", lead.platform?.toUpperCase()],
    ["Organic", lead.is_organic === undefined ? undefined : lead.is_organic ? "Yes" : "No"],
    ["Partner", lead.partner_name],
    ["Lead id", lead.id],
  ];
  const disclaimers = (lead.custom_disclaimer_responses ?? []).map(
    (d) => [d.checkbox_key ?? "", d.is_checked === "1" || d.is_checked === true ? "Checked" : "Not checked"] as const,
  );
  return `
    <a href="/" data-link class="back">← All leads</a>
    <section class="card">
      <h1>${esc(lead.title)}</h1>
      ${lead.fetch_error ? `<p class="banner">${esc(lead.fetch_error)} <button class="link" id="refresh-btn">Retry</button></p>` : ""}
      <dl class="fields">
        ${lead.fields
          .map((f) => `<dt>${esc(f.label)}</dt><dd>${linkify(f.name, f.value) || '<span class="muted">—</span>'}</dd>`)
          .join("")}
        ${disclaimers.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join("")}
      </dl>
    </section>
    <section class="card">
      <h2>Ad details</h2>
      <dl class="fields">${adRows
        .filter(([, v]) => v)
        .map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`)
        .join("")}</dl>
    </section>`;
}
