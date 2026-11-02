import { describe, expect, it } from "vitest";
import { esc, linkify, relTime, urlBase64ToUint8Array } from "../format";
import { leadView, listView } from "../views";
import { isWorkerMessage, type Lead } from "../types";

describe("format", () => {
  it("escapes html", () => {
    expect(esc(`<a href="x">&'</a>`)).toBe("&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;");
  });

  it("links email, phone and urls only when they look right", () => {
    expect(linkify("email", "a@b.co")).toContain('href="mailto:a@b.co"');
    expect(linkify("phone_number", "+972 50-123-4567")).toContain('href="tel:+972501234567"');
    expect(linkify("phone_number", "+972 50-123-4567")).toContain("wa.me/972501234567");
    expect(linkify("website", "https://x.example/a")).toContain('target="_blank"');
    expect(linkify("email", "not an email")).toBe("not an email");
    expect(linkify("note", "<script>")).toBe("&lt;script&gt;");
  });

  it("formats relative times", () => {
    const now = Date.parse("2026-11-02T12:00:00Z");
    expect(relTime("2026-11-02T11:59:50Z", now)).toBe("just now");
    expect(relTime("2026-11-02T11:30:00Z", now)).toBe("30m ago");
    expect(relTime("2026-11-02T09:00:00Z", now)).toBe("3h ago");
    expect(relTime("garbage", now)).toBe("");
  });

  it("recognises worker messages", () => {
    expect(isWorkerMessage({ type: "open", url: "/leads/1" })).toBe(true);
    expect(isWorkerMessage({ type: "new-lead" })).toBe(true);
    expect(isWorkerMessage({ type: "open" })).toBe(false);
    expect(isWorkerMessage(null)).toBe(false);
    expect(isWorkerMessage("open")).toBe(false);
  });

  it("decodes the VAPID public key", () => {
    expect(Array.from(urlBase64ToUint8Array("AQID"))).toEqual([1, 2, 3]);
  });
});

describe("views", () => {
  const lead: Lead = {
    id: "111",
    title: "Dana <b>Levi</b>",
    created_time: "2026-11-02T08:30:00Z",
    fields: [{ name: "email", label: "Email", value: "dana@example.com" }],
    campaign_name: "Autumn promo",
    platform: "ig",
    is_organic: false,
    source: "webhook",
    received_at: "2026-11-02T08:30:01Z",
  };

  it("shows every field and escapes the title", () => {
    const html = leadView(lead);
    expect(html).toContain("dana@example.com");
    expect(html).toContain("Autumn promo");
    expect(html).toContain("IG");
    expect(html).not.toContain("<b>Levi</b>");
  });

  it("shows disclaimer answers", () => {
    const html = leadView({ ...lead, disclaimers: [{ key: "terms", checked: true }, { key: "ads", checked: false }] });
    expect(html).toContain("Checked");
    expect(html).toContain("Not checked");
  });

  it("offers a retry when the Graph lookup failed", () => {
    expect(leadView({ ...lead, fetch_error: "boom" })).toContain('id="refresh-btn"');
  });

  it("links list rows to the lead page", () => {
    const html = listView([lead], { metaConfigured: true, pushEnabled: true, vapidPublicKey: "k" });
    expect(html).toContain('href="/leads/111"');
    expect(listView([], { metaConfigured: false, pushEnabled: false, vapidPublicKey: null })).toContain("Meta page token");
  });
});
