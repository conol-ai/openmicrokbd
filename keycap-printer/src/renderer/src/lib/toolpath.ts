import type { IconNode, SVGProps } from "lucide";
import { SVGPathData } from "svg-pathdata";
import { svgPathProperties } from "svg-path-properties";
import { HY_LASER_MAX_FEED, HY_LASER_MAX_POWER } from "../../../shared/laser";

export { HY_LASER_MAX_FEED, HY_LASER_MAX_POWER } from "../../../shared/laser";

export const INDICATOR_POWER_PERCENT = 2;
export const ENGRAVE_POWER_PERCENT = 100;
export const ENGRAVE_SPEED_PERCENT = 10;
export const DEFAULT_LINE_WIDTH_MM = 0.7;
export const RASTER_PIXEL_MM = 0.1;
export const GRAYSCALE_LEVELS = 16;

const SUPERSAMPLE_SIDE = 4;
const FULL_COVERAGE_MASK = (1 << GRAYSCALE_LEVELS) - 1;
const PIXEL_HALF_DIAGONAL = (RASTER_PIXEL_MM * Math.SQRT2) / 2;
const SUBPIXEL_OFFSETS = Array.from({ length: GRAYSCALE_LEVELS }, (_, index) => ({
  x: (((index % SUPERSAMPLE_SIDE) + 0.5) / SUPERSAMPLE_SIDE - 0.5) * RASTER_PIXEL_MM,
  y: ((Math.floor(index / SUPERSAMPLE_SIDE) + 0.5) / SUPERSAMPLE_SIDE - 0.5) * RASTER_PIXEL_MM
}));

export function powerFromPercent(percent: number): number {
  return Math.round((clampNumber(percent, 0, 100, 0) / 100) * HY_LASER_MAX_POWER);
}

export function powerToPercent(power: number): number {
  return (clampNumber(power, 0, HY_LASER_MAX_POWER, 0) / HY_LASER_MAX_POWER) * 100;
}

export function feedFromPercent(percent: number): number {
  return Math.round((clampNumber(percent, 1, 100, 1) / 100) * HY_LASER_MAX_FEED);
}

export function feedToPercent(feed: number): number {
  return (clampNumber(feed, 1, HY_LASER_MAX_FEED, 1) / HY_LASER_MAX_FEED) * 100;
}

