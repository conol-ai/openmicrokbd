#!/usr/bin/env python3
"""Original pixel-art footer landscape for openmicrokbd.org, day + night.

Drawn from scratch at 384x256 (chunky 2px feature grid), upscaled x4 nearest
to 1536x1024. Same geometry for both variants; only the palette changes.
Landmark: keycap-shaped standing stones ("keyhenge") — no borrowed scenery.

Regenerate the shipped assets with:
    python3 site/tools/gen_landscape.py
    cwebp -lossless landscape_day.png   -o site/public/assets/landscape_day.webp
    cwebp -lossless landscape_night.png -o site/public/assets/landscape_night.webp
"""
import math
import random
from PIL import Image

W, H = 384, 256
SCALE = 4

DAY = {
    "sky":       ["#5fc4ee", "#74cef2", "#8fd9f6", "#aee7fb", "#c9f0fd", "#e6f8ff"],
    "sun":       "#ffd95e", "sun_core": "#ffe9a0",
    "cloud":     "#fffaec", "cloud_sh": "#e8e0cc",
    "mountain":  "#8fb4c9", "mountain_hi": "#c8dde9",
    "hill_back": "#78b25c", "hill_mid": "#5d9c40", "hill_front": "#4a8a33",
    "ground":    "#3d7029",
    "tree_dark": "#2c5220", "tree_lite": "#3a6a2a", "trunk": "#5c3a22",
    "path":      "#d9b877", "path_edge": "#bfa066",
    "stone":     "#e9e1cd", "stone_sh": "#c9c0aa", "stone_hi": "#f7f1de",
    "legend":    "#6b5b45",
    "flowers":   ["#c9543c", "#f5c53d", "#fffaec", "#d97757"],
    "star": None, "firefly": None, "moon": None, "moon_sh": None,
}

NIGHT = {
    "sky":       ["#0a0f28", "#0e1430", "#131a3a", "#182046", "#1f2852", "#28325f"],
    "sun":       None, "sun_core": None,
    "cloud":     "#39426e", "cloud_sh": "#2c3459",
    "mountain":  "#232e52", "mountain_hi": "#39476e",
    "hill_back": "#2c4d42", "hill_mid": "#24413a", "hill_front": "#1d3630",
    "ground":    "#172b26",
    "tree_dark": "#10201c", "tree_lite": "#182e28", "trunk": "#241a12",
    "path":      "#5d5847", "path_edge": "#494538",
    "stone":     "#a7aecb", "stone_sh": "#7d84a6", "stone_hi": "#c6cce4",
    "legend":    "#3c405c",
    "flowers":   ["#5a4a6e", "#6e5a3c"],
    "star":      "#fff3c4", "firefly": "#ffe28a",
    "moon":      "#ffd977", "moon_sh": "#e0b452",
}


def hx(c):
    return tuple(int(c[i:i + 2], 16) for i in (1, 3, 5))


def q2(v):
    return int(v) // 2 * 2


class Px:
    def __init__(self, palette):
        self.p = palette
        self.im = Image.new("RGB", (W, H))
        self.px = self.im.load()

    def rect(self, x0, y0, x1, y1, color):
        c = hx(color)
        for y in range(max(0, y0), min(H, y1)):
            for x in range(max(0, x0), min(W, x1)):
                self.px[x, y] = c

    def dot(self, x, y, color, s=2):
        self.rect(x, y, x + s, y + s, color)


def sky(cv):
    bands = cv.p["sky"]
    n = len(bands)
    horizon = 132
    for i, c in enumerate(bands):
        y0 = 0 if i == 0 else int(horizon * i / n)
        y1 = horizon if i == n - 1 else int(horizon * (i + 1) / n)
        cv.rect(0, y0, W, y1 + 40 if i == n - 1 else y1, c)
    # bottom of frame gets ground fill later; extend last band a bit under horizon
    cv.rect(0, horizon, W, H, bands[-1])


def stars(cv, rng):
    if not cv.p["star"]:
        return
    for _ in range(90):
        x, y = rng.randrange(0, W, 2), rng.randrange(0, 110, 2)
        s = 2 if rng.random() < 0.85 else 3
        cv.dot(x, y, cv.p["star"], s)
    # a few 5px twinkles
    for _ in range(6):
        x, y = rng.randrange(8, W - 8, 2), rng.randrange(8, 90, 2)
        cv.dot(x, y, cv.p["star"], 2)
        cv.dot(x - 2, y, cv.p["star"], 2)
        cv.dot(x + 2, y, cv.p["star"], 2)
        cv.dot(x, y - 2, cv.p["star"], 2)
        cv.dot(x, y + 2, cv.p["star"], 2)


