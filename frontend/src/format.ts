const ESCAPES: Readonly<Record<string, string>> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

export const esc = (s: unknown): string => String(s ?? "").replace(/[&<>"']/g, (c) => ESCAPES[c] ?? c);

export function fmtDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export function relTime(iso: string, now: number = Date.now()): string {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return "";
  const s = (now - t) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return new Date(t).toLocaleDateString();
}

/** Renders a field value as HTML, turning emails, phone numbers and URLs into links. */
export function linkify(name: string, value: string): string {
  const v = esc(value);
  if (/email/i.test(name) && /^[^\s@]+@[^\s@]+$/.test(value)) return `<a href="mailto:${v}">${v}</a>`;
  if (/phone/i.test(name) && /^[+\d][\d\s().-]{5,}$/.test(value)) {
    const tel = value.replace(/[^\d+]/g, "");
    return `<a href="tel:${esc(tel)}">${v}</a> · <a href="https://wa.me/${esc(tel.replace("+", ""))}" target="_blank" rel="noopener">WhatsApp</a>`;
  }
  if (/^https?:\/\/\S+$/.test(value)) return `<a href="${v}" target="_blank" rel="noopener">${v}</a>`;
  return v;
}

export function urlBase64ToUint8Array(base64: string): Uint8Array {
  const pad = "=".repeat((4 - (base64.length % 4)) % 4);
  const raw = atob((base64 + pad).replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(raw, (c) => c.charCodeAt(0));
}