export interface LaserSettings {
  baudRate: number;
  keycapWidth: number;
  keycapHeight: number;
  iconSize: number;
  lineWidth: number;
  offsetX: number;
  offsetY: number;
  rotation: number;
  mirrorX: boolean;
  mirrorY: boolean;
  power: number;
  engraveFeed: number;
  passes: number;
  curveQuality: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface Bounds {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
}

export interface JobStats {
  motionSegments: number;
  pointCount: number;
  pixelCount: number;
  scanlineCount: number;
  grayscaleLevels: number;
  travelMm: number;
  cutsMm: number;
  bounds: Bounds | null;
  workArea: Bounds;
  fitsKeycap: boolean;
}

export interface LaserJob {
  segments: Point[][];
  intensities: number[];
  stats: JobStats;
}

export const DEFAULT_SETTINGS: LaserSettings = {
  baudRate: 115200,
  keycapWidth: 14,
  keycapHeight: 14,
  iconSize: 8.5,
  lineWidth: DEFAULT_LINE_WIDTH_MM,
  offsetX: 0,
  offsetY: 0,
  rotation: 0,
  mirrorX: false,
  mirrorY: false,
  power: powerFromPercent(ENGRAVE_POWER_PERCENT),
  engraveFeed: feedFromPercent(ENGRAVE_SPEED_PERCENT),
  passes: 1,
  curveQuality: 36
};

export function normalizeSettings(settings: Partial<LaserSettings> = {}): LaserSettings {
  return {
    ...DEFAULT_SETTINGS,
    ...settings,
    baudRate: clampInteger(settings.baudRate, 300, 921600, DEFAULT_SETTINGS.baudRate),
    keycapWidth: clampNumber(settings.keycapWidth, 1, 80, DEFAULT_SETTINGS.keycapWidth),
    keycapHeight: clampNumber(settings.keycapHeight, 1, 80, DEFAULT_SETTINGS.keycapHeight),
    iconSize: clampNumber(settings.iconSize, 0.5, 70, DEFAULT_SETTINGS.iconSize),
    lineWidth: clampNumber(settings.lineWidth, 0.1, 2, DEFAULT_SETTINGS.lineWidth),
    offsetX: clampNumber(settings.offsetX, -500, 500, DEFAULT_SETTINGS.offsetX),
    offsetY: clampNumber(settings.offsetY, -500, 500, DEFAULT_SETTINGS.offsetY),
    rotation: clampNumber(settings.rotation, -360, 360, DEFAULT_SETTINGS.rotation),
    power: clampInteger(settings.power, 0, HY_LASER_MAX_POWER, DEFAULT_SETTINGS.power),
    engraveFeed: clampInteger(settings.engraveFeed, 1, HY_LASER_MAX_FEED, DEFAULT_SETTINGS.engraveFeed),
    passes: clampInteger(settings.passes, 1, 20, DEFAULT_SETTINGS.passes),
    curveQuality: clampInteger(settings.curveQuality, 8, 96, DEFAULT_SETTINGS.curveQuality),
    mirrorX: Boolean(settings.mirrorX),
    mirrorY: Boolean(settings.mirrorY)
  };
}

export function buildIconJob(iconNode: IconNode, settings: Partial<LaserSettings>): LaserJob {
  const normalized = normalizeSettings(settings);
  const source = iconNode.flatMap(([tag, attrs]) => elementToPolylines(tag, attrs, normalized.curveQuality));
  const centerlines = transformPolylines(source.map(dedupeAdjacentPoints).filter((line) => line.length > 1), normalized);
  const raster = rasterizePolylines(centerlines, normalized.lineWidth);
  return buildLaserJob(raster, normalized);
}

export function buildSimpleIconJob(pathData: string, settings: Partial<LaserSettings>): LaserJob {
  const normalized = normalizeSettings(settings);
  const contours = transformPolylines(
    pathToPolylines(pathData, normalized.curveQuality).map(dedupeAdjacentPoints).filter((line) => line.length > 1),
    normalized
  );
  return buildLaserJob(rasterizeFilledPolylines(contours), normalized);
}

function buildLaserJob(raster: RasterizedToolpath, settings: LaserSettings): LaserJob {
  const baseStats = pathStats(raster.segments);
  const workArea = keycapBounds(settings);

  return {
    segments: raster.segments,
    intensities: raster.intensities,
    stats: {
      ...baseStats,
      pixelCount: raster.pixelCount,
      scanlineCount: raster.scanlineCount,
      grayscaleLevels: raster.grayscaleLevels,
      bounds: raster.bounds,
      workArea,
      fitsKeycap: raster.bounds !== null && boundsInside(raster.bounds, workArea)
    }
  };
}

function elementToPolylines(tag: string, attrs: SVGProps, quality: number): Point[][] {
  switch (tag) {
    case "path":
      return pathToPolylines(String(attrs.d ?? ""), quality);
    case "line":
      return [[point(attrs.x1, attrs.y1), point(attrs.x2, attrs.y2)]];
    case "polyline":
      return [parsePoints(attrs.points)];
    case "polygon": {
      const points = parsePoints(attrs.points);
      if (points.length > 1) points.push({ ...points[0] });
      return [points];
    }
    case "circle":
      return [ellipse(numberAttr(attrs.cx), numberAttr(attrs.cy), numberAttr(attrs.r), numberAttr(attrs.r), quality * 2)];
    case "ellipse":
      return [ellipse(numberAttr(attrs.cx), numberAttr(attrs.cy), numberAttr(attrs.rx), numberAttr(attrs.ry), quality * 2)];
    case "rect":
      return [rectToPolyline(attrs, quality)];
    default:
      return [];
  }
}

function pathToPolylines(pathData: string, quality: number): Point[][] {
  if (!pathData.trim()) return [];
  const normalizedPath = new SVGPathData(pathData).encode();
  const parts = new svgPathProperties(normalizedPath).getParts();
  const lines: Point[][] = [];
  let current: Point[] = [];

  for (const part of parts) {
    if (current.length === 0 || !pointsNearlyEqual(current.at(-1)!, part.start)) {
      if (current.length > 1) lines.push(current);
      current = [{ ...part.start }];
    }

    const samples = Math.max(1, Math.ceil((part.length / 24) * quality));
    for (let index = 1; index <= samples; index += 1) {
      const point = part.getPointAtLength((part.length * index) / samples);
      current.push({ x: point.x, y: point.y });
    }
  }

  if (current.length > 1) lines.push(current);
  return lines;
}

function transformPolylines(polylines: Point[][], settings: LaserSettings): Point[][] {
  const scale = settings.iconSize / 24;
  const radians = (settings.rotation * Math.PI) / 180;
  const cos = Math.cos(radians);
  const sin = Math.sin(radians);
  const origin = iconCenter(settings);

  return polylines.map((line) =>
    line.map((source) => {
      let x = (source.x - 12) * scale * (settings.mirrorX ? -1 : 1);
      let y = (12 - source.y) * scale * (settings.mirrorY ? -1 : 1);
      const rotatedX = x * cos - y * sin;
      const rotatedY = x * sin + y * cos;
      return { x: rotatedX + origin.x, y: rotatedY + origin.y };
    })
  );
}

interface RasterizedToolpath {
  segments: Point[][];
  intensities: number[];
  bounds: Bounds | null;
  pixelCount: number;
  scanlineCount: number;
  grayscaleLevels: number;
}

type RasterRows = Map<number, Map<number, number>>;

function rasterizePolylines(polylines: Point[][], width: number): RasterizedToolpath {
  const rows: RasterRows = new Map();
  const radius = width / 2;
  const fullCoverageRadius = Math.max(0, radius - PIXEL_HALF_DIAGONAL);

  for (const polyline of polylines) {
    for (let index = 1; index < polyline.length; index += 1) {
      const from = polyline[index - 1];
      const to = polyline[index];
      const reach = radius + PIXEL_HALF_DIAGONAL;
      const minColumn = Math.floor((Math.min(from.x, to.x) - reach) / RASTER_PIXEL_MM);
      const maxColumn = Math.ceil((Math.max(from.x, to.x) + reach) / RASTER_PIXEL_MM);
      const minRow = Math.floor((Math.min(from.y, to.y) - reach) / RASTER_PIXEL_MM);
      const maxRow = Math.ceil((Math.max(from.y, to.y) + reach) / RASTER_PIXEL_MM);

      for (let row = minRow; row <= maxRow; row += 1) {
        const y = row * RASTER_PIXEL_MM;
        for (let column = minColumn; column <= maxColumn; column += 1) {
          const x = column * RASTER_PIXEL_MM;
          const centerDistanceSquared = squaredDistanceToSegment({ x, y }, from, to);
          let mask = 0;
          if (fullCoverageRadius > 0 && centerDistanceSquared <= fullCoverageRadius * fullCoverageRadius) {
            mask = FULL_COVERAGE_MASK;
          } else if (centerDistanceSquared <= reach * reach) {
            mask = sampleCoverageMask(x, y, (sample) => squaredDistanceToSegment(sample, from, to) <= radius * radius);
          }
          addCoverageMask(rows, row, column, mask);
        }
      }
    }
  }

  return rasterRowsToToolpath(rows);
}

function rasterizeFilledPolylines(contours: Point[][]): RasterizedToolpath {
  const points = contours.flat();
  if (points.length === 0) return emptyRasterizedToolpath();

  const rows: RasterRows = new Map();
  const minX = Math.min(...points.map((point) => point.x));
  const maxX = Math.max(...points.map((point) => point.x));
  const minY = Math.min(...points.map((point) => point.y));
  const maxY = Math.max(...points.map((point) => point.y));
  const halfPixel = RASTER_PIXEL_MM / 2;
  const minColumn = Math.floor((minX - halfPixel) / RASTER_PIXEL_MM);
  const maxColumn = Math.ceil((maxX + halfPixel) / RASTER_PIXEL_MM);
  const minRow = Math.floor((minY - halfPixel) / RASTER_PIXEL_MM);
  const maxRow = Math.ceil((maxY + halfPixel) / RASTER_PIXEL_MM);

  for (let row = minRow; row <= maxRow; row += 1) {
    const y = row * RASTER_PIXEL_MM;
    for (let column = minColumn; column <= maxColumn; column += 1) {
      const x = column * RASTER_PIXEL_MM;
      const mask = sampleCoverageMask(x, y, (sample) => isInsideFilledPath(sample, contours));
      addCoverageMask(rows, row, column, mask);
    }
  }

  return rasterRowsToToolpath(rows);
}

function isInsideFilledPath(point: Point, contours: Point[][]): boolean {
  let winding = 0;
  for (const contour of contours) {
    for (let index = 0; index < contour.length; index += 1) {
      const from = contour[index];
      const to = contour[(index + 1) % contour.length];
      const side = (to.x - from.x) * (point.y - from.y) - (point.x - from.x) * (to.y - from.y);
      if (from.y <= point.y && to.y > point.y && side > 0) winding += 1;
      else if (from.y > point.y && to.y <= point.y && side < 0) winding -= 1;
    }
  }
  return winding !== 0;
}

function rasterRowsToToolpath(rows: RasterRows): RasterizedToolpath {
  const orderedRows = [...rows.entries()].sort(([a], [b]) => a - b);
  const segments: Point[][] = [];
  const intensities: number[] = [];
  const usedLevels = new Set<number>();
  let pixelCount = 0;
  let minimumColumn = Number.POSITIVE_INFINITY;
  let maximumColumn = Number.NEGATIVE_INFINITY;

  for (const [rowIndex, [row, columnMap]] of orderedRows.entries()) {
    const columns = [...columnMap.keys()].sort((a, b) => a - b);
    pixelCount += columns.length;
    minimumColumn = Math.min(minimumColumn, columns[0]);
    maximumColumn = Math.max(maximumColumn, columns.at(-1)!);
    const runs: Array<[number, number, number]> = [];
    let runStart = columns[0];
    let runEnd = columns[0];
    let runLevel = coverageLevel(columnMap.get(columns[0])!);
    usedLevels.add(runLevel);

    for (const column of columns.slice(1)) {
      const level = coverageLevel(columnMap.get(column)!);
      usedLevels.add(level);
      if (column === runEnd + 1 && level === runLevel) runEnd = column;
      else {
        runs.push([runStart, runEnd, runLevel]);
        runStart = column;
        runEnd = column;
        runLevel = level;
      }
    }
    runs.push([runStart, runEnd, runLevel]);

    const y = roundRasterCoordinate(row * RASTER_PIXEL_MM);
    const orderedRuns = rowIndex % 2 === 0 ? runs : runs.reverse();
    for (const [start, end, level] of orderedRuns) {
      const left = roundRasterCoordinate((start - 0.5) * RASTER_PIXEL_MM);
      const right = roundRasterCoordinate((end + 0.5) * RASTER_PIXEL_MM);
      segments.push(rowIndex % 2 === 0 ? [{ x: left, y }, { x: right, y }] : [{ x: right, y }, { x: left, y }]);
      intensities.push(level / GRAYSCALE_LEVELS);
    }
  }

  if (orderedRows.length === 0) return emptyRasterizedToolpath();
  const firstRow = orderedRows[0][0];
  const lastRow = orderedRows.at(-1)![0];
  return {
    segments,
    intensities,
    bounds: {
      minX: roundRasterCoordinate((minimumColumn - 0.5) * RASTER_PIXEL_MM),
      maxX: roundRasterCoordinate((maximumColumn + 0.5) * RASTER_PIXEL_MM),
      minY: roundRasterCoordinate((firstRow - 0.5) * RASTER_PIXEL_MM),
      maxY: roundRasterCoordinate((lastRow + 0.5) * RASTER_PIXEL_MM)
    },
    pixelCount,
    scanlineCount: orderedRows.length,
    grayscaleLevels: usedLevels.size
  };
}

function emptyRasterizedToolpath(): RasterizedToolpath {
  return { segments: [], intensities: [], bounds: null, pixelCount: 0, scanlineCount: 0, grayscaleLevels: 0 };
}

function sampleCoverageMask(x: number, y: number, isCovered: (sample: Point) => boolean): number {
  let mask = 0;
  for (const [index, offset] of SUBPIXEL_OFFSETS.entries()) {
    if (isCovered({ x: x + offset.x, y: y + offset.y })) mask |= 1 << index;
  }
  return mask;
}

function addCoverageMask(rows: RasterRows, row: number, column: number, mask: number): void {
  if (mask === 0) return;
  const columns = rows.get(row) ?? new Map<number, number>();
  columns.set(column, (columns.get(column) ?? 0) | mask);
  rows.set(row, columns);
}

function coverageLevel(mask: number): number {
  let level = 0;
  for (let value = mask; value !== 0; value &= value - 1) level += 1;
  return level;
}

function squaredDistanceToSegment(point: Point, from: Point, to: Point): number {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared < 1e-12) return (point.x - from.x) ** 2 + (point.y - from.y) ** 2;
  const projection = Math.max(0, Math.min(1, ((point.x - from.x) * dx + (point.y - from.y) * dy) / lengthSquared));
  const nearestX = from.x + projection * dx;
  const nearestY = from.y + projection * dy;
  return (point.x - nearestX) ** 2 + (point.y - nearestY) ** 2;
}

