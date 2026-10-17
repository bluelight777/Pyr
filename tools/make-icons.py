#!/usr/bin/env python3
"""Draw the Pyr icon (a campfire on a warm dark tile) and export every size we ship.

    python3 tools/make-icons.py

Writes into crates/app/assets/icon/:
    pyr.ico          Windows icon (16-256 px) for the .exe, shortcuts, tray and installer
    pyr-<N>.png      16-512 px for the Linux app menu (hicolor theme)
    pyr-128.rgba     raw RGBA the app uses as its window icon
plus installer/windows/welcome.bmp (the side picture of the Windows setup wizard) and
installer/site/pyr-icon.png (the download page's logo and favicon).
Needs Pillow 8.2 or newer (pip install pillow).

The scene is a three-quarter view from about 25 degrees above the ground, and the logs follow it:
  * Two logs cross in an X under the fire. Each tilts 17 degrees on screen, so on the ground it
    lies about 36 degrees off the screen's horizontal.
  * A log's cut end is a circle, so on screen it is an ellipse: the log's full width across, and
    squashed along the log by sin(36) * cos(25), about 0.53.
  * Each log's near end (lower on screen) points at the viewer and shows its cut face; the far
    end points away, so only its rounded bark edge shows.
  * The fire sits on the crossing: the far halves of the logs go behind the flame, the near
    halves and the crossing are in front, and the top-left-to-bottom-right log rests on the other.
  * The fire is the only light. The logs' tops face it and are lit; the sides we see and the
    undersides are darker; the ends dim with distance from it; the cut faces point away from it
    and sit in shade. The top log shades the bottom one where they cross, and the pile shades
    the ground under it.

The big sizes are scaled down from one drawing. 16, 20, 24 and 32 px (the tray and the
taskbar) are drawn again straight onto their own pixel grid, with the tile filling the square,
a bigger fire and fewer details, because a shrunken 512 px drawing smears every edge there.
"""
import math
import os
import re

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..")
OUT = os.path.join(ROOT, "crates", "app", "assets", "icon")

# Colours (the flame ones match crates/app/src/theme.rs).
TILE_TOP, TILE_BOTTOM = (0x2E, 0x24, 0x1F), (0x17, 0x12, 0x0F)
EMBER = (0xFF, 0x7A, 0x3D)
GOLD = (0xFF, 0xD2, 0x7A)
# Across a log, top edge to underside: where the fire catches it, the side we see, the underside.
LOG_SHADING = ((0.0, (0xE7, 0xA0, 0x61)), (0.22, (0xF0, 0xAA, 0x68)), (0.58, (0xB4, 0x74, 0x47)), (1.0, (0x6A, 0x3E, 0x26)))
BARK, WOOD, RING = (0x5A, 0x34, 0x20), (0xC9, 0x98, 0x68), (0x9A, 0x6A, 0x43)
SHADE = (0x12, 0x0A, 0x06)

# The flame, in a 100 x 140 box with its base centre at (50, 140): a main tongue that curls a
# touch left at the tip, a smaller tongue on the left shoulder, and a gold heart sitting low.
FLAME = (
    "M50 140 C76 140 96 121 96 96 C96 74 87 60 77 46 C68 33 61 19 60 0 "
    "C47 13 37 31 39 58 C31 53 26 45 25 33 C14 46 4 65 4 96 C4 121 24 140 50 140 Z"
)
CORE = (
    "M50 135 C63 135 72 125 72 112 C72 99 63 91 56 84 C51 79 48 73 47 66 "
    "C40 79 28 93 28 112 C28 125 37 135 50 135 Z"
)

# The fire on the 512 grid: flame base and height; where the logs cross; each log's half
# length (between its end centres) and radius; the cut-face squash; the tilt; and, along each
# log from the crossing, where it starts being in front of the flame (None: all of it).
FIRE = dict(base=366, height=300, cx=256, cy=384, half=124, r=26, squash=0.53, tilt=17, front_a=None, front_b=0)


def path_points(d, scale, dx, dy, steps=48):
    """Points along an SVG path made of M, C and Z (all this icon uses), scaled and moved."""
    nums = [float(v) for v in re.findall(r"-?\d+(?:\.\d+)?", d)]
    cmds = re.findall(r"[MCZ]", d)
    pts, i = [], 0
    cur = (0.0, 0.0)
    for c in cmds:
        if c == "M":
            cur = (nums[i], nums[i + 1])
            i += 2
            pts.append(cur)
        elif c == "C":
            p1, p2, p3 = (nums[i], nums[i + 1]), (nums[i + 2], nums[i + 3]), (nums[i + 4], nums[i + 5])
            i += 6
            for s in range(1, steps + 1):
                t = s / steps
                u = 1 - t
                x = u**3 * cur[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0]
                y = u**3 * cur[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]
                pts.append((x, y))
            cur = p3
    return [(x * scale + dx, y * scale + dy) for x, y in pts]


