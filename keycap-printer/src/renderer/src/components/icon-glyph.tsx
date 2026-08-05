import { createElement, type SVGProps } from "react";
import type { IconNode } from "lucide";
import { cn } from "@/lib/utils";

interface IconGlyphProps extends SVGProps<SVGSVGElement> {
  node?: IconNode;
  brandPath?: string;
}

export function IconGlyph({ node, brandPath, className, ...props }: IconGlyphProps) {
  const isBrand = typeof brandPath === "string";
  return (
    <svg
      viewBox="0 0 24 24"
      fill={isBrand ? "currentColor" : "none"}
      stroke={isBrand ? "none" : "currentColor"}
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={cn("size-5", className)}
      aria-hidden="true"
      {...props}
    >
      {brandPath ? <path d={brandPath} /> : node?.map(([tag, attrs], index) => createElement(tag, { ...attrs, key: `${tag}-${index}` }))}
    </svg>
  );
}