function roundRasterCoordinate(value: number): number {
  const rounded = Math.round(value * 1_000) / 1_000;
  return Math.abs(rounded) < 0.0005 ? 0 : rounded;
}

function iconCenter(settings: LaserSettings): Point {
  return { x: settings.offsetX, y: settings.offsetY };
}

function keycapBounds(settings: LaserSettings): Bounds {
  const minX = -settings.keycapWidth / 2;
  const minY = -settings.keycapHeight / 2;
  return { minX, maxX: minX + settings.keycapWidth, minY, maxY: minY + settings.keycapHeight };
}

function pathStats(
  segments: Point[][]
): Omit<JobStats, "pixelCount" | "scanlineCount" | "grayscaleLevels" | "bounds" | "workArea" | "fitsKeycap"> {
  let cutsMm = 0;
  let travelMm = 0;
  let previousEnd: Point | null = null;
  let motionSegments = 0;
  let pointCount = 0;

  for (const segment of segments) {
    if (segment.length < 2) continue;
    motionSegments += 1;
    pointCount += segment.length;
    if (previousEnd) travelMm += distance(previousEnd, segment[0]);
    for (let index = 1; index < segment.length; index += 1) cutsMm += distance(segment[index - 1], segment[index]);
    previousEnd = segment.at(-1)!;
  }

  return { motionSegments, pointCount, travelMm, cutsMm };
}

