import { LOCALE, t } from "./strings";
import type { ImportReport } from "./types";

const ESCAPES: Readonly<Record<string, string>> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

export const esc = (s: unknown): string => String(s ?? "").replace(/[&<>"']/g, (c) => ESCAPES[c] ?? c);

export function fmtDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleString(LOCALE, { dateStyle: "medium", timeStyle: "short" });
}

export function relTime(iso: string, now: number = Date.now()): string {
  const at = new Date(iso).getTime();
  if (Number.isNaN(at)) return "";
  const s = (now - at) / 1000;
  if (s < 60) return t.time.now;
  if (s < 3600) return t.time.minutes(Math.floor(s / 60));
  if (s < 86400) return t.time.hours(Math.floor(s / 3600));
  return new Date(at).toLocaleDateString(LOCALE);
}

/** Renders a field value as HTML, turning emails, phone numbers and URLs into links. */
export function linkify(name: string, value: string): string {
  const v = esc(value);
  // Emails, numbers and URLs always read left to right, even inside right-to-left content.
  const ltr = `<bdi dir="ltr">${v}</bdi>`;
  if (/email/i.test(name) && /^[^\s@]+@[^\s@]+$/.test(value)) return `<a href="mailto:${v}">${ltr}</a>`;
  if (/phone/i.test(name) && /^[+\d][\d\s().-]{5,}$/.test(value)) {
    const tel = value.replace(/[^\d+]/g, "");
    return `<a href="tel:${esc(tel)}">${ltr}</a> · <a href="https://wa.me/${esc(tel.replace("+", ""))}" target="_blank" rel="noopener">${t.lead.whatsapp}</a>`;
  }
  if (/^https?:\/\/\S+$/.test(value)) return `<a href="${v}" target="_blank" rel="noopener">${ltr}</a>`;
  return `<bdi>${v}</bdi>`;
}

export function urlBase64ToUint8Array(base64: string): Uint8Array {
  const pad = "=".repeat((4 - (base64.length % 4)) % 4);
  const raw = atob((base64 + pad).replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(raw, (c) => c.charCodeAt(0));
}

/** One toast line for a sync: what changed, and what could not be checked. */
export function importSummary(r: ImportReport): string {
  let text = r.added > 0 || r.removed > 0 ? t.sync.changes(r.added, r.removed) : t.sync.upToDate;
  if (r.skipped_forms > 0) text += ` ${t.sync.skippedForms(r.skipped_forms)}`;
  if (r.unverified > 0) text += ` ${t.sync.unverified(r.unverified)}`;
  return text;
}

export type PlatformKind = "instagram" | "facebook" | "other";

/** Meta sends "ig", "fb" and similar short codes; undefined when it did not say. */
export function platformInfo(platform: string | null | undefined): { kind: PlatformKind; label: string } | undefined {
  const p = platform?.trim().toLowerCase();
  if (!p) return undefined;
  if (p === "ig" || p === "instagram") return { kind: "instagram", label: "Instagram" };
  if (p === "fb" || p === "facebook") return { kind: "facebook", label: "Facebook" };
  return { kind: "other", label: p.charAt(0).toUpperCase() + p.slice(1) };
}

export function platformBadge(platform: string | null | undefined): string {
  const info = platformInfo(platform);
  return info ? `<span class="badge ${info.kind}">${esc(info.label)}</span>` : "";
}

/** Text of unknown direction (names, numbers, ids) isolated so it reads correctly inside right-to-left content. */
export function bidi(text: string): string {
  return `<bdi>${esc(text)}</bdi>`;
}
