import { platformInfo } from "./format";
import { t } from "./strings";
import type { LeadSummary } from "./types";

export const RANGES = [7, 30, 90] as const;
export type Range = (typeof RANGES)[number];
export const DEFAULT_RANGE: Range = 30;

export function parseRange(value: string | null): Range {
  const n = Number(value);
  return RANGES.find((r) => r === n) ?? DEFAULT_RANGE;
}

export interface Bucket {
  label: string;
  count: number;
  /** Share of the leads in the range, 0 to 1. */
  share: number;
}

export interface DayCount {
  /** Local date as YYYY-MM-DD. */
  date: string;
  count: number;
}

export interface Stats {
  days: Range;
  total: number;
  /** Leads in the equal-length period just before; null when that period has none to compare. */
  previousTotal: number | null;
  today: number;
  perDay: DayCount[];
  averagePerDay: number;
  busiestDay: DayCount | null;
  /** Hour of the day (0-23, local time) with the most leads. */
  peakHour: number | null;
  byHour: number[];
  byPlatform: Bucket[];
  byAd: Bucket[];
  byCampaign: Bucket[];
  byForm: Bucket[];
  paid: number;
  organic: number;
}

const pad = (n: number): string => String(n).padStart(2, "0");

/** The local calendar day of a moment, as YYYY-MM-DD. */
export function dayKey(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function startOfDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate());
}

/** `days` back from `from`, counted in calendar days (safe across daylight-saving changes). */
function addDays(from: Date, days: number): Date {
  return new Date(from.getFullYear(), from.getMonth(), from.getDate() + days);
}

const TOP = 8;

/** The largest groups, with the rest folded into "Other"; `label` maps a lead to its group (or undefined). */
function group(leads: LeadSummary[], label: (l: LeadSummary) => string | undefined, total: number): Bucket[] {
  const counts = new Map<string, number>();
  for (const l of leads) {
    const key = label(l)?.trim() || t.stats.unknown;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  const sorted = [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  const shown = sorted.slice(0, TOP);
  const rest = sorted.slice(TOP).reduce((sum, [, n]) => sum + n, 0);
  if (rest > 0) shown.push([t.stats.other(sorted.length - TOP), rest]);
  return shown.map(([name, count]) => ({ label: name, count, share: total ? count / total : 0 }));
}

/** Everything the stats page shows, for the leads created in the last `days` days (today included). */
export function computeStats(leads: LeadSummary[], days: Range, now: Date = new Date()): Stats {
  const today = startOfDay(now);
  const start = addDays(today, -(days - 1));
  const previousStart = addDays(start, -days);
  const dated = leads.flatMap((l) => {
    const t = new Date(l.created_time);
    return Number.isNaN(t.getTime()) ? [] : [{ lead: l, time: t }];
  });
  const inRange = dated.filter(({ time }) => time >= start && time < addDays(today, 1));
  const previous = dated.filter(({ time }) => time >= previousStart && time < start);

  const perDay: DayCount[] = Array.from({ length: days }, (_, i) => ({ date: dayKey(addDays(start, i)), count: 0 }));
  const index = new Map(perDay.map((d, i) => [d.date, i]));
  const byHour = new Array<number>(24).fill(0);
  for (const { time } of inRange) {
    const slot = perDay[index.get(dayKey(time)) ?? -1];
    if (slot) slot.count += 1;
    byHour[time.getHours()] = (byHour[time.getHours()] ?? 0) + 1;
  }

  const total = inRange.length;
  const rangeLeads = inRange.map(({ lead }) => lead);
  const busiest = perDay.reduce<DayCount | null>((best, d) => (d.count > (best?.count ?? 0) ? d : best), null);
  const peak = Math.max(...byHour);
  return {
    days,
    total,
    previousTotal: previous.length > 0 ? previous.length : null,
    today: perDay[perDay.length - 1]?.count ?? 0,
    perDay,
    averagePerDay: total / days,
    busiestDay: busiest,
    peakHour: peak > 0 ? byHour.indexOf(peak) : null,
    byHour,
    byPlatform: group(rangeLeads, (l) => platformInfo(l.platform)?.label, total),
    byAd: group(rangeLeads, (l) => l.ad ?? undefined, total),
    byCampaign: group(rangeLeads, (l) => l.campaign_name ?? undefined, total),
    byForm: group(rangeLeads, (l) => l.form_name ?? undefined, total),
    paid: rangeLeads.filter((l) => l.is_organic === false).length,
    organic: rangeLeads.filter((l) => l.is_organic === true).length,
  };
}

/** "+12%" or "-8%" against the previous period; undefined when there is nothing to compare with. */
export function changeVsPrevious(total: number, previous: number | null): string | undefined {
  if (previous === null) return undefined;
  const pct = Math.round(((total - previous) / previous) * 100);
  return `${pct > 0 ? "+" : ""}${pct}%`;
}