function boundsInside(inner: Bounds, outer: Bounds): boolean {
  const tolerance = 0.0001;
  return (
    inner.minX >= outer.minX - tolerance &&
    inner.maxX <= outer.maxX + tolerance &&
    inner.minY >= outer.minY - tolerance &&
    inner.maxY <= outer.maxY + tolerance
  );
}

function rectToPolyline(attrs: SVGProps, quality: number): Point[] {
  const x = numberAttr(attrs.x);
  const y = numberAttr(attrs.y);
  const width = numberAttr(attrs.width);
  const height = numberAttr(attrs.height);
  const rx = Math.min(numberAttr(attrs.rx), width / 2);
  const ry = Math.min(numberAttr(attrs.ry ?? attrs.rx), height / 2);
  if (!rx || !ry) return [{ x, y }, { x: x + width, y }, { x: x + width, y: y + height }, { x, y: y + height }, { x, y }];

  const path = `M${x + rx} ${y}H${x + width - rx}A${rx} ${ry} 0 0 1 ${x + width} ${y + ry}V${y + height - ry}A${rx} ${ry} 0 0 1 ${x + width - rx} ${y + height}H${x + rx}A${rx} ${ry} 0 0 1 ${x} ${y + height - ry}V${y + ry}A${rx} ${ry} 0 0 1 ${x + rx} ${y}Z`;
  return pathToPolylines(path, quality)[0] ?? [];
}

