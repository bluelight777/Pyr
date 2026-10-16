#!/usr/bin/env python3
"""Draw the pictures of the skins Firelit Games makes for the Ingle Library.

    python3 tools/make-skin-art.py

Writes skins/blood-moon/background.webp: a blood-red moon rising over a dark ridge, under a
night sky with a few stars and a red mist. It's drawn from numbers (a fixed random seed), so
running this again gives the same picture. Needs Pillow with WebP (pip install pillow).

The picture is seen through Ingle's colours (30% of it, behind the chat), so it's dark with
one strong shape, the moon, a little right of the middle and high, where it shows between the
messages and the member list.
"""

import math
import os
import random

from PIL import Image, ImageChops, ImageDraw, ImageFilter

W, H = 2560, 1440
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "skins", "blood-moon", "background.webp")


def lerp(a, b, t):
    return a + (b - a) * t


def mix(c1, c2, t):
    return tuple(int(round(lerp(a, b, t))) for a, b in zip(c1, c2))


def sky(rng):
    """Near-black at the top, deep crimson towards the ridge."""
    top, mid, low = (6, 3, 8), (22, 5, 10), (70, 10, 16)
    img = Image.new("RGB", (1, H))
    for y in range(H):
        t = y / (H - 1)
        c = mix(top, mid, t / 0.55) if t < 0.55 else mix(mid, low, (t - 0.55) / 0.45)
        img.putpixel((0, y), c)
    img = img.resize((W, H))
    # A little grain, so the gradient doesn't band.
    grain = Image.effect_noise((W, H), 4).filter(ImageFilter.GaussianBlur(0.8)).convert("RGB")
    return ImageChops.add(img, grain, scale=1.0, offset=-128)


def stars(img, rng, moon):
    d = ImageDraw.Draw(img)
    mx, my, mr = moon
    for _ in range(900):
        x, y = rng.uniform(0, W), rng.uniform(0, H * 0.62)
        if math.hypot(x - mx, y - my) < mr * 2.2:
            continue
        b = rng.random() ** 3
        c = mix((60, 40, 45), (255, 228, 222), b)
        r = 0.6 + b * 1.6
        d.ellipse((x - r, y - r, x + r, y + r), fill=c)


def moon_layer(rng, moon):
    """The moon on its own layer: lit from its upper right, with craters and a red glow."""
    mx, my, mr = moon
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    # The glow, wide and soft.
    glow = Image.new("L", (W, H), 0)
    g = ImageDraw.Draw(glow)
    for i in range(48, 0, -1):
        r = mr * (1.0 + i * 0.06)
        g.ellipse((mx - r, my - r, mx + r, my + r), fill=int(120 * (1 - i / 48) ** 2.2))
    glow = glow.filter(ImageFilter.GaussianBlur(60))
    red = Image.new("RGBA", (W, H), (190, 20, 30, 255))
    layer = Image.composite(red, layer, glow)
    # The disc, shaded pixel by pixel.
    size = int(mr * 2) + 2
    disc = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    px = disc.load()
    lx, ly, lz = 0.55, -0.45, 0.70  # where the light comes from
    n = math.sqrt(lx * lx + ly * ly + lz * lz)
    lx, ly, lz = lx / n, ly / n, lz / n
    # Craters: (x, y, radius) in the disc's own units (-1..1).
    craters = []
    for _ in range(30):
        a, d = rng.uniform(0, math.tau), math.sqrt(rng.random()) * 0.9
        craters.append((math.cos(a) * d, math.sin(a) * d, rng.uniform(0.02, 0.085)))
    # Darker seas, big and soft.
    seas = [(-0.25, -0.15, 0.38), (0.2, 0.25, 0.3), (-0.05, 0.45, 0.22), (0.35, -0.35, 0.2)]
    base, dark = (196, 30, 36), (110, 12, 20)
    for j in range(size):
        for i in range(size):
            x = (i - size / 2) / mr
            y = (j - size / 2) / mr
            rr = x * x + y * y
            if rr > 1.0:
                continue
            z = math.sqrt(1 - rr)
            lit = max(0.0, x * lx + y * ly + z * lz)
            # Lit from the upper right, and darker towards the rim (limb darkening).
            shade = (0.30 + 0.70 * lit) * (0.62 + 0.38 * z)
            sea = 0.0
            for sx, sy, sr in seas:
                dd = math.hypot(x - sx, y - sy) / sr
                if dd < 1:
                    sea = max(sea, (1 - dd * dd) * 0.55)
            crater = 0.0
            for cx, cy, cr in craters:
                dd = math.hypot(x - cx, y - cy) / cr
                if dd < 1.2:
                    # A soft dark floor and a lit rim on the side towards the light.
                    rim = math.exp(-((dd - 1.0) ** 2) / 0.012)
                    side = ((x - cx) * lx + (y - cy) * ly) / max(1e-6, cr)
                    floor = 0.3 * max(0.0, 1 - dd * dd) ** 0.6
                    crater += floor - rim * 0.25 * side
            t = max(0.0, min(1.0, sea + crater * 0.5))
            c = mix(base, dark, t)
            c = tuple(min(255, int(v * shade)) for v in c)
            edge = min(1.0, (1.0 - math.sqrt(rr)) * mr / 1.5)  # smooth the rim
            px[i, j] = (c[0], c[1], c[2], int(255 * edge))
    layer.alpha_composite(disc, (int(mx - size / 2), int(my - size / 2)))
    return layer


