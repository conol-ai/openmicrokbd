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

interface ScanRun {
  start: MachinePoint;
  end: MachinePoint;
  intensity: number;
}

interface ScanLine {
  normalCoordinate: number;
  runs: ScanRun[];
}

const RASTER_OVERSCAN_MM = 0.25;
const SCANLINE_TOLERANCE_MM = 0.02;

export function buildLightBurnGCode(job: EngraveJobData): string[] {
  if (!isEngraveJobData(job)) throw new Error("Invalid engraving vector payload");

  const edges = job.edges.map((polyline) => polyline.map(toMachinePoint));
  const plans = job.fillPlans.map((plan) => buildScanLines(plan.segments, plan.intensities));
  if (edges.length === 0 && plans.every((lines) => lines.length === 0)) {
    throw new Error("The icon has no engravable raster pixels");
  }

  const power = Math.round((job.powerPercent / 100) * HY_LASER_MAX_POWER);
  const feed = Math.round((job.speedPercent / 100) * HY_LASER_MAX_FEED);
  const lines = ["G00 G17 G40 G21 G54", "G90", "M8", "M5", "G91"];
  let position: MachinePoint = { x: 0, y: 0 };

  for (let pass = 0; pass < job.passes; pass += 1) {
    let armed = false;
    let feedPending = true;

    const feedSuffix = (): string => {
      if (!feedPending) return "";
      feedPending = false;
      return ` F${feed}`;
    };

    const travelTo = (target: MachinePoint): void => {
      const move = relativeMove(position, target);
      if (!armed) {
        if (move) lines.push(`G0 ${move}`);
        lines.push("M3");
        armed = true;
      } else if (move) {
        lines.push(`G1 ${move}${feedSuffix()} S0`);
      }
      position = target;
    };

    const burnTo = (target: MachinePoint, intensity: number): void => {
      const move = relativeMove(position, target);
      const runPower = power === 0 ? 0 : Math.max(1, Math.round(power * intensity));
      if (move) lines.push(`G1 ${move}${feedSuffix()} S${runPower}`);
      position = target;
    };

    for (const polyline of edges) {
      travelTo(polyline[0]);
      for (let index = 1; index < polyline.length; index += 1) burnTo(polyline[index], 1);
    }

    for (const line of plans[pass % plans.length]) {
      const first = line.runs[0];
      const last = line.runs.at(-1)!;
      travelTo(overscanPoint(first.start, unitVector(first.start, first.end), -RASTER_OVERSCAN_MM));
      for (const run of line.runs) {
        travelTo(run.start);
        burnTo(run.end, run.intensity);
      }
      travelTo(overscanPoint(last.end, unitVector(last.start, last.end), RASTER_OVERSCAN_MM));
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

function buildScanLines(segments: EngravePoint[][], intensities: number[]): ScanLine[] {
  const runs: ScanRun[] = segments.map((segment, index) => {
    if (segment.length !== 2) throw new Error("Raster runs must contain exactly two points");
    const start = toMachinePoint(segment[0]);
    const end = toMachinePoint(segment[1]);
    if (start.x === end.x && start.y === end.y) throw new Error("Raster runs must not be empty");
    return { start, end, intensity: intensities[index] };
  });
  if (runs.length === 0) return [];

  // The longest run defines the plan's scan axis with the least rounding error.
  const longest = runs.reduce((best, run) => (runLength(run) > runLength(best) ? run : best));
  const axis = unitVector(longest.start, longest.end);
  const normal = { x: -axis.y, y: axis.x };

  const scanLines: ScanLine[] = [];
  for (const run of runs) {
    const along = (run.end.x - run.start.x) * normal.x + (run.end.y - run.start.y) * normal.y;
    if (Math.abs(along) > SCANLINE_TOLERANCE_MM) throw new Error("Raster runs must be parallel within one fill plan");
    const normalCoordinate = run.start.x * normal.x + run.start.y * normal.y;
    const line = scanLines.at(-1);
    if (line && Math.abs(normalCoordinate - line.normalCoordinate) <= SCANLINE_TOLERANCE_MM) line.runs.push(run);
    else scanLines.push({ normalCoordinate, runs: [run] });
  }

  for (const line of scanLines) {
    const direction = Math.sign(dot(unitVector(line.runs[0].start, line.runs[0].end), axis));
    if (line.runs.some((run) => Math.sign(dot(unitVector(run.start, run.end), axis)) !== direction)) {
      throw new Error("Raster runs in one scanline must use the same direction");
    }
  }
  return scanLines;
}

function toMachinePoint(point: EngravePoint): MachinePoint {
  return { x: roundCoordinate(point.x), y: roundCoordinate(-point.y) };
}

function runLength(run: ScanRun): number {
  return Math.hypot(run.end.x - run.start.x, run.end.y - run.start.y);
}

function unitVector(from: MachinePoint, to: MachinePoint): MachinePoint {
  const length = Math.hypot(to.x - from.x, to.y - from.y);
  return { x: (to.x - from.x) / length, y: (to.y - from.y) / length };
}

function dot(a: MachinePoint, b: MachinePoint): number {
  return a.x * b.x + a.y * b.y;
}

function overscanPoint(point: MachinePoint, direction: MachinePoint, distance: number): MachinePoint {
  return {
    x: roundCoordinate(point.x + direction.x * distance),
    y: roundCoordinate(point.y + direction.y * distance)
  };
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
