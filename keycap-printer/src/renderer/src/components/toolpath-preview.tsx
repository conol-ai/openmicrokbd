import { useMemo } from "react";
import type { LaserJob, LaserSettings } from "@/lib/toolpath";

interface ToolpathPreviewProps {
  job: LaserJob | null;
  settings: LaserSettings;
}

export function ToolpathPreview({ job, settings }: ToolpathPreviewProps) {
  const geometry = useMemo(() => {
    const pad = 2;
    const area = job?.stats.workArea ?? {
      minX: -settings.keycapWidth / 2,
      maxX: settings.keycapWidth / 2,
      minY: -settings.keycapHeight / 2,
      maxY: settings.keycapHeight / 2
    };
    const bounds = job?.stats.bounds;
    const minX = Math.min(area.minX, bounds?.minX ?? area.minX, 0) - pad;
    const maxX = Math.max(area.maxX, bounds?.maxX ?? area.maxX, 0) + pad;
    const minY = Math.min(area.minY, bounds?.minY ?? area.minY, 0) - pad;
    const maxY = Math.max(area.maxY, bounds?.maxY ?? area.maxY, 0) + pad;
    const step = 2;
    const vertical = range(Math.floor(minX / step) * step, Math.ceil(maxX / step) * step, step);
    const horizontal = range(Math.floor(minY / step) * step, Math.ceil(maxY / step) * step, step);
    return { area, minX, maxX, minY, maxY, vertical, horizontal };
  }, [job, settings]);

  const width = geometry.maxX - geometry.minX;
  const height = geometry.maxY - geometry.minY;

  return (
    <div className="relative grid min-h-[390px] flex-1 place-items-center overflow-hidden bg-preview p-5">
      <svg
        viewBox={`${geometry.minX} ${-geometry.maxY} ${width} ${height}`}
        className="aspect-square h-full max-h-[560px] w-full max-w-[680px]"
        role="img"
        aria-label="Laser toolpath preview"
      >
        <g transform="scale(1 -1)">
          <g className="stroke-grid" strokeWidth="0.025">
            {geometry.vertical.map((x) => (
              <line key={`x-${x}`} x1={x} y1={geometry.minY} x2={x} y2={geometry.maxY} />
            ))}
            {geometry.horizontal.map((y) => (
              <line key={`y-${y}`} x1={geometry.minX} y1={y} x2={geometry.maxX} y2={y} />
            ))}
          </g>
          <rect
            x={geometry.area.minX}
            y={geometry.area.minY}
            width={geometry.area.maxX - geometry.area.minX}
            height={geometry.area.maxY - geometry.area.minY}
            rx="1.1"
            className="fill-keycap stroke-keycap"
            strokeWidth="0.09"
          />
          <g className="stroke-origin" strokeWidth="0.08">
            <line x1={-1} y1={0} x2={1} y2={0} />
            <line x1={0} y1={-1} x2={0} y2={1} />
          </g>
          <g className="fill-none stroke-toolpath" strokeWidth="0.13" strokeLinecap="round" strokeLinejoin="round">
            {job?.fillPlans[0]?.segments.map((segment, index) => (
              <polyline
                key={index}
                points={segment.map(({ x, y }) => `${x},${y}`).join(" ")}
                strokeOpacity={0.12 + job.fillPlans[0].intensities[index] * 0.88}
              />
            ))}
          </g>
          <g className="fill-none stroke-toolpath" strokeWidth="0.18" strokeLinecap="round" strokeLinejoin="round">
            {job?.edges.map((edge, index) => (
              <polyline key={index} points={edge.map(({ x, y }) => `${x},${y}`).join(" ")} />
            ))}
          </g>
        </g>
      </svg>

      {job && !job.stats.fitsKeycap ? (
        <div className="absolute left-4 top-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs font-medium text-destructive">
          Toolpath exceeds keycap boundary
        </div>
      ) : null}
    </div>
  );
}

function range(start: number, end: number, step: number): number[] {
  const values: number[] = [];
  for (let value = start; value <= end; value += step) values.push(value);
  return values;
}