def flame(d, cx, base, height):
    """The flame (or its heart) with its base centre at (cx, base), `height` tall."""
    k = height / 140
    return path_points(d, k, cx - 50 * k, base - height)


def squircle(size, margin, n=4.6, steps=720):
    """A superellipse tile: softer corners than a rounded rectangle."""
    a, c = (size - 2 * margin) / 2, size / 2
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        ct, st = math.cos(t), math.sin(t)
        pts.append((c + a * math.copysign(abs(ct) ** (2 / n), ct), c + a * math.copysign(abs(st) ** (2 / n), st)))
    return pts


def vertical_gradient(size, top, bottom, y0, y1):
    im = Image.new("RGBA", (size, size))
    d = ImageDraw.Draw(im)
    for y in range(size):
        t = min(max((y - y0) / (y1 - y0), 0.0), 1.0)
        d.line([(0, y), (size, y)], fill=tuple(int(round(a + (b - a) * t)) for a, b in zip(top, bottom)) + (255,))
    return im


def tinted(colour, alpha):
    """A layer of one colour, as see-through as `alpha` says."""
    layer = Image.new("RGBA", alpha.size, colour + (0,))
    layer.putalpha(alpha)
    return layer


def ember_glow(size, cx, cy, r, opacity, mask):
    """Ember light fading out from (cx, cy): full `opacity` at the centre, 35% of it at 55% of
    the radius, gone at the edge. Clipped to `mask` (the tile)."""
    ramp = Image.radial_gradient("L").resize((int(2 * r), int(2 * r)), Image.BILINEAR)

    def alpha(v):
        t = v / 255 * math.sqrt(2)  # Pillow's ramp reaches 255 at the corner, not the edge
        a = opacity * (1 - t / 0.55 * 0.65) if t < 0.55 else opacity * 0.35 * (1 - (t - 0.55) / 0.45)
        return int(round(max(a, 0.0) * 255))

    a = Image.new("L", (size, size), 0)
    a.paste(ramp.point(alpha), (int(cx - r), int(cy - r)))
    return tinted(EMBER, ImageChops.multiply(a, mask))


def across_log(t):
    """The log's colour at `t` of the way from its top edge (0) to its underside (1)."""
    t = min(max(t, 0.0), 1.0)
    for (t0, c0), (t1, c1) in zip(LOG_SHADING, LOG_SHADING[1:]):
        if t <= t1:
            return lerp(c0, c1, (t - t0) / (t1 - t0))
    return LOG_SHADING[-1][1]


def log_outline(half, r, kr, steps=48):
    """A log in its own frame (x toward its near end, y down): a band with an elliptical end
    at each side, kr wide along the log and r tall."""
    near = [(half + kr * math.cos(a), r * math.sin(a)) for a in (-math.pi / 2 + math.pi * i / steps for i in range(steps + 1))]
    far = [(-half + kr * math.cos(a), r * math.sin(a)) for a in (math.pi / 2 + math.pi * i / steps for i in range(steps + 1))]
    return near + far