function ellipse(cx: number, cy: number, rx: number, ry: number, quality: number): Point[] {
  const samples = Math.max(24, quality);
  return Array.from({ length: samples + 1 }, (_, index) => {
    const angle = (Math.PI * 2 * index) / samples;
    return { x: cx + Math.cos(angle) * rx, y: cy + Math.sin(angle) * ry };
  });
}

function parsePoints(value: string | number | undefined): Point[] {
  const numbers = String(value ?? "").match(/[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?/g)?.map(Number) ?? [];
  const points: Point[] = [];
  for (let index = 0; index < numbers.length - 1; index += 2) points.push({ x: numbers[index], y: numbers[index + 1] });
  return points;
}

function point(x: string | number | undefined, y: string | number | undefined): Point {
  return { x: numberAttr(x), y: numberAttr(y) };
}

function numberAttr(value: string | number | undefined): number {
  const parsed = Number.parseFloat(String(value ?? 0));
  return Number.isFinite(parsed) ? parsed : 0;
}

function dedupeAdjacentPoints(points: Point[]): Point[] {
  return points.filter((point, index) => index === 0 || !pointsNearlyEqual(point, points[index - 1]));
}

function pointsNearlyEqual(a: Point, b: Point): boolean {
  return Math.abs(a.x - b.x) < 0.00001 && Math.abs(a.y - b.y) < 0.00001;
}

function distance(a: Point, b: Point): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

function clampNumber(value: number | undefined, min: number, max: number, fallback: number): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.min(max, Math.max(min, parsed)) : fallback;
}

function clampInteger(value: number | undefined, min: number, max: number, fallback: number): number {
  return Math.round(clampNumber(value, min, max, fallback));
}
