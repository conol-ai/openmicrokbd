import { describe, expect, it } from "vitest";
import { buildLightBurnGCode } from "../src/main/lightburn-gcode";
import type { EngravePoint } from "../src/shared/laser";

function fillOnlyJob(segments: EngravePoint[][], intensities: number[], extras: object = {}) {
  return {
    edges: [],
    fillPlans: [{ segments, intensities }],
    powerPercent: 100,
    speedPercent: 10,
    passes: 1,
    ...extras
  };
}

describe("LightBurn-compatible G-code", () => {
  it("uses the proven GRBL-M3 power and coolant sequence", () => {
    expect(buildLightBurnGCode(fillOnlyJob([[{ x: -1, y: 0 }, { x: 1, y: 0 }]], [1]))).toEqual([
      "G00 G17 G40 G21 G54",
      "G90",
      "M8",
      "M5",
      "G91",
      "G0 X-1.25",
      "M3",
      "G1 X0.25 F600 S0",
      "G1 X2 S1000",
      "G1 X0.25 S0",
      "G1 S0",
      "M5",
      "M9",
      "G0 X-1.25",
      "G90",
      "M2"
    ]);
  });

  it("reverses operator Y and scans adjacent rows in alternating directions", () => {
    const lines = buildLightBurnGCode(fillOnlyJob(
      [
        [{ x: -1, y: 0 }, { x: -0.5, y: 0 }],
        [{ x: 0.5, y: 0 }, { x: 1, y: 0 }],
        [{ x: 1, y: 0.1 }, { x: 0.5, y: 0.1 }],
        [{ x: -0.5, y: 0.1 }, { x: -1, y: 0.1 }]
      ],
      [0.25, 0.5, 0.75, 1],
      { powerPercent: 50, speedPercent: 20 }
    ));

    expect(lines).toContain("G1 X0.25 F1200 S0");
    expect(lines).toContain("G1 X0.5 S125");
    expect(lines).toContain("G1 X0.5 S250");
    expect(lines).toContain("G1 X-0.5 S375");
    expect(lines).toContain("G1 X1 S0");
    expect(lines).toContain("G1 Y-0.1 S0");
    expect(lines).toContain("G1 X-0.5 S500");
  });

  it("traces edge contours at full power before the fill of every pass", () => {
    expect(buildLightBurnGCode({
      edges: [[{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }]],
      fillPlans: [{ segments: [[{ x: -1, y: 0 }, { x: 1, y: 0 }]], intensities: [0.5] }],
      powerPercent: 100,
      speedPercent: 10,
      passes: 1
    })).toEqual([
      "G00 G17 G40 G21 G54",
      "G90",
      "M8",
      "M5",
      "G91",
      "M3",
      "G1 X1 F600 S1000",
      "G1 Y-1 S1000",
      "G1 X-2.25 Y1 S0",
      "G1 X0.25 S0",
      "G1 X2 S500",
      "G1 X0.25 S0",
      "G1 S0",
      "M5",
      "M9",
      "G0 X-1.25",
      "G90",
      "M2"
    ]);
  });

  it("cycles through the fill plans across passes", () => {
    const lines = buildLightBurnGCode({
      edges: [],
      fillPlans: [
        { segments: [[{ x: -1, y: 0 }, { x: 1, y: 0 }]], intensities: [1] },
        { segments: [[{ x: 0, y: -1 }, { x: 0, y: 1 }]], intensities: [1] }
      ],
      powerPercent: 100,
      speedPercent: 10,
      passes: 3
    });

    expect(lines.filter((line) => line === "G1 X2 S1000")).toHaveLength(2);
    expect(lines.filter((line) => line === "G1 Y-2 S1000")).toHaveLength(1);
    expect(lines.filter((line) => line === "M8")).toHaveLength(3);
  });

  it("applies overscan along the scan direction for diagonal runs", () => {
    const lines = buildLightBurnGCode(fillOnlyJob([[{ x: 0, y: 0 }, { x: 1, y: 1 }]], [1]));

    expect(lines).toContain("G0 X-0.177 Y0.177");
    expect(lines).toContain("G1 X0.177 Y-0.177 F600 S0");
    expect(lines).toContain("G1 X1 Y-1 S1000");
    expect(lines).toContain("G1 X0.177 Y-0.177 S0");
  });

  it("rejects runs that are not parallel to the plan's scan axis", () => {
    expect(() => buildLightBurnGCode(fillOnlyJob(
      [[{ x: -1, y: 0 }, { x: 1, y: 0 }], [{ x: 0, y: 1 }, { x: 1, y: 2 }]],
      [1, 1]
    ))).toThrow(/parallel/);
  });

  it("rejects runs that reverse direction inside one scanline", () => {
    expect(() => buildLightBurnGCode(fillOnlyJob(
      [[{ x: -1, y: 0 }, { x: -0.5, y: 0 }], [{ x: 1, y: 0 }, { x: 0.5, y: 0 }]],
      [1, 1]
    ))).toThrow(/same direction/);
  });
});
