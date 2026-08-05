import { describe, expect, it } from "vitest";
import { icons, type IconNode } from "lucide";
import { siApple, siDiscord, siGithub, siYoutube } from "simple-icons";
import {
  buildIconJob,
  buildSimpleIconJob,
  DEFAULT_SETTINGS,
  feedFromPercent,
  normalizeSettings,
  powerFromPercent
} from "../src/renderer/src/lib/toolpath";

describe("Lucide toolpath generation", () => {
  it("generates finite toolpaths for every installed Lucide icon", { timeout: 20000 }, () => {
    let generated = 0;
    for (const [name, node] of Object.entries(icons)) {
      if (!Array.isArray(node)) continue;
      const job = buildIconJob(node as IconNode, DEFAULT_SETTINGS);
      expect(job.segments.length, name).toBeGreaterThan(0);
      expect(job.segments.flat().every((point) => Number.isFinite(point.x) && Number.isFinite(point.y)), name).toBe(true);
      expect(job.segments.every((segment) => segment.length === 2 && segment[0].y === segment[1].y), name).toBe(true);
      expect(job.intensities.length, name).toBe(job.segments.length);
      expect(job.intensities.every((intensity) => intensity > 0 && intensity <= 1), name).toBe(true);
      expect(job.stats.pixelCount, name).toBeGreaterThan(0);
      expect(job.stats.pointCount, name).toBeLessThanOrEqual(200_000);
      generated += 1;
    }
    expect(generated).toBeGreaterThan(1000);
  });

  it("supports compact SVG arc flags used by the Barrel icon", () => {
    const job = buildIconJob(icons.Barrel, DEFAULT_SETTINGS);
    expect(job.segments.length).toBeGreaterThan(0);
    expect(job.stats.pointCount).toBeGreaterThan(job.stats.motionSegments);
  });

  it("blocks oversized toolpaths at the job boundary", () => {
    const job = buildIconJob(icons.Keyboard, { ...DEFAULT_SETTINGS, keycapWidth: 8, keycapHeight: 8, iconSize: 14 });
    expect(job.stats.fitsKeycap).toBe(false);
  });

  it("normalizes unsafe numeric values", () => {
    const settings = normalizeSettings({ power: 5000, passes: 0, iconSize: Number.NaN });
    expect(settings.power).toBe(1000);
    expect(settings.passes).toBe(1);
    expect(settings.iconSize).toBe(DEFAULT_SETTINGS.iconSize);
  });

  it("maps the calibrated percentages to HY-Laser S and F values", () => {
    expect(powerFromPercent(2)).toBe(20);
    expect(powerFromPercent(100)).toBe(1000);
    expect(feedFromPercent(10)).toBe(600);
    expect(DEFAULT_SETTINGS.power).toBe(1000);
    expect(DEFAULT_SETTINGS.engraveFeed).toBe(600);
  });

  it("centers transformed vectors on the configured fine offset", () => {
    const job = buildIconJob(icons.Square, { ...DEFAULT_SETTINGS, offsetX: 1.5, offsetY: -2.5 });
    const bounds = job.stats.bounds!;
    expect((bounds.minX + bounds.maxX) / 2).toBeCloseTo(1.5, 5);
    expect((bounds.minY + bounds.maxY) / 2).toBeCloseTo(-2.5, 5);
  });

  it("rasterizes adjustable Lucide strokes with grayscale edge coverage", () => {
    const thin = buildIconJob(icons.Minus, { ...DEFAULT_SETTINGS, lineWidth: 0.1 });
    const thick = buildIconJob(icons.Minus, { ...DEFAULT_SETTINGS, lineWidth: 0.7 });

    expect(thin.stats.scanlineCount).toBe(1);
    expect(thick.stats.scanlineCount).toBe(7);
    expect(thin.stats.grayscaleLevels).toBeGreaterThan(1);
    expect(thick.stats.grayscaleLevels).toBeGreaterThan(1);
    expect(thick.stats.bounds!.maxY - thick.stats.bounds!.minY).toBeCloseTo(0.7, 5);
  });

  it("keeps the keycap boundary fixed when the icon is offset", () => {
    const job = buildIconJob(icons.Square, { ...DEFAULT_SETTINGS, offsetX: 10 });
    expect(job.stats.workArea).toEqual({ minX: -7, maxX: 7, minY: -7, maxY: 7 });
    expect(job.stats.fitsKeycap).toBe(false);
  });

  it("rasterizes representative Simple Icons as filled horizontal scan runs", () => {
    for (const icon of [siApple, siDiscord, siGithub, siYoutube]) {
      const job = buildSimpleIconJob(icon.path, DEFAULT_SETTINGS);
      expect(job.stats.pixelCount, icon.title).toBeGreaterThan(100);
      expect(job.stats.fitsKeycap, icon.title).toBe(true);
      expect(job.segments.every((segment) => segment.length === 2 && segment[0].y === segment[1].y), icon.title).toBe(true);
      expect(job.intensities.length, icon.title).toBe(job.segments.length);
      expect(job.stats.grayscaleLevels, icon.title).toBeGreaterThan(1);
    }
  });

  it("preserves holes in compound filled brand paths", () => {
    const job = buildSimpleIconJob("M2 2H22V22H2Z M8 8V16H16V8Z", { ...DEFAULT_SETTINGS, iconSize: 12 });
    const centerRuns = job.segments.filter((segment) => segment[0].y === 0);
    expect(centerRuns.length).toBeGreaterThan(2);
    expect(centerRuns.every((run) => Math.max(run[0].x, run[1].x) < 0 || Math.min(run[0].x, run[1].x) > 0)).toBe(true);
    expect(job.stats.grayscaleLevels).toBeGreaterThan(1);
  });
});
