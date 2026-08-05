import { describe, expect, it } from "vitest";
import { buildHyLaserJogCommand, buildJogCommand } from "../src/shared/laser";

describe("GRBL jog commands", () => {
  it("builds bounded incremental X and Y moves", () => {
    expect(buildJogCommand("X", 1, 1000)).toBe("$J=G91 G21 X1.000 F1000");
    expect(buildJogCommand("Y", -0.1, 750)).toBe("$J=G91 G21 Y-0.100 F750");
  });

  it("maps operator Y controls to the HY-Laser's reversed machine axis", () => {
    expect(buildHyLaserJogCommand("X", 1, 1000)).toBe("$J=G91 G21 X1.000 F1000");
    expect(buildHyLaserJogCommand("Y", 1, 1000)).toBe("$J=G91 G21 Y-1.000 F1000");
    expect(buildHyLaserJogCommand("Y", -0.1, 750)).toBe("$J=G91 G21 Y0.100 F750");
  });

  it("rejects zero, excessive distance, and invalid feed", () => {
    expect(() => buildJogCommand("X", 0, 1000)).toThrow(/distance/);
    expect(() => buildJogCommand("X", 101, 1000)).toThrow(/distance/);
    expect(() => buildJogCommand("Y", 1, 0)).toThrow(/feed/);
  });
});