def draw_log(S, u, f, mirror, split, detail):
    """One log drawn in its own frame, then turned into place on the S x S canvas (u pixels per
    grid unit). `mirror` gives the log whose near end is at the lower left. Returns the painted
    layer, the log's outline mask, and the mask of its part from `split` (along the log, from
    the crossing) to the near end, which stays in front of the fire."""
    half, r = f["half"], f["r"]
    kr = f["squash"] * r
    ox, oy = (half + kr + 2) * u, (r + 2) * u  # the crossing, on the log's own canvas
    w, h = int(2 * ox) + 1, int(2 * oy) + 1
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).polygon([(ox + x * u, oy + y * u) for x, y in log_outline(half, r, kr)], fill=255)

    # Lit top fading through the side we see to the dark underside (painted edge to edge, so
    # turning the layer never drags in black from outside the log).
    paint = Image.new("RGBA", (w, h))
    d = ImageDraw.Draw(paint)
    for row in range(h):
        d.line([(0, row), (w, row)], fill=across_log(((row + 0.5 - oy) / u + r) / (2 * r)) + (255,))
    if detail["falloff"]:
        # Dimmer with distance from the fire: a little at the far end, more at the near end,
        # whose side also turns away from the flame.
        ramp = Image.new("L", (w, 1))
        for x in range(w):
            t = ((x - ox) / u + half + kr) / (2 * (half + kr))
            a = 0.24 * (0.36 - t) / 0.36 if t < 0.36 else (0.30 * (t - 0.56) / 0.44 if t > 0.56 else 0.0)
            ramp.putpixel((x, 0), int(round(min(max(a, 0.0), 0.3) * 255)))
        paint = Image.alpha_composite(paint, tinted(SHADE, ramp.resize((w, h))))
    # The cut face at the near end: a bark rim, pale wood and a growth ring.
    d = ImageDraw.Draw(paint)
    fx, fy = ox + half * u, oy

    def ellipse(cx, cy, rx, ry, **kw):
        d.ellipse([cx - rx, cy - ry, cx + rx, cy + ry], **kw)

    ellipse(fx, fy, kr * u, r * u, fill=BARK)
    ellipse(fx + 0.6 * u, fy + 0.8 * u, kr * 0.80 * u, r * 0.84 * u, fill=WOOD)
    if detail["rings"]:
        ring_w = 2.8 * u
        ellipse(fx + 0.6 * u, fy + 0.8 * u, kr * 0.42 * u + ring_w / 2, r * 0.44 * u + ring_w / 2, outline=RING, width=max(1, round(ring_w)))
    paint.putalpha(mask)
    front = mask.copy()
    if split is not None:
        ImageDraw.Draw(front).rectangle([0, 0, ox + split * u, h], fill=0)

    # Turn it into place: canvas pixel P comes from the log's own pixel
    # (ox + (P - C).along, oy + (P - C).down), where C is the crossing.
    a = math.radians(f["tilt"])
    ux, uy = (-math.cos(a) if mirror else math.cos(a)), math.sin(a)
    dx, dy = (math.sin(a) if mirror else -math.sin(a)), math.cos(a)
    cx, cy = f["cx"] * u, f["cy"] * u
    coeffs = (ux, uy, ox - cx * ux - cy * uy, dx, dy, oy - cx * dx - cy * dy)
    return [im.transform((S, S), Image.AFFINE, coeffs, resample=Image.BICUBIC) for im in (paint, mask, front)]


def draw_campfire(S, grid, tile, fire, glow, detail):
    """The icon on an S x S canvas, `grid` units wide. tile: ("squircle", margin) or
    ("rounded", corner radius); glow: (x, y, radius, opacity) or None; all in grid units."""
    u = S / grid
    f = fire
    k = f["r"] / FIRE["r"]  # shadows scale with the logs
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    mask = Image.new("L", (S, S), 0)
    if tile[0] == "squircle":
        ImageDraw.Draw(mask).polygon([(x * u, y * u) for x, y in squircle(grid, tile[1])], fill=255)
        img.paste(vertical_gradient(S, TILE_TOP, TILE_BOTTOM, tile[1] * u, S - tile[1] * u), (0, 0), mask)
    else:
        ImageDraw.Draw(mask).rounded_rectangle([0, 0, S - 1, S - 1], radius=tile[1] * u, fill=255)
        img.paste(vertical_gradient(S, TILE_TOP, TILE_BOTTOM, 0, S), (0, 0), mask)
    if glow:
        img = Image.alpha_composite(img, ember_glow(S, glow[0] * u, glow[1] * u, glow[2] * u, glow[3], mask))
    if detail["ground"]:
        g = Image.new("L", (S, S), 0)
        gx, gy, rx, ry = f["cx"] * u, (f["cy"] + 48 * k) * u, 136 * k * u, 20 * k * u
        ImageDraw.Draw(g).ellipse([gx - rx, gy - ry, gx + rx, gy + ry], fill=128)
        img = Image.alpha_composite(img, tinted((0x08, 0x05, 0x04), g.filter(ImageFilter.GaussianBlur(8 * k * u))))

    # B (near end at the lower left) lies underneath; A (near end at the lower right) rests on it.
    log_b, body_b, front_b = draw_log(S, u, f, True, f["front_b"], detail)
    log_a, body_a, front_a = draw_log(S, u, f, False, f["front_a"], detail)
    img = Image.alpha_composite(img, log_b)
    if detail["contact"]:
        # A's soft shadow on B where they cross (the light is above)
        sh = ImageChops.offset(body_a, 0, round(7 * k * u)).filter(ImageFilter.GaussianBlur(5 * k * u))
        img = Image.alpha_composite(img, tinted((0x0B, 0x06, 0x04), ImageChops.multiply(sh.point(lambda v: int(v * 0.7)), body_b)))
    img = Image.alpha_composite(img, log_a)

    # The fire sits on top of the logs, in front of them.
    fl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(fl)
    d.polygon([(x * u, y * u) for x, y in flame(FLAME, f["cx"], f["base"], f["height"])], fill=EMBER)
    d.polygon([(x * u, y * u) for x, y in flame(CORE, f["cx"], f["base"], f["height"])], fill=GOLD)
    return Image.alpha_composite(img, fl)


