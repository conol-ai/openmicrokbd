import { describe, expect, it } from "vitest";
import { buildLightBurnGCode } from "../src/main/lightburn-gcode";

describe("LightBurn-compatible G-code", () => {
  it("uses the proven GRBL-M3 power and coolant sequence", () => {
    expect(buildLightBurnGCode({
      segments: [[{ x: -1, y: 0 }, { x: 1, y: 0 }]],
      intensities: [1],
      powerPercent: 100,
      speedPercent: 10,
      passes: 1
    })).toEqual([
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
    const lines = buildLightBurnGCode({
      segments: [
        [{ x: -1, y: 0 }, { x: -0.5, y: 0 }],
        [{ x: 0.5, y: 0 }, { x: 1, y: 0 }],
        [{ x: 1, y: 0.1 }, { x: 0.5, y: 0.1 }],
        [{ x: -0.5, y: 0.1 }, { x: -1, y: 0.1 }]
      ],
      intensities: [0.25, 0.5, 0.75, 1],
      powerPercent: 50,
      speedPercent: 20,
      passes: 1
    });

    expect(lines).toContain("G1 X0.25 F1200 S0");
    expect(lines).toContain("G1 X0.5 S125");
    expect(lines).toContain("G1 X0.5 S250");
    expect(lines).toContain("G1 X-0.5 S375");
    expect(lines).toContain("G1 X1 S0");
    expect(lines).toContain("G1 Y-0.1 S0");
    expect(lines).toContain("G1 X-0.5 S500");
  });

  it("rejects centerline vectors that have not been rasterized", () => {
    expect(() => buildLightBurnGCode({
      segments: [[{ x: 0, y: -1 }, { x: 0, y: 1 }]],
      intensities: [1],
      powerPercent: 100,
      speedPercent: 10,
      passes: 1
    })).toThrow(/horizontal/);
  });
});
