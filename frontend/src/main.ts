import "./style.css";
import * as api from "./api";
import * as push from "./push";
import { computeStats, parseRange } from "./stats";
import { statsView } from "./stats-view";
import { leadView, listView } from "./views";
import { esc, importSummary } from "./format";
import { enablePullToRefresh } from "./gestures";
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

// Tapping a bar in a stats chart shows its value in the line above the chart.
view.addEventListener("click", (e) => {
  const tip = e.target instanceof Element ? e.target.closest<SVGElement>("[data-tip]")?.dataset.tip : undefined;
  const readout = e.target instanceof Element ? e.target.closest(".card")?.querySelector(".readout") : null;
  if (tip && readout) readout.textContent = tip;
});

document.addEventListener("click", (e) => {
  const a = e.target instanceof Element ? e.target.closest<HTMLAnchorElement>("a[data-link]") : null;
  if (a && a.origin === location.origin && !e.metaKey && !e.ctrlKey) {
    e.preventDefault();
    navigate(a.pathname + a.search);
  }
});
// A long press on a lead row opens the browser's link menu (Android); the rows are tap targets only.
document.addEventListener("contextmenu", (e) => {
  if (e.target instanceof Element && e.target.closest(".lead-row")) e.preventDefault();
});
window.addEventListener("popstate", () => void route());

/** Shows the current page. `quiet` (a pull to refresh or a resume) keeps what is on screen
 * while loading and, on failure, keeps it and says so. Resolves to false on failure. */
async function route(quiet = false): Promise<boolean> {
  menu.hidden = true;
  const leadId = location.pathname.match(/^\/leads\/([^/]+)$/)?.[1];
  try {
    if (location.pathname === "/stats") {
      const days = parseRange(new URLSearchParams(location.search).get("days"));
      view.innerHTML = statsView(computeStats(await api.listLeads(), days));
    } else if (leadId) await renderLead(decodeURIComponent(leadId), quiet);
    else view.innerHTML = listView(await api.listLeads(), config);
    return true;
  } catch (err) {
    if (quiet) toast(errorMessage(err));
    else view.innerHTML = `<p class="empty">Something went wrong: ${esc(errorMessage(err))}</p>`;
    return false;
  }
}

async function renderLead(id: string, quiet = false): Promise<void> {
  if (!quiet) view.innerHTML = `<p class="empty">Loading…</p>`;
  try {
    const lead = await api.getLead(id);
    view.innerHTML = leadView(lead);
    document.querySelector("#refresh-btn")?.addEventListener("click", async () => {
      await api.getLead(id, true);
      await renderLead(id);
    });
  } catch (err) {
    if (err instanceof api.ApiError && err.status === 404) {
      view.innerHTML = `<p class="empty">This lead isn't here (it may have been removed in Meta). <a href="/" data-link>Back to leads</a></p>`;
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
    if (action === "test-push") {
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

// An installed PWA is usually resumed from the background, not reloaded: look for a new version
// and re-read the data whenever it becomes visible again (throttled against bursts).
let lastLoad = Date.now();

function onResume(): void {
  if (document.visibilityState !== "visible" || !config || Date.now() - lastLoad < 5000) return;
  lastLoad = Date.now();
  void swRegistration?.update().catch(() => {});
  void route(true);
}
document.addEventListener("visibilitychange", onResume);
window.addEventListener("pageshow", (e) => e.persisted && onResume());
window.addEventListener("online", onResume);

/** Pull to refresh: show the local data at once, then sync with Meta (the server answers repeated
 * syncs within 30 seconds from the last one, so pulling often is cheap). A failure keeps what is on screen. */
async function pullRefresh(): Promise<boolean> {
  lastLoad = Date.now();
  const shown = await route(true);
  if (!config.metaConfigured) return shown;
  try {
    toast(importSummary(await api.syncLeads()), 6000);
  } catch (err) {
    toast(`Could not sync with Meta: ${errorMessage(err)}`, 6000);
    return false;
  }
  lastLoad = Date.now();
  return route(true);
}
enablePullToRefresh(view, pullRefresh);

// ---------- updates ----------

let swRegistration: ServiceWorkerRegistration | undefined;

/** The service worker takes over as soon as a new version installs (skipWaiting and claim); reload
 * once so the page runs the new code too. Not on the very first install, which has nothing to replace. */
function reloadOnUpdate(): void {
  const sw = navigator.serviceWorker;
  if (!sw) return;
  let hadController = Boolean(sw.controller);
  let reloading = false;
  sw.addEventListener("controllerchange", () => {
    if (!hadController) {
      hadController = true; // the first install claiming this page
      return;
    }
    if (reloading || document.querySelector("dialog[open]")) return;
    reloading = true;
    location.reload();
  });
}

// ---------- boot ----------

async function boot(): Promise<void> {
  config = await api.getConfig();
  $("#menu-btn").hidden = false;
  if ("serviceWorker" in navigator) {
    reloadOnUpdate();
    // updateViaCache "none": always ask the server whether sw.js changed, so every open picks up a release.
    swRegistration = await navigator.serviceWorker.register("/sw.js", { updateViaCache: "none" });
    void swRegistration.update().catch(() => {});
    push.updateNotifyButton(notifyBtn, config).catch(() => {});
  }
  await route();
}

boot().catch((err) => {
  view.innerHTML = `<p class="empty">Could not start: ${esc(errorMessage(err))}</p>`;
});