def sun_or_moon(cv):
    cx, cy, r = 316, 38, 17
    if cv.p["sun"]:
        for y in range(cy - r - 2, cy + r + 2):
            for x in range(cx - r - 2, cx + r + 2):
                d = math.hypot(x - cx, y - cy)
                if d <= r - 6:
                    cv.px[x, y] = hx(cv.p["sun_core"])
                elif d <= r:
                    cv.px[x, y] = hx(cv.p["sun"])
    elif cv.p["moon"]:
        for y in range(cy - r - 2, cy + r + 2):
            for x in range(cx - r - 2, cx + r + 2):
                d = math.hypot(x - cx, y - cy)
                d2 = math.hypot(x - (cx + 9), y - (cy - 4))
                if d <= r and d2 > r - 3:
                    c = cv.p["moon"] if d <= r - 4 else cv.p["moon_sh"]
                    cv.px[x, y] = hx(c)


def clouds(cv, rng):
    spots = [(40, 30, 1.4), (150, 52, 1.0), (250, 22, 1.2), (330, 66, 0.8), (90, 74, 0.7)]
    for cx, cy, s in spots:
        wds = [(0, 0, 30, 8), (8, -6, 26, 8), (18, -10, 18, 6), (-10, 2, 16, 6)]
        for dx, dy, ww, hh in wds:
            x0 = q2(cx + dx * s)
            y0 = q2(cy + dy * s)
            cv.rect(x0, y0, x0 + q2(ww * s), y0 + q2(hh * s), cv.p["cloud"])
        x0 = q2(cx - 10 * s)
        cv.rect(x0, q2(cy + 8 * s), x0 + q2(52 * s), q2(cy + 8 * s) + 2, cv.p["cloud_sh"])


def ridge(cv, base, amp, lam, phase, color, ymax=H):
    ys = []
    for x in range(W):
        y = base + amp * math.sin(x / lam + phase) + amp * 0.5 * math.sin(x / (lam * 0.37) + phase * 2.1)
        ys.append(q2(y))
    for x in range(W):
        cv.rect(x, ys[x], x + 1, ymax, color)
    return ys


def mountains(cv):
    peaks = [(-20, 128), (30, 96), (75, 124), (120, 88), (170, 122), (215, 100),
             (262, 126), (305, 106), (352, 124), (404, 100)]
    def yat(x):
        for i in range(len(peaks) - 1):
            x0, y0 = peaks[i]
            x1, y1 = peaks[i + 1]
            if x0 <= x <= x1:
                t = (x - x0) / (x1 - x0)
                return y0 + (y1 - y0) * t
        return 128
    for x in range(W):
        y = q2(yat(x))
        cv.rect(x, y, x + 1, 140, cv.p["mountain"])
        # lit ridgeline
        cv.rect(x, y, x + 1, y + 2, cv.p["mountain_hi"])


