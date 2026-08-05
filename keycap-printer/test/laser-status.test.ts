import { describe, expect, it } from "vitest";
import { isEngraveJobData, parseHyLaserStatus } from "../src/shared/laser";

describe("HY-Laser status parsing", () => {
  it("keeps the vendor's persistent Z limit separate from the cover", () => {
    expect(parseHyLaserStatus("<Idle|MPos:0.000,0.000,0.000|FS:0,0|Pn:Z>")).toEqual({
      state: "Idle",
      position: "0.000,0.000,0.000",
      pins: "Z",
      cover: "unavailable"
    });
  });

  it("does not invent a cover state when no door input is reported", () => {
    expect(parseHyLaserStatus("<Idle|WPos:1.000,2.000,0.000|FS:0,0>")).toEqual({
      state: "Idle",
      position: "1.000,2.000,0.000",
      pins: "",
      cover: "unavailable"
    });
  });

  it("also supports standard GRBL safety-door states", () => {
    expect(parseHyLaserStatus("<Door:1|MPos:0.000,0.000,0.000|Pn:D>")?.cover).toBe("open");
    expect(parseHyLaserStatus("<Door:0|MPos:0.000,0.000,0.000>")?.cover).toBe("closed");
  });

  it("rejects non-status lines", () => {
    expect(parseHyLaserStatus("ok")).toBeNull();
  });

  it("validates bounded engraving payloads at the IPC boundary", () => {
    expect(isEngraveJobData({
      segments: [[{ x: -1, y: 0 }, { x: 1, y: 0 }]],
      intensities: [1],
      powerPercent: 100,
      speedPercent: 10,
      passes: 1
    })).toBe(true);
    expect(isEngraveJobData({
      segments: [[{ x: Number.NaN, y: 0 }, { x: 1, y: 0 }]],
      intensities: [1],
      powerPercent: 100,
      speedPercent: 10,
      passes: 1
    })).toBe(false);
    expect(isEngraveJobData({ segments: [], intensities: [], powerPercent: 100, speedPercent: 10, passes: 1 })).toBe(false);
    expect(isEngraveJobData({
      segments: [[{ x: 0, y: 0 }, { x: 1, y: 0 }]],
      intensities: [1.1],
      powerPercent: 100,
      speedPercent: 10,
      passes: 1
    })).toBe(false);
  });
});
