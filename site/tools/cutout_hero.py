#!/usr/bin/env python3
"""Knock the flat backdrop out of the hero illustrations.

Both hero images are keyboards sitting on a flat studio backdrop. A plain
colour key cannot separate them: in the day image the case's lit cream sits
only ~15 RGB units from the backdrop cream, so any tolerance wide enough to
swallow the baked drop shadow (~55-70 units away) also eats the case.

Two things make the separation reliable instead:

  * Reachability. Removal floods inward from the border, so a colour is only
    deleted when a path of removable pixels connects it to the outside. The
    case survives because the flood has to cross it to get there.
  * Chroma. The drop shadow is the backdrop darkened *warmly* — its blue
    channel falls faster than its red — while the case is neutral-to-cool.
    Normalising each channel against the backdrop separates them cleanly
    (shadow ~0.85, case ~1.0-1.11), so the flood may pass through shadow
    without being allowed into the case.

Isolated specks left behind (the night sky's stars) are dropped afterwards by
keeping only the one large component the keyboard forms.

    python3 site/tools/cutout_hero.py in.webp out.png [--shadow]
"""
import sys

import numpy as np
from PIL import Image
from scipy import ndimage

# Backdrop match tolerance, in RGB units. The backdrops are near-uniform (a few
# units of noise), so this stays tight enough to exclude the day case's lit
# cream at ~15 units.
BACKDROP_TOLERANCE = 11.0

# How far from the backdrop a warm-darkened pixel may sit and still count as
# drop shadow rather than product.
SHADOW_MAX_DISTANCE = 95.0

# Blue-vs-red ratio (both normalised against the backdrop) below which a
# darker pixel reads as warm shadow rather than neutral casework.
SHADOW_CHROMA_RATIO = 0.95

# Opaque islands smaller than this fraction of the main subject are debris.
ISLAND_FRACTION = 0.02


def backdrop_colour(rgb: np.ndarray) -> np.ndarray:
    """Median of the four corner patches — robust to a star or a stray speck."""
    h, w, _ = rgb.shape
    k = 24
    corners = np.concatenate([
        rgb[:k, :k].reshape(-1, 3),
        rgb[:k, w - k:].reshape(-1, 3),
        rgb[h - k:, :k].reshape(-1, 3),
        rgb[h - k:, w - k:].reshape(-1, 3),
    ])
    return np.median(corners, axis=0)


def removable_mask(rgb: np.ndarray, bg: np.ndarray, drop_shadow: bool) -> np.ndarray:
    """Pixels the flood is *allowed* to delete, before reachability is applied."""
    dist = np.linalg.norm(rgb - bg, axis=2)
    allowed = dist <= BACKDROP_TOLERANCE

    if drop_shadow:
        safe = np.maximum(bg, 1.0)
        norm = rgb / safe                       # per-channel, relative to backdrop
        with np.errstate(divide="ignore", invalid="ignore"):
            chroma = np.where(norm[..., 0] > 0.01, norm[..., 2] / norm[..., 0], 1.0)
        darker = rgb.sum(axis=2) < bg.sum()
        allowed |= (dist <= SHADOW_MAX_DISTANCE) & (chroma <= SHADOW_CHROMA_RATIO) & darker

    return allowed


def flood_from_border(allowed: np.ndarray) -> np.ndarray:
    """Removable pixels connected to the image border."""
    labels, count = ndimage.label(allowed)
    if count == 0:
        return np.zeros_like(allowed)
    edge = np.concatenate([labels[0, :], labels[-1, :], labels[:, 0], labels[:, -1]])
    touching = set(int(v) for v in np.unique(edge) if v)
    return np.isin(labels, list(touching)) if touching else np.zeros_like(allowed)


def drop_islands(opaque: np.ndarray) -> np.ndarray:
    """Keep the subject; discard specks such as background stars."""
    labels, count = ndimage.label(opaque)
    if count <= 1:
        return opaque
    sizes = ndimage.sum(opaque, labels, range(1, count + 1))
    keep = [i + 1 for i, s in enumerate(sizes) if s >= sizes.max() * ISLAND_FRACTION]
    dropped = count - len(keep)
    if dropped:
        print(f"  dropped {dropped} isolated island(s)")
    return np.isin(labels, keep)


def main() -> int:
    src, dst = sys.argv[1], sys.argv[2]
    drop_shadow = "--shadow" in sys.argv[3:]

    image = Image.open(src).convert("RGB")
    rgb = np.asarray(image).astype(np.float64)

    bg = backdrop_colour(rgb)
    allowed = removable_mask(rgb, bg, drop_shadow)
    background = flood_from_border(allowed)
    opaque = drop_islands(~background)

    out = np.dstack([np.asarray(image), np.where(opaque, 255, 0).astype(np.uint8)])
    Image.fromarray(out, "RGBA").save(dst)

    pct = 100.0 * (1.0 - opaque.mean())
    print(f"  backdrop rgb({int(bg[0])},{int(bg[1])},{int(bg[2])}) -> {pct:.1f}% transparent")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
