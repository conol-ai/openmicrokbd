import { describe, expect, it } from "vitest";
import {
  buildCenteredDjGrblJob,
  crc16Ccitt,
  decodeDjGrblResponse,
  DjGrblResponseParser,
  encodeDjGrblCommand
} from "../src/main/djgrbl";

describe("DJGRBL protocol", () => {
  it("matches captured official command frames byte-for-byte", () => {
    const frame = encodeDjGrblCommand(Buffer.from([0x11, 0x0b]));
    expect(frame.toString("hex")).toBe("a10017b93a0942e0a72cefbb9f39e85306f23166885f3c");
    expect(crc16Ccitt(frame.subarray(0, -2))).toBe(frame.readUInt16BE(frame.length - 2));
    expect(encodeDjGrblCommand(Buffer.from([0x11, 0x16])).toString("hex"))
      .toBe("a10017b747e5b88f7121aed4878062cc977ab5668883b7");
    expect(encodeDjGrblCommand(Buffer.from([0x12, 0x0e, 0x4d, 0x35, 0x0a])).toString("hex"))
      .toBe("a100178a72b23a039751c99ebec7827c24245f6688e7ef");
  });

  it("decrypts and parses captured device responses", () => {
    const frame = Buffer.from("b1001526ff5c8dc1b18be66c12687365bdc2af4c7d", "hex");
    expect(decodeDjGrblResponse(frame).toString("hex")).toBe("11030064000001");

    const parser = new DjGrblResponseParser();
    expect(parser.push(frame.subarray(0, 8))).toEqual([]);
    expect(parser.push(frame.subarray(8)).map((item) => item.toString("hex"))).toEqual(["11030064000001"]);
  });

  it("builds centered vector records with calibrated power and speed", () => {
    const plan = buildCenteredDjGrblJob(
      {
        segments: [[{ x: -1, y: 0 }, { x: 1, y: 0 }]],
        intensities: [1],
        powerPercent: 100,
        speedPercent: 10,
        passes: 1
      },
      { x: 50, y: 50 }
    );

    expect(plan.dataCommands).toHaveLength(1);
    expect(plan.recordCount).toBe(18);
    expect(plan.progressTotal).toBe(16);
    const command = plan.dataCommands[0];
    expect(command.subarray(0, 4).toString("hex")).toBe("10010000");
    expect(command.subarray(4, 9).toString("hex")).toBe("0500001999");
    expect(command.subarray(49, 54).toString("hex")).toBe("04000003e8");
    expect(command.subarray(69, 74).toString("hex")).toBe("027d707fff");
    expect(command.subarray(74, 79).toString("hex")).toBe("01828e7fff");
    expect(plan.endCommand.toString("hex")).toBe("10010001ffffffffff");
    expect(plan.startCommand.toString("hex")).toBe("120000");
  });

  it("rejects a centered job that would leave the device workspace", () => {
    expect(() => buildCenteredDjGrblJob({
      segments: [[{ x: -2, y: 0 }, { x: 2, y: 0 }]],
      intensities: [1],
      powerPercent: 100,
      speedPercent: 10,
      passes: 1
    }, { x: 1, y: 1 })).toThrow(/workspace/);
  });

  it("maps positive operator Y toward lower DJGRBL coordinates", () => {
    const plan = buildCenteredDjGrblJob({
      segments: [[{ x: 0, y: 1 }, { x: 0, y: -1 }]],
      intensities: [1],
      powerPercent: 100,
      speedPercent: 10,
      passes: 1
    }, { x: 50, y: 50 });

    const command = plan.dataCommands[0];
    expect(command.subarray(69, 74).toString("hex")).toBe("027fff7d70");
    expect(command.subarray(74, 79).toString("hex")).toBe("017fff828e");
  });
});
