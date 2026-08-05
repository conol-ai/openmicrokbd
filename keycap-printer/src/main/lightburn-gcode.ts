import {
  HY_LASER_MAX_FEED,
  HY_LASER_MAX_POWER,
  isEngraveJobData,
  type EngraveJobData,
  type EngravePoint
} from "../shared/laser";

interface MachinePoint {
  x: number;
  y: number;
}

interface ScanRow {
  y: number;
  runs: Array<{ start: MachinePoint; end: MachinePoint; intensity: number }>;
}

const RASTER_OVERSCAN_MM = 0.25;

export function buildLightBurnGCode(job: EngraveJobData): string[] {
  if (!isEngraveJobData(job)) throw new Error("Invalid engraving vector payload");

  const rows = buildScanRows(job.segments, job.intensities);
  if (rows.length === 0) throw new Error("The icon has no engravable raster pixels");

  const power = Math.round((job.powerPercent / 100) * HY_LASER_MAX_POWER);
  const feed = Math.round((job.speedPercent / 100) * HY_LASER_MAX_FEED);
  const lines = ["G00 G17 G40 G21 G54", "G90", "M8", "M5", "G91"];
  let position: MachinePoint = { x: 0, y: 0 };

  for (let pass = 0; pass < job.passes; pass += 1) {
    let feedPending = true;
    for (const [rowIndex, row] of rows.entries()) {
      const direction = Math.sign(row.runs[0].end.x - row.runs[0].start.x);
      const lead = { x: roundCoordinate(row.runs[0].start.x - direction * RASTER_OVERSCAN_MM), y: row.y };
      const leadTravel = relativeMove(position, lead);
      if (leadTravel) lines.push(`${rowIndex === 0 ? "G0" : "G1"} ${leadTravel}${rowIndex === 0 ? "" : " S0"}`);
      position = lead;

      if (rowIndex === 0) lines.push("M3");
      for (const run of row.runs) {
        const blankMove = relativeMove(position, run.start);
        if (blankMove) {
          lines.push(`G1 ${blankMove}${feedPending ? ` F${feed}` : ""} S0`);
          feedPending = false;
        }
        position = run.start;

        const burnMove = relativeMove(position, run.end);
        const runPower = power === 0 ? 0 : Math.max(1, Math.round(power * run.intensity));
        if (burnMove) lines.push(`G1 ${burnMove} S${runPower}`);
        position = run.end;
      }

      const lastRun = row.runs.at(-1)!;
      const trail = { x: roundCoordinate(lastRun.end.x + direction * RASTER_OVERSCAN_MM), y: row.y };
      const trailMove = relativeMove(position, trail);
      if (trailMove) lines.push(`G1 ${trailMove} S0`);
      position = trail;
    }

    lines.push("G1 S0", "M5", "M9");
    const returnMove = relativeMove(position, { x: 0, y: 0 });
    if (returnMove) lines.push(`G0 ${returnMove}`);
    position = { x: 0, y: 0 };
    if (pass < job.passes - 1) lines.push("M8");
  }

  lines.push("G90", "M2");
  return lines;
}

function buildScanRows(segments: EngravePoint[][], intensities: number[]): ScanRow[] {
  const rows: ScanRow[] = [];
  for (const [segmentIndex, segment] of segments.entries()) {
    if (segment.length !== 2) throw new Error("Raster runs must contain exactly two points");
    const start = toMachinePoint(segment[0]);
    const end = toMachinePoint(segment[1]);
    if (start.y !== end.y || start.x === end.x) throw new Error("Raster runs must be non-empty horizontal lines");

    const row = rows.at(-1);
    const intensity = intensities[segmentIndex];
    if (row?.y === start.y) row.runs.push({ start, end, intensity });
    else rows.push({ y: start.y, runs: [{ start, end, intensity }] });
  }

  for (const row of rows) {
    const direction = Math.sign(row.runs[0].end.x - row.runs[0].start.x);
    if (row.runs.some((run) => Math.sign(run.end.x - run.start.x) !== direction)) {
      throw new Error("Raster runs in one scanline must use the same direction");
    }
  }
  return rows;
}

function toMachinePoint(point: EngravePoint): MachinePoint {
  return { x: roundCoordinate(point.x), y: roundCoordinate(-point.y) };
}

function relativeMove(from: MachinePoint, to: MachinePoint): string {
  const x = roundCoordinate(to.x - from.x);
  const y = roundCoordinate(to.y - from.y);
  return [x === 0 ? "" : `X${formatCoordinate(x)}`, y === 0 ? "" : `Y${formatCoordinate(y)}`]
    .filter(Boolean)
    .join(" ");
}

function roundCoordinate(value: number): number {
  const rounded = Math.round(value * 1_000) / 1_000;
  return Math.abs(rounded) < 0.0005 ? 0 : rounded;
}

function formatCoordinate(value: number): string {
  return value.toFixed(3).replace(/\.?0+$/, "");
}
