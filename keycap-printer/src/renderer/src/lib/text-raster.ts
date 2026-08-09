import { buildSamplerJob, normalizeSettings, type InkSampler, type LaserJob, type LaserSettings } from "./toolpath";

export interface TextSpec {
  text: string;
  fontFamily: string;
  fontWeight: number;
  fontSizeMm: number;
}

export const DEFAULT_TEXT_SPEC: TextSpec = {
  text: "A",
  fontFamily: "Helvetica Neue",
  fontWeight: 600,
  fontSizeMm: 8
};

export const FONT_WEIGHTS = [
  { value: 100, label: "100 Thin" },
  { value: 200, label: "200 ExtraLight" },
  { value: 300, label: "300 Light" },
  { value: 400, label: "400 Regular" },
  { value: 500, label: "500 Medium" },
  { value: 600, label: "600 SemiBold" },
  { value: 700, label: "700 Bold" },
  { value: 800, label: "800 ExtraBold" },
  { value: 900, label: "900 Black" }
] as const;

export const FALLBACK_FONT_FAMILIES = [
  "Arial",
  "Arial Black",
  "Avenir Next",
  "Courier New",
  "Futura",
  "Georgia",
  "Helvetica Neue",
  "Impact",
  "Menlo",
  "Monaco",
  "Times New Roman",
  "Trebuchet MS",
  "Verdana"
];

// One canvas pixel per 4x4 supersample step: 0.025 mm.
const TEXT_PX_PER_MM = 40;
const CANVAS_MARGIN_PX = 8;
const MAX_CANVAS_PX = 16384;
const INK_ALPHA_THRESHOLD = 128;

export function buildTextJob(spec: TextSpec, settings: Partial<LaserSettings>): LaserJob {
  const normalized = normalizeSettings(settings);
  return buildSamplerJob(createTextSampler(spec, normalized), normalized);
}

function createTextSampler(spec: TextSpec, settings: LaserSettings): InkSampler {
  if (!spec.text.trim()) throw new Error("Enter text to engrave");
  const fontSizeMm = Math.min(70, Math.max(1, spec.fontSizeMm));
  const weight = Math.min(900, Math.max(100, Math.round(spec.fontWeight / 100) * 100));
  const font = `${weight} ${fontSizeMm * TEXT_PX_PER_MM}px ${JSON.stringify(spec.fontFamily)}`;

  const canvas = document.createElement("canvas");
  const measureContext = canvas.getContext("2d", { willReadFrequently: true });
  if (!measureContext) throw new Error("Canvas 2D is unavailable");
  measureContext.font = font;
  measureContext.textBaseline = "alphabetic";
  const metrics = measureContext.measureText(spec.text);
  const inkWidth = metrics.actualBoundingBoxLeft + metrics.actualBoundingBoxRight;
  const inkHeight = metrics.actualBoundingBoxAscent + metrics.actualBoundingBoxDescent;
  if (inkWidth <= 0 || inkHeight <= 0) throw new Error("The text has no engravable ink");

  canvas.width = Math.ceil(inkWidth) + CANVAS_MARGIN_PX * 2;
  canvas.height = Math.ceil(inkHeight) + CANVAS_MARGIN_PX * 2;
  if (canvas.width > MAX_CANVAS_PX || canvas.height > MAX_CANVAS_PX) {
    throw new Error("The text is too large to rasterize; reduce the font size or shorten the text");
  }

  // Resizing the canvas reset its state; configure the context again.
  const context = canvas.getContext("2d", { willReadFrequently: true })!;
  context.font = font;
  context.textBaseline = "alphabetic";
  context.fillStyle = "#000";
  context.fillText(spec.text, CANVAS_MARGIN_PX + metrics.actualBoundingBoxLeft, CANVAS_MARGIN_PX + metrics.actualBoundingBoxAscent);
  const image = context.getImageData(0, 0, canvas.width, canvas.height);

  const center = { x: CANVAS_MARGIN_PX + inkWidth / 2, y: CANVAS_MARGIN_PX + inkHeight / 2 };
  const halfWidthMm = inkWidth / (2 * TEXT_PX_PER_MM);
  const halfHeightMm = inkHeight / (2 * TEXT_PX_PER_MM);
  const radians = (settings.rotation * Math.PI) / 180;
  const cos = Math.cos(radians);
  const sin = Math.sin(radians);
  const mirrorX = settings.mirrorX ? -1 : 1;
  const mirrorY = settings.mirrorY ? -1 : 1;
  const origin = { x: settings.offsetX, y: settings.offsetY };

  const boundsHalfWidth = Math.abs(cos) * halfWidthMm + Math.abs(sin) * halfHeightMm;
  const boundsHalfHeight = Math.abs(sin) * halfWidthMm + Math.abs(cos) * halfHeightMm;

  return {
    bounds: {
      minX: origin.x - boundsHalfWidth,
      maxX: origin.x + boundsHalfWidth,
      minY: origin.y - boundsHalfHeight,
      maxY: origin.y + boundsHalfHeight
    },
    isInk(x: number, y: number): boolean {
      // Invert the machine transform: translate, un-rotate, un-mirror, then y-flip into canvas space.
      const dx = x - origin.x;
      const dy = y - origin.y;
      const localX = (dx * cos + dy * sin) * mirrorX;
      const localY = (-dx * sin + dy * cos) * mirrorY;
      const column = Math.floor(center.x + localX * TEXT_PX_PER_MM);
      const rowIndex = Math.floor(center.y - localY * TEXT_PX_PER_MM);
      if (column < 0 || rowIndex < 0 || column >= image.width || rowIndex >= image.height) return false;
      return image.data[(rowIndex * image.width + column) * 4 + 3] >= INK_ALPHA_THRESHOLD;
    }
  };
}