def mist(rng):
    """Red mist low in the sky: soft bands of noise."""
    small = Image.new("L", (W // 16, H // 16), 0)
    p = small.load()
    for y in range(small.height):
        t = y / small.height
        band = max(0.0, 1 - abs(t - 0.74) / 0.16)
        for x in range(small.width):
            p[x, y] = int(255 * band * (0.45 + 0.55 * rng.random()))
    big = small.resize((W, H), Image.BICUBIC).filter(ImageFilter.GaussianBlur(26))
    layer = Image.new("RGBA", (W, H), (140, 18, 28, 0))
    layer.putalpha(big.point(lambda v: int(v * 0.42)))
    return layer


def ridge(rng):
    """A dark ridge along the bottom, with a few pines on it."""
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    pts = []
    phase = [rng.uniform(0, math.tau) for _ in range(4)]
    for x in range(0, W + 8, 8):
        t = x / W
        y = H * 0.885 - (55 * math.sin(t * 2.3 + phase[0]) + 26 * math.sin(t * 6.1 + phase[1]) + 9 * math.sin(t * 17 + phase[2]) + 30)
        pts.append((x, y))
    dark = (8, 3, 5, 255)
    d.polygon(pts + [(W, H), (0, H)], fill=dark)
    # Pines: a trunk and three tiers, thinner towards the top.
    for x, y in pts[::3]:
        if rng.random() < 0.42:
            h = rng.uniform(36, 120)
            w = h * rng.uniform(0.22, 0.3)
            x += rng.uniform(-10, 10)
            for k in range(3):
                top = y - h + k * h * 0.22
                base = y - h * (0.45 - k * 0.2)
                ww = w * (0.55 + k * 0.28)
                d.polygon([(x, top), (x - ww, base), (x + ww, base)], fill=dark)
            d.rectangle((x - w * 0.08, y - h * 0.3, x + w * 0.08, y + 6), fill=dark)
    return layer


def main():
    rng = random.Random(1166)
    moon = (W * 0.66, H * 0.34, 250)
    img = sky(rng)
    stars(img, rng, moon)
    img = img.convert("RGBA")
    img.alpha_composite(moon_layer(rng, moon))
    img.alpha_composite(mist(rng))
    img.alpha_composite(ridge(rng))
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    img.convert("RGB").save(OUT, "WEBP", quality=86, method=6)
    print(f"wrote {os.path.relpath(OUT)}: {os.path.getsize(OUT) // 1000} KB")


if __name__ == "__main__":
    main()
