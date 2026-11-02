import * as api from "./api";
import { urlBase64ToUint8Array } from "./format";
import { t } from "./strings";
import type { AppConfig } from "./types";

const supported = () => "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;

async function currentSubscription(): Promise<PushSubscription | null> {
  if (!supported()) return null;
  const reg = await navigator.serviceWorker.ready;
  return reg.pushManager.getSubscription();
}

/** Shows or hides the t.notifications.enable button to match this device's state. */
export async function updateNotifyButton(btn: HTMLButtonElement, config: AppConfig): Promise<void> {
  if (!config.pushEnabled) {
    btn.hidden = true;
    return;
  }
  if (!supported()) {
    // iOS only exposes Web Push once the app is added to the Home Screen.
    btn.hidden = false;
    btn.disabled = true;
    btn.textContent = /iPhone|iPad/.test(navigator.userAgent) ? t.notifications.addToHomeScreen : t.notifications.unsupported;
    return;
  }
  const sub = await currentSubscription();
  // Re-register so the server always knows this device (cheap and idempotent).
  if (sub) api.saveSubscription(sub.toJSON()).catch(() => {});
  btn.hidden = Boolean(sub) || Notification.permission === "denied";
  btn.disabled = false;
  btn.textContent = t.notifications.enable;
}

export async function subscribe(config: AppConfig): Promise<boolean> {
  if (!config.vapidPublicKey) return false;
  if ((await Notification.requestPermission()) !== "granted") return false;
  const reg = await navigator.serviceWorker.ready;
  const sub = await reg.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: urlBase64ToUint8Array(config.vapidPublicKey) as BufferSource,
  });
  await api.saveSubscription(sub.toJSON());
  return true;
}

/** Returns false when this device wasn't subscribed. */
export async function unsubscribe(): Promise<boolean> {
  const sub = await currentSubscription();
  if (!sub) return false;
  await api.deleteSubscription(sub.endpoint);
  await sub.unsubscribe();
  return true;
}
