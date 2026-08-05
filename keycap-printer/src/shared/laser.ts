export type CoverState = "open" | "closed" | "unavailable" | "unknown";
export type JogAxis = "X" | "Y";

export const HY_LASER_MAX_POWER = 1000;
export const HY_LASER_MAX_FEED = 6000;
export const HY_LASER_INDICATOR_POWER = 20;
export const HY_LASER_INDICATOR_FEED = 1000;

export function buildJogCommand(axis: JogAxis, distance: number, feed: number): string {
  if (axis !== "X" && axis !== "Y") throw new Error("Jog axis must be X or Y");
  if (!Number.isFinite(distance) || distance === 0 || Math.abs(distance) > 100) {
    throw new Error("Jog distance must be between -100 and 100 mm, excluding zero");
  }
  if (!Number.isFinite(feed) || feed < 1 || feed > 10_000) {
    throw new Error("Jog feed must be between 1 and 10000 mm/min");
  }
  return `$J=G91 G21 ${axis}${distance.toFixed(3)} F${Math.round(feed)}`;
}

export function buildHyLaserJogCommand(axis: JogAxis, operatorDistance: number, feed: number): string {
  const machineDistance = axis === "Y" ? -operatorDistance : operatorDistance;
  return buildJogCommand(axis, machineDistance, feed);
}

export interface MachineStatus {
  state: string;
  position: string;
  pins: string;
  cover: CoverState;
}

export function emptyMachineStatus(): MachineStatus {
  return { state: "--", position: "--", pins: "--", cover: "unknown" };
}

export function parseHyLaserStatus(line: string): MachineStatus | null {
  if (!line.startsWith("<") || !line.endsWith(">")) return null;

  const parts = line.slice(1, -1).split("|");
  const state = parts[0] || "--";
  const position = parts.find((part) => part.startsWith("MPos:") || part.startsWith("WPos:"));
  const pinField = parts.find((part) => part.startsWith("Pn:"));
  const pins = pinField?.slice(3) ?? "";
  const doorState = state.match(/^Door:(\d)$/)?.[1];

  let cover: CoverState;
  if (doorState === "1" || doorState === "2") cover = "open";
  else if (doorState === "0" || doorState === "3") cover = "closed";
  else cover = pins.includes("D") ? "open" : "unavailable";

  return {
    state,
    position: position?.replace(/^[MW]Pos:/, "") ?? "--",
    pins,
    cover
  };
}

export interface StreamStatus {
  printing: boolean;
  current: number;
  total: number;
  line: string;
}

export interface LaserSnapshot {
  connected: boolean;
  busy: boolean;
  indicatorOn: boolean;
  machine: MachineStatus;
  stream: StreamStatus;
}

export interface EngravePoint {
  x: number;
  y: number;
}

export interface EngraveJobData {
  segments: EngravePoint[][];
  intensities: number[];
  powerPercent: number;
  speedPercent: number;
  passes: number;
}

export function isEngraveJobData(value: unknown): value is EngraveJobData {
  if (!value || typeof value !== "object") return false;
  const job = value as Partial<EngraveJobData>;
  if (!Array.isArray(job.segments) || job.segments.length === 0 || job.segments.length > 10_000) return false;
  if (!Array.isArray(job.intensities) || job.intensities.length !== job.segments.length) return false;
  if (typeof job.powerPercent !== "number" || !Number.isFinite(job.powerPercent) || job.powerPercent < 0 || job.powerPercent > 100) return false;
  if (typeof job.speedPercent !== "number" || !Number.isFinite(job.speedPercent) || job.speedPercent < 1 || job.speedPercent > 100) return false;
  if (typeof job.passes !== "number" || !Number.isInteger(job.passes) || job.passes < 1 || job.passes > 20) return false;

  let pointCount = 0;
  for (const [segmentIndex, segment] of job.segments.entries()) {
    if (!Array.isArray(segment)) return false;
    const intensity = job.intensities[segmentIndex];
    if (typeof intensity !== "number" || !Number.isFinite(intensity) || intensity <= 0 || intensity > 1) return false;
    pointCount += segment.length;
    if (pointCount > 200_000) return false;
    for (const point of segment) {
      if (
        !point ||
        typeof point !== "object" ||
        typeof point.x !== "number" ||
        typeof point.y !== "number" ||
        !Number.isFinite(point.x) ||
        !Number.isFinite(point.y) ||
        Math.abs(point.x) > 100 ||
        Math.abs(point.y) > 100
      ) return false;
    }
  }
  return pointCount >= 2;
}

export type LaserEvent =
  | { type: "log"; message: string }
  | { type: "connection"; connected: boolean }
  | { type: "busy"; busy: boolean }
  | { type: "indicator"; enabled: boolean }
  | { type: "machine"; machine: MachineStatus }
  | { type: "stream"; stream: StreamStatus };

export interface LaserApi {
  getSnapshot: () => Promise<LaserSnapshot>;
  connect: (baudRate: number) => Promise<void>;
  disconnect: () => Promise<void>;
  probe: () => Promise<void>;
  unlock: () => Promise<void>;
  home: () => Promise<void>;
  jog: (axis: JogAxis, distance: number, feed: number) => Promise<void>;
  setIndicator: (enabled: boolean) => Promise<void>;
  runCentered: (job: EngraveJobData, label: string) => Promise<void>;
  reset: () => Promise<void>;
  onEvent: (listener: (event: LaserEvent) => void) => () => void;
}

export const LASER_CHANNELS = {
  event: "laser:event",
  snapshot: "laser:snapshot",
  connect: "laser:connect",
  disconnect: "laser:disconnect",
  probe: "laser:probe",
  unlock: "laser:unlock",
  home: "laser:home",
  jog: "laser:jog",
  indicator: "laser:indicator",
  runCentered: "laser:run-centered",
  reset: "laser:reset"
} as const;