def draw_icon(S=2048):
    """The full icon on the 512 grid at S pixels (scale down for every big size)."""
    return draw_campfire(S, 512, ("squircle", 20), FIRE, (256, 270, 230, 0.55), dict(ground=True, contact=True, rings=True, falloff=True))


# Small sizes: the fire scaled up to fill a full-bleed tile, the logs a touch thicker at the
# smallest sizes, and only the details that survive there.
SMALL = {
    16: dict(radius=3.5, fill=0.94, thicken=1.18, glow=0.0, detail=dict(ground=False, contact=False, rings=False, falloff=False)),
    20: dict(radius=4.5, fill=0.93, thicken=1.12, glow=0.25, detail=dict(ground=False, contact=False, rings=False, falloff=True)),
    24: dict(radius=5.5, fill=0.92, thicken=1.06, glow=0.35, detail=dict(ground=True, contact=False, rings=False, falloff=True)),
    32: dict(radius=7.0, fill=0.92, thicken=1.0, glow=0.45, detail=dict(ground=True, contact=True, rings=False, falloff=True)),
}
FIRE_TOP, FIRE_BOTTOM = 66, 455  # the big fire's extent on the 512 grid, flame tip to log ends


def draw_small(n, ss=16):
    s = SMALL[n]
    k = s["fill"] * n / (FIRE_BOTTOM - FIRE_TOP)
    top = (n - (FIRE_BOTTOM - FIRE_TOP) * k) / 2
    f = dict(FIRE, cx=n / 2, base=top + (FIRE["base"] - FIRE_TOP) * k, cy=top + (FIRE["cy"] - FIRE_TOP) * k)
    for key in ("height", "half"):
        f[key] = FIRE[key] * k
    for key in ("front_a", "front_b"):
        f[key] = None if FIRE[key] is None else FIRE[key] * k
    f["r"] = FIRE["r"] * k * s["thicken"]
    glow = (n / 2, n * 0.55, n * 0.48, s["glow"]) if s["glow"] else None
    return draw_campfire(n * ss, n, ("rounded", s["radius"]), f, glow, s["detail"]).resize((n, n), Image.BOX)


def lerp(a, b, t):
    return tuple(int(round(x + (y - x) * t)) for x, y in zip(a, b))


def welcome_bitmap(icon):
    """164x314 picture for the left side of the setup wizard's first and last pages."""
    w, h = 164, 314
    img = Image.new("RGB", (w, h))
    d = ImageDraw.Draw(img)
    top, bottom = (0x2e, 0x24, 0x1e), (0x14, 0x10, 0x0e)
    for y in range(h):
        d.line([(0, y), (w, y)], fill=lerp(top, bottom, y / h))
    glow = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse([w / 2 - 70, 70, w / 2 + 70, 210], fill=(0xff, 0x7a, 0x3d, 90))
    glow = glow.filter(ImageFilter.GaussianBlur(22))
    img = Image.alpha_composite(img.convert("RGBA"), glow)
    mark = icon.resize((112, 112), Image.LANCZOS)
    img.alpha_composite(mark, (int(w / 2 - 56), 84))
    font_path = os.path.join(HERE, "..", "crates", "app", "assets", "fonts", "Inter-SemiBold.otf")
    try:
        font = ImageFont.truetype(font_path, 30)
        small = ImageFont.truetype(font_path, 12)
    except OSError:
        font = small = ImageFont.load_default()
    d = ImageDraw.Draw(img)
    d.text((w / 2, 226), "Pyr", font=font, fill=(0xf4, 0xe9, 0xdf), anchor="mm")
    d.text((w / 2, 254), "gather round", font=small, fill=(0xc9, 0x9a, 0x78), anchor="mm")
    out = os.path.join(HERE, "..", "installer", "windows", "welcome.bmp")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    img.convert("RGB").save(out)


def main():
    os.makedirs(OUT, exist_ok=True)
    big = draw_icon()
    frames = {n: draw_small(n) for n in SMALL}
    for n in (40, 48, 64, 128, 256, 512):
        frames[n] = big.resize((n, n), Image.LANCZOS)
    for n in (16, 32, 48, 64, 128, 256, 512):
        frames[n].save(os.path.join(OUT, f"pyr-{n}.png"))
    ico = [frames[n] for n in (16, 20, 24, 32, 40, 48, 64, 128)]
    frames[256].save(os.path.join(OUT, "pyr.ico"), sizes=[(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)], append_images=ico)
    with open(os.path.join(OUT, "pyr-128.rgba"), "wb") as f:
        f.write(frames[128].tobytes())
    welcome_bitmap(big)
    frames[256].save(os.path.join(ROOT, "installer", "site", "pyr-icon.png"))
    print("icons written to", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
