// Pull to refresh (touch screens): the page follows the finger from the top with rubber-band
// resistance, a spinner rides along, a short haptic tick marks the threshold (navigator.vibrate,
// unavailable on iOS Safari) and release glides back with an eased transition.

const THRESHOLD = 72;
const MAX = 120;

// iOS Safari ignores user-scalable=no and touch-action for pinch, but still fires these events.
for (const g of ["gesturestart", "gesturechange", "gestureend"]) {
  document.addEventListener(g, (e) => e.preventDefault());
}

/** `refresh` resolves to false when it failed (the user feels a double buzz). */
export function enablePullToRefresh(content: HTMLElement, refresh: () => Promise<boolean>): void {
  const spinner = document.createElement("div");
  spinner.className = "ptr";
  spinner.innerHTML = `<div class="ptr-dot"></div>`;
  document.body.append(spinner);

  const root = document.documentElement.style;
  let startY: number | undefined;
  let dist = 0;
  let armed = false;
  let busy = false;

  const setPull = (d: number): void => {
    root.setProperty("--pull", `${d}px`);
    root.setProperty("--pull-p", String(Math.min(1, d / THRESHOLD)));
  };
  const glideTo = (d: number): void => {
    document.body.classList.add("gliding");
    setPull(d);
  };

  document.addEventListener(
    "touchstart",
    (e) => {
      const first = e.touches[0];
      const open = document.querySelector("dialog[open]") !== null;
      startY = first && e.touches.length === 1 && window.scrollY <= 0 && !open && !busy ? first.clientY : undefined;
      dist = 0;
      armed = false;
      document.body.classList.remove("gliding");
    },
    { passive: true },
  );

  document.addEventListener(
    "touchmove",
    (e) => {
      const touch = e.touches[0];
      if (startY === undefined || !touch) return;
      const dy = touch.clientY - startY;
      if (dy <= 0 || window.scrollY > 0) {
        if (dist === 0) startY = undefined;
        else setPull((dist = 0));
        return;
      }
      e.preventDefault();
      // Rubber band: resistance grows as the page is pulled further.
      dist = MAX * (1 - Math.exp(-dy / (MAX * 1.6)));
      setPull(dist);
      if (!armed && dist >= THRESHOLD) navigator.vibrate?.(12);
      armed = dist >= THRESHOLD;
      spinner.classList.toggle("ready", armed);
    },
    { passive: false },
  );

  const end = async (): Promise<void> => {
    if (startY === undefined) return;
    startY = undefined;
    if (!armed) return glideTo(0);
    busy = true;
    spinner.classList.add("spin");
    glideTo(THRESHOLD * 0.8);
    // Show the spinner for a moment even when the reload is instant.
    const [ok] = await Promise.all([refresh(), new Promise((r) => setTimeout(r, 600))]);
    if (!ok) navigator.vibrate?.([30, 40, 30]);
    spinner.classList.remove("spin", "ready");
    glideTo(0);
    armed = false;
    busy = false;
  };
  document.addEventListener("touchend", () => void end());
  document.addEventListener("touchcancel", () => void end());
  content.setAttribute("data-pull", "");
}

const EDGE = 28;
const SWIPE_MIN = 70;

/** iOS reserves the left-edge swipe for "back", which points the wrong way in a right-to-left app, and
 * apps cannot change that. So the app also goes back on a swipe in from the right edge, matching the
 * back button. `canGoBack` says whether the app has a page of its own to return to. */
export function enableEdgeSwipeBack(canGoBack: () => boolean, goBack: () => void): void {
  let start: { x: number; y: number } | undefined;
  document.addEventListener(
    "touchstart",
    (e) => {
      const first = e.touches[0];
      const fromRightEdge = first !== undefined && first.clientX >= window.innerWidth - EDGE;
      const open = document.querySelector("dialog[open]") !== null;
      start = fromRightEdge && e.touches.length === 1 && !open && canGoBack() ? { x: first.clientX, y: first.clientY } : undefined;
    },
    { passive: true },
  );
  document.addEventListener(
    "touchend",
    (e) => {
      const touch = e.changedTouches[0];
      const from = start;
      start = undefined;
      if (!from || !touch) return;
      const dx = touch.clientX - from.x;
      // A deliberate, mostly horizontal swipe towards the inside of the screen.
      if (dx <= -SWIPE_MIN && Math.abs(dx) > 2 * Math.abs(touch.clientY - from.y)) goBack();
    },
    { passive: true },
  );
  document.addEventListener("touchcancel", () => (start = undefined));
}
