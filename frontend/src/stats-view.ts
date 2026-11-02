import { bidi, esc, platformInfo } from "./format";
import { backLink } from "./views";
import { changeVsPrevious, RANGES, type Bucket, type DayCount, type Stats } from "./stats";

const count = (n: number, word: string): string => `${n} ${word}${n === 1 ? "" : "s"}`;
const pct = (share: number): string => `${Math.round(share * 100)}%`;
const hourLabel = (h: number): string => `${String(h).padStart(2, "0")}:00`;

function shortDate(key: string): string {
  const [y, m, d] = key.split("-").map(Number);
  return new Date(y ?? 1970, (m ?? 1) - 1, d ?? 1).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** A 4px-rounded top, square at the baseline (see the data-viz mark specs). */
function columnPath(x: number, y: number, w: number, h: number): string {
  const r = Math.min(4, w / 2, h);
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
}

interface Column {
  count: number;
  tip: string;
}

/**
 * A single-series column chart. Bars are at most 24px wide with a 2px gap; the whole slot is the tap target
 * (`data-tip` fills the readout line) and the `<title>` serves a mouse. `labels` are x-axis texts by column index.
 */
function columnChart(columns: Column[], labels: Map<number, string>): string {
  const W = 336;
  const H = 120;
  const left = 22;
  const bottom = 18;
  const top = 8;
  const plotW = W - left;
  const plotH = H - bottom - top;
  const max = Math.max(1, ...columns.map((c) => c.count));
  const slot = plotW / columns.length;
  const barW = Math.max(2, Math.min(24, slot - 2));
  const y = (v: number): number => top + plotH - (v / max) * plotH;
  const cols = columns
    .map((c, i) => {
      const x = left + i * slot + (slot - barW) / 2;
      const h = (c.count / max) * plotH;
      const bar = c.count > 0 ? `<path d="${columnPath(x, y(c.count), barW, h)}" class="bar"/>` : "";
      return `<g data-tip="${esc(c.tip)}">${bar}<rect x="${left + i * slot}" y="${top}" width="${slot}" height="${plotH}" class="hit"/><title>${esc(c.tip)}</title></g>`;
    })
    .join("");
  const xLabels = [...labels]
    .map(([i, text]) => {
      const x = left + i * slot + slot / 2;
      const anchor = i === 0 ? "start" : i === columns.length - 1 ? "end" : "middle";
      const ax = i === 0 ? left + i * slot : i === columns.length - 1 ? left + (i + 1) * slot : x;
      return `<text x="${ax}" y="${H - 4}" text-anchor="${anchor}" class="axis">${esc(text)}</text>`;
    })
    .join("");
  return `<svg viewBox="0 0 ${W} ${H}" class="chart" role="img">
    <line x1="${left}" x2="${W}" y1="${y(max)}" y2="${y(max)}" class="grid"/>
    <line x1="${left}" x2="${W}" y1="${y(0)}" y2="${y(0)}" class="grid"/>
    <text x="${left - 4}" y="${y(max) + 4}" text-anchor="end" class="axis">${max}</text>
    <text x="${left - 4}" y="${y(0) + 4}" text-anchor="end" class="axis">0</text>
    ${cols}${xLabels}</svg>`;
}

function dayChart(perDay: DayCount[]): string {
  const columns = perDay.map((d) => ({ count: d.count, tip: `${shortDate(d.date)}: ${count(d.count, "lead")}` }));
  const labels = new Map<number, string>([
    [0, shortDate(perDay[0]?.date ?? "")],
    [perDay.length - 1, "Today"],
  ]);
  return columnChart(columns, labels);
}

function hourChart(byHour: number[]): string {
  const columns = byHour.map((n, h) => ({ count: n, tip: `${hourLabel(h)} to ${String(h).padStart(2, "0")}:59: ${count(n, "lead")}` }));
  const labels = new Map<number, string>([0, 6, 12, 18, 23].map((h) => [h, String(h)]));
  return columnChart(columns, labels);
}

function ranking(buckets: Bucket[], colored = false): string {
  const top = Math.max(...buckets.map((b) => b.count), 1);
  return `<ul class="rank">${buckets
    .map((b) => {
      const kind = colored ? platformInfo(b.label)?.kind : undefined;
      return `<li>
        <div class="rank-top"><span class="rank-name">${bidi(b.label)}</span><span class="rank-n" dir="ltr">${b.count} · ${pct(b.share)}</span></div>
        <div class="rank-bar"><span class="${kind ?? ""}" style="width:${Math.max(2, Math.round((b.count / top) * 100))}%"></span></div>
      </li>`;
    })
    .join("")}</ul>`;
}

function tile(label: string, value: string, note = ""): string {
  return `<div class="tile"><div class="tile-label">${esc(label)}</div><div class="tile-value">${bidi(value)}</div>${note ? `<div class="tile-note en">${esc(note)}</div>` : ""}</div>`;
}

function section(title: string, body: string): string {
  return `<section class="card"><h2>${esc(title)}</h2>${body}</section>`;
}

export function statsView(s: Stats): string {
  const ranges = `<nav class="ranges" aria-label="Time range">${RANGES.map(
    (r) => `<a href="/stats?days=${r}" data-link data-replace class="chip${r === s.days ? " on" : ""}">${r} days</a>`,
  ).join("")}</nav>`;
  if (s.total === 0) return `${backLink()}${ranges}<p class="empty en">No leads in the last ${s.days} days.</p>`;

  const change = changeVsPrevious(s.total, s.previousTotal);
  const avg = s.averagePerDay.toFixed(1).replace(/\.0$/, "");
  const tiles = [
    tile("Leads", String(s.total), change ? `${change} vs the ${s.days} days before` : ""),
    tile("Today", String(s.today)),
    tile("Per day", avg, "on average"),
    s.busiestDay ? tile("Busiest day", shortDate(s.busiestDay.date), count(s.busiestDay.count, "lead")) : "",
    s.peakHour !== null ? tile("Peak hour", hourLabel(s.peakHour), `${count(s.byHour[s.peakHour] ?? 0, "lead")}, local time`) : "",
    s.paid + s.organic > 0 ? tile("Paid / organic", `${s.paid} / ${s.organic}`) : "",
  ].join("");

  return `${backLink()}${ranges}
    <div class="tiles">${tiles}</div>
    ${section("Leads per day", `<p class="readout en" id="readout">${esc(count(s.total, "lead"))} in the last ${s.days} days. Tap a bar for its day.</p>${dayChart(s.perDay)}`)}
    ${section("By platform", ranking(s.byPlatform, true))}
    ${section("By ad", ranking(s.byAd))}
    ${section("By campaign", ranking(s.byCampaign))}
    ${section("By form", ranking(s.byForm))}
    ${section("By time of day", `<p class="readout en">When leads arrive, in your local time.</p>${hourChart(s.byHour)}`)}`;
}
