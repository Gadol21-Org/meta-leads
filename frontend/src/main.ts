import "./style.css";
import * as api from "./api";
import * as push from "./push";
import { leadView, listView } from "./views";
import { esc } from "./format";
import { isWorkerMessage, type AppConfig } from "./types";

function $<T extends HTMLElement>(sel: string): T {
  const el = document.querySelector<T>(sel);
  if (!el) throw new Error(`missing element ${sel}`);
  return el;
}
const view = $("#view");
const menu = $("#menu");
const notifyBtn = $<HTMLButtonElement>("#notify-btn");
let config: AppConfig;

const errorMessage = (e: unknown): string => (e instanceof Error ? e.message : String(e));

let toastTimer: number | undefined;

function toast(msg: string, ms = 3000): void {
  const t = $("#toast");
  t.textContent = msg;
  t.hidden = false;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (t.hidden = true), ms);
}

// ---------- routing ----------

function navigate(url: string): void {
  history.pushState({}, "", url);
  void route();
}

document.addEventListener("click", (e) => {
  const a = e.target instanceof Element ? e.target.closest<HTMLAnchorElement>("a[data-link]") : null;
  if (a && a.origin === location.origin && !e.metaKey && !e.ctrlKey) {
    e.preventDefault();
    navigate(a.pathname + a.search);
  }
});
window.addEventListener("popstate", () => void route());

async function route(): Promise<void> {
  menu.hidden = true;
  const leadId = location.pathname.match(/^\/leads\/([^/]+)$/)?.[1];
  try {
    if (leadId) await renderLead(decodeURIComponent(leadId));
    else view.innerHTML = listView(await api.listLeads(), config);
  } catch (err) {
    view.innerHTML = `<p class="empty">Something went wrong: ${esc(errorMessage(err))}</p>`;
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
    if (err instanceof api.ApiError && err.status === 404) {
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
    toast(errorMessage(e));
  }
});

$("#menu-btn").addEventListener("click", () => (menu.hidden = !menu.hidden));
menu.addEventListener("click", async (e) => {
  const action = e.target instanceof HTMLElement ? e.target.dataset.action : undefined;
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
    toast(errorMessage(err));
  }
});

// The service worker tells us to navigate when a notification is tapped while
// the app is open, and when a new lead arrives.
navigator.serviceWorker?.addEventListener("message", (e: MessageEvent<unknown>) => {
  if (!isWorkerMessage(e.data)) return;
  if (e.data.type === "open") navigate(e.data.url);
  else if (location.pathname === "/") void route();
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
  view.innerHTML = `<p class="empty">Could not start: ${esc(errorMessage(err))}</p>`;
});