def tree(cv, x, y, s):
    """Pine: stacked shrinking slabs. (x, y) = base center, s = height factor."""
    trunk_h = max(3, int(3 * s))
    cv.rect(x - 1, y - trunk_h, x + 1, y, cv.p["trunk"])
    layers = max(3, int(4 * s))
    top = y - trunk_h
    for i in range(layers):
        w = q2((layers - i) * 3 * s) + 2
        hgt = max(2, q2(3 * s))
        yy = top - (i + 1) * hgt
        cv.rect(x - w // 2, yy, x + w // 2, yy + hgt, cv.p["tree_dark"] if i % 2 == 0 else cv.p["tree_lite"])


def keycap_stone(cv, x, y, s, legend=None):
    """A keycap-shaped monolith. (x, y) = bottom-left, s = width."""
    h = int(s * 0.92)
    top = y - h
    # body with 2px cut corners
    cv.rect(x + 2, top, x + s - 2, y, cv.p["stone"])
    cv.rect(x, top + 2, x + s, y - 2, cv.p["stone"])
    # top highlight + right/bottom shade rings
    cv.rect(x + 2, top, x + s - 2, top + 2, cv.p["stone_hi"])
    cv.rect(x, top + 2, x + 2, y - 2, cv.p["stone_hi"])
    cv.rect(x + s - 2, top + 2, x + s, y - 2, cv.p["stone_sh"])
    cv.rect(x + 2, y - 2, x + s - 2, y, cv.p["stone_sh"])
    # inner face (the keycap "top surface")
    m = max(3, s // 6)
    cv.rect(x + m, top + m, x + s - m, y - m - 1, cv.p["stone_hi"])
    if legend == "smile":
        cx, cy = x + s // 2, top + h // 2
        e = max(2, s // 10)
        # two square eyes
        cv.dot(cx - 2 * e, cy - 2 * e, cv.p["legend"], e)
        cv.dot(cx + e, cy - 2 * e, cv.p["legend"], e)
        # U-shaped mouth
        cv.rect(cx - 2 * e, cy + e, cx + 2 * e, cy + 2 * e, cv.p["legend"])
        cv.rect(cx - 3 * e, cy - e // 2, cx - 2 * e, cy + 2 * e, cv.p["legend"])
        cv.rect(cx + 2 * e, cy - e // 2, cx + 3 * e, cy + 2 * e, cv.p["legend"])
    elif legend == "plus":
        # the "unassigned key" glyph from the demo pad
        cx, cy = x + s // 2, top + h // 2
        e = max(2, s // 10)
        cv.rect(cx - e, cy - 3 * e, cx + e, cy + 3 * e, cv.p["legend"])
        cv.rect(cx - 3 * e, cy - e, cx + 3 * e, cy + e, cv.p["legend"])
    # grass tuft at the base
    cv.rect(x - 2, y - 2, x + s + 2, y, cv.p["hill_front"])


def path(cv, ys_front):
    """Winding dirt path from bottom edge up to the front ridge."""
    x0 = 208
    for yy in range(H - 1, 150, -1):
        t = (H - 1 - yy) / (H - 150)          # 0 at bottom, 1 at top
        wobble = 26 * math.sin(t * 4.2) + 10 * math.sin(t * 9.1)
        wdt = max(4, int(30 * (1 - t) ** 1.6) + 4)
        cx = int(x0 + wobble - 30 * t)
        if yy % 2:
            continue
        cv.rect(q2(cx - wdt // 2) - 2, yy, q2(cx - wdt // 2), yy + 2, cv.p["path_edge"])
        cv.rect(q2(cx - wdt // 2), yy, q2(cx + wdt // 2), yy + 2, cv.p["path"])
        cv.rect(q2(cx + wdt // 2), yy, q2(cx + wdt // 2) + 2, yy + 2, cv.p["path_edge"])


def flowers(cv, rng):
    key = "firefly" if cv.p["firefly"] else None
    for _ in range(130):
        x = rng.randrange(0, W, 2)
        y = rng.randrange(198, H - 2, 2)
        if key:
            if rng.random() < 0.25:
                cv.dot(x, y - rng.randrange(0, 40, 2), cv.p[key], 2)
        else:
            cv.dot(x, y, rng.choice(cv.p["flowers"]), 2)
    # grass texture ticks
    tex = cv.p["hill_front"]
    for _ in range(220):
        x = rng.randrange(0, W, 2)
        y = rng.randrange(192, H - 2, 2)
        cv.dot(x, y, tex, 2)


def render(palette, out):
    rng = random.Random(20260810)
    cv = Px(palette)
    sky(cv)
    stars(cv, rng)
    sun_or_moon(cv)
    clouds(cv, rng)
    mountains(cv)

    ys_back = ridge(cv, 138, 7, 60, 0.8, palette["hill_back"])
    # trees on the back ridge
    for tx in (18, 44, 62, 120, 205, 238, 356, 374):
        tree(cv, tx, ys_back[min(tx, W - 1)] + 6, 0.8)

    ys_mid = ridge(cv, 168, 9, 48, 2.4, palette["hill_mid"])
    for tx in (86, 104, 282, 306, 328):
        tree(cv, tx, ys_mid[min(tx, W - 1)] + 8, 1.15)
    keycap_stone(cv, 148, ys_mid[152] + 12, 16)

    ys_front = ridge(cv, 200, 10, 74, 4.9, palette["hill_front"])
    cv.rect(0, 224, W, H, palette["ground"])
    flowers(cv, rng)
    path(cv, ys_front)
    # keyhenge on the front hill
    keycap_stone(cv, 64, 214, 30, legend="plus")
    keycap_stone(cv, 286, 226, 40, legend="smile")
    keycap_stone(cv, 340, 220, 22)

    big = cv.im.resize((W * SCALE, H * SCALE), Image.NEAREST)
    big.save(out)
    print("wrote", out, big.size)


if __name__ == "__main__":
    render(DAY, "landscape_day.png")
    render(NIGHT, "landscape_night.png")
