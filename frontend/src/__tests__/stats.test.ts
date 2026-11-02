import { describe, expect, it } from "vitest";
import { changeVsPrevious, computeStats, dayKey, parseRange } from "../stats";
import type { LeadSummary } from "../types";

// Local-time constructors keep these tests independent of the machine's time zone.
const NOW = new Date(2026, 10, 15, 12, 0); // Nov 15, 2026, noon
const at = (month: number, day: number, hour = 10): string => new Date(2026, month, day, hour).toISOString();

function lead(id: string, created: string, extra: Partial<LeadSummary> = {}): LeadSummary {
  return { id, title: id, created_time: created, ...extra };
}

describe("computeStats", () => {
  const leads = [
    lead("a", at(10, 15, 9), { platform: "ig", ad: "Ad 1", campaign_name: "Q4", form_name: "Spring", is_organic: false }),
    lead("b", at(10, 15, 9), { platform: "fb", ad: "Ad 1", campaign_name: "Q4", form_name: "Spring", is_organic: false }),
    lead("c", at(10, 14, 20), { platform: "ig", ad: "Ad 2", is_organic: true }),
    lead("d", at(10, 1, 9), { platform: "ig" }), // 14 days ago: in the 30 day range, outside 7
    lead("e", at(9, 20, 9), { platform: "fb" }), // 26 days ago
    lead("f", at(8, 1, 9)), // 75 days ago: outside both 30-day windows
    lead("g", at(9, 1, 9)), // 45 days ago: in the previous 30-day period
    lead("bad", "not a date"),
  ];

  it("counts per local day including empty days, and the range's totals", () => {
    const s = computeStats(leads, 7, NOW);
    expect(s.perDay).toHaveLength(7);
    expect(s.perDay[6]).toEqual({ date: dayKey(NOW), count: 2 });
    expect(s.perDay[5]?.count).toBe(1);
    expect(s.perDay[0]?.count).toBe(0);
    expect(s.total).toBe(3);
    expect(s.today).toBe(2);
    expect(s.busiestDay?.count).toBe(2);
    expect(s.averagePerDay).toBeCloseTo(3 / 7);
  });

  it("compares with the equal-length period before", () => {
    const s = computeStats(leads, 30, NOW);
    expect(s.total).toBe(5);
    expect(s.previousTotal).toBe(1);
    const weekly = computeStats(leads, 7, NOW);
    expect(weekly.previousTotal).toBeNull();
    expect(changeVsPrevious(6, 4)).toBe("+50%");
    expect(changeVsPrevious(3, 4)).toBe("-25%");
    expect(changeVsPrevious(3, null)).toBeUndefined();
  });

  it("groups by platform, ad, campaign and form, with Unknown for missing values", () => {
    const s = computeStats(leads, 30, NOW);
    expect(s.byPlatform.map((b) => [b.label, b.count])).toEqual([
      ["Instagram", 3],
      ["Facebook", 2],
    ]);
    expect(s.byAd.map((b) => [b.label, b.count])).toEqual([
      ["Ad 1", 2],
      ["Unknown", 2],
      ["Ad 2", 1],
    ]);
    expect(s.byAd[0]?.share).toBeCloseTo(0.4);
    expect(s.byCampaign[0]).toMatchObject({ label: "Unknown", count: 3 });
    expect(s.paid).toBe(2);
    expect(s.organic).toBe(1);
  });

  it("finds the peak hour and folds small groups into Other", () => {
    const s = computeStats(leads, 7, NOW);
    expect(s.peakHour).toBe(9);
    expect(s.byHour[20]).toBe(1);
    const many = Array.from({ length: 12 }, (_, i) => lead(`x${i}`, at(10, 15), { ad: `Ad ${i}` }));
    const grouped = computeStats(many, 7, NOW).byAd;
    expect(grouped).toHaveLength(9);
    expect(grouped[8]).toMatchObject({ label: "Other (4)", count: 4 });
  });

  it("handles no leads at all", () => {
    const s = computeStats([], 30, NOW);
    expect(s.total).toBe(0);
    expect(s.peakHour).toBeNull();
    expect(s.busiestDay).toBeNull();
    expect(s.byPlatform).toEqual([]);
  });
});

describe("parseRange", () => {
  it("accepts the offered ranges and falls back to 30 days", () => {
    expect(parseRange("7")).toBe(7);
    expect(parseRange("90")).toBe(90);
    expect(parseRange("5")).toBe(30);
    expect(parseRange(null)).toBe(30);
  });
});
