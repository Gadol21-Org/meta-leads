import "./style.css";
import * as api from "./api";
import * as push from "./push";
import { leadView, listView } from "./views";
import { esc } from "./format";
import type { AppConfig } from "./types";

const $ = <T extends HTMLElement>(sel: string) => document.querySelector<T>(sel)!;
const view = $("#view");
const menu = $("#menu");
const notifyBtn = $<HTMLButtonElement>("#notify-btn");
let config: AppConfig;

function toast(msg: string, ms = 3000): void {
  const t = $("#toast");
  t.textContent = msg;
  t.hidden = false;
  clearTimeout((toast as unknown as { timer?: number }).timer);
  (toast as unknown as { timer?: number }).timer = window.setTimeout(() => (t.hidden = true), ms);
}

// ---------- routing ----------

function navigate(url: string): void {
  history.pushState({}, "", url);
  void route();
}

document.addEventListener("click", (e) => {
  const a = (e.target as Element).closest<HTMLAnchorElement>("a[data-link]");
  if (a && a.origin === location.origin && !e.metaKey && !e.ctrlKey) {
    e.preventDefault();
    navigate(a.pathname + a.search);
  }
});
window.addEventListener("popstate", () => void route());

async function route(): Promise<void> {
  menu.hidden = true;
  const m = location.pathname.match(/^\/leads\/([^/]+)$/);
  try {
    if (m) await renderLead(decodeURIComponent(m[1]));
    else view.innerHTML = listView(await api.listLeads(), config);
  } catch (err) {
    view.innerHTML = `<p class="empty">Something went wrong: ${esc((err as Error).message)}</p>`;
  }
}

async function renderLead(id: string): Promise<void> {
  view.innerHTML = `<p class="empty">Loading…</p>`;
  try {
    const lead = await api.getLead(id);
    view.innerHTML = leadView(lead);
    document.querySelector("#refresh-btn")?.addEventListener("click", async () => {
      await api.getLead(id, true);
      await renderLead(id);
    });
  } catch (err) {
    if ((err as api.ApiError).status === 404) {
      view.innerHTML = `<p class="empty">This lead isn't here yet. <a href="/" data-link>Back to leads</a></p>`;
      return;
    }
    throw err;
  }
}

// ---------- menu and notifications ----------

notifyBtn.addEventListener("click", async () => {
  try {
    if (await push.subscribe(config)) toast("Notifications are on for this device.");
    else toast("Notifications were not allowed.");
    await push.updateNotifyButton(notifyBtn, config);
  } catch (e) {
    toast((e as Error).message);
  }
});

$("#menu-btn").addEventListener("click", () => (menu.hidden = !menu.hidden));
menu.addEventListener("click", async (e) => {
  const action = (e.target as HTMLElement).dataset.action;
  if (!action) return;
  menu.hidden = true;
  try {
    if (action === "sync") {
      toast("Importing leads from Meta…", 10000);
      const { added } = await api.syncLeads();
      toast(`Imported ${added} lead${added === 1 ? "" : "s"}.`);
      await route();
    } else if (action === "test-push") {
      const { sent } = await api.sendTestPush();
      toast(`Sent to ${sent} device${sent === 1 ? "" : "s"}.`);
    } else if (action === "unsubscribe") {
      toast((await push.unsubscribe()) ? "Notifications are off for this device." : "This device is not subscribed.");
      await push.updateNotifyButton(notifyBtn, config);
    }
  } catch (err) {
    toast((err as Error).message);
  }
});

// The service worker tells us to navigate when a notification is tapped while
// the app is open, and when a new lead arrives.
navigator.serviceWorker?.addEventListener("message", (e: MessageEvent) => {
  if (e.data?.type === "open" && e.data.url) navigate(e.data.url);
  if (e.data?.type === "new-lead" && location.pathname === "/") void route();
});

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible" && config && location.pathname === "/") void route();
});

// ---------- boot ----------

async function boot(): Promise<void> {
  config = await api.getConfig();
  $("#menu-btn").hidden = false;
  if ("serviceWorker" in navigator) {
    await navigator.serviceWorker.register("/sw.js");
    push.updateNotifyButton(notifyBtn, config).catch(() => {});
  }
  await route();
}

boot().catch((err) => {
  view.innerHTML = `<p class="empty">Could not start: ${esc((err as Error).message)}</p>`;
});
