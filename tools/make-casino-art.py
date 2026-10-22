#!/usr/bin/env python3
"""Makes Pyr Casino's pictures (crates/app/assets/casino) from the Meshy renders.

  python3 tools/make-casino-art.py <renders> crates/app/assets/casino

<renders> holds the renders as Meshy made them (text to image, nano-banana-pro), in three
folders: concepts/ (the first eight), set/ (the floor, tiles, tables and Ember Dragon) and
reels/ (Reel Row's twelve machines: a tile and a sheet of nine symbols each).

Tiles and backdrops are cropped to 16:9 and made smaller; a sheet of symbols is cut into its
nine cells, each cell's black background taken out (what's dark and joined to the cell's edge;
glows in it stay, faded) and the nine packed into one 3 x 3 picture. Everything is WebP.
"""
import os
import sys

import numpy as np
from PIL import Image
from scipy import ndimage

SRC, OUT = sys.argv[1], sys.argv[2]
os.makedirs(OUT, exist_ok=True)


def load(path):
    return Image.open(os.path.join(SRC, path)).convert("RGB")


def save(im, name, quality=82):
    im.save(os.path.join(OUT, name), "WEBP", quality=quality, method=6)
    print(f"{name}: {im.size[0]}x{im.size[1]}, {os.path.getsize(os.path.join(OUT, name)) // 1024} KB")


def cover(im, w, h, box=None):
    """`im` (or the `box` of it) cut to w:h about its middle, then made w x h."""
    if box:
        im = im.crop(box)
    sw, sh = im.size
    want = w / h
    if sw / sh > want:
        nw = round(sh * want)
        x = (sw - nw) // 2
        im = im.crop((x, 0, x + nw, sh))
    else:
        nh = round(sw / want)
        y = (sh - nh) // 2
        im = im.crop((0, y, sw, y + nh))
    return im.resize((w, h), Image.LANCZOS)


def content_box(im, thresh=14):
    """Where the picture is, inside any black border round it (some renders came framed)."""
    a = np.asarray(im).max(axis=2)
    rows = np.where((a > thresh).mean(axis=1) > 0.25)[0]
    cols = np.where((a > thresh).mean(axis=0) > 0.25)[0]
    if len(rows) == 0 or len(cols) == 0:
        return None
    return (int(cols[0]), int(rows[0]), int(cols[-1]) + 1, int(rows[-1]) + 1)


def key_out(im, thresh=30):
    """Its black background taken out: dark and joined to the edge is background (its glow
    kept as see-through color), the rest stays as it is."""
    a = np.asarray(im).astype(np.float32)
    lum = a.max(axis=2)
    lab, _ = ndimage.label(lum < thresh)
    edge = set(np.unique(np.concatenate([lab[0], lab[-1], lab[:, 0], lab[:, -1]]))) - {0}
    bg = np.isin(lab, list(edge))
    # What counts as no light at all: the background's own murk at the picture's edges (some
    # renders sit each symbol on a dark panel, not black), so it goes wholly see-through.
    ring = np.concatenate([lum[:4].ravel(), lum[-4:].ravel(), lum[:, :4].ravel(), lum[:, -4:].ravel()])
    floor = min(max(6.0, float(np.percentile(ring, 95)) + 3.0), float(thresh))
    glow = np.clip((lum - floor) / 70.0, 0.0, 1.0)
    alpha = np.where(bg, glow, 1.0)
    # A pixel's color over black was color x alpha: take the black back out of the glow.
    safe = np.maximum(alpha, 0.02)[..., None]
    rgb = np.where(bg[..., None], np.clip(a / safe, 0, 255), a)
    out = np.dstack([rgb, alpha * 255.0]).astype(np.uint8)
    return Image.fromarray(out, "RGBA")


def sheet(path, name, cell=192, inset=0.06, thresh=30, keep=False):
    """A 3 x 3 sheet of symbols, each cell cut out (and its background taken out unless `keep`),
    packed into one picture of 3 x 3 cells."""
    im = load(path)
    w, h = im.size
    out = Image.new("RGBA", (cell * 3, cell * 3), (0, 0, 0, 0))
    for i in range(9):
        cx, cy = i % 3, i // 3
        x0, y0 = cx * w / 3, cy * h / 3
        m = inset * w / 3
        c = im.crop((round(x0 + m), round(y0 + m), round(x0 + w / 3 - m), round(y0 + h / 3 - m)))
        c = c.convert("RGBA") if keep else key_out(c, thresh)
        c = c.resize((cell, cell), Image.LANCZOS)
        out.alpha_composite(c, (cx * cell, cy * cell))
    save(out, name, quality=88)


# The floor, Ember Dragon's lair and the tables: backdrops.
save(cover(load("set/lobby2.png"), 1376, 774), "floor.webp")
save(cover(load("set/dragon_lair.png"), 1376, 774), "dragon-lair.webp")
save(cover(load("concepts/poker_table.png"), 1376, 774), "table-poker.webp", quality=86)
save(cover(load("concepts/bj_table.png"), 1376, 774), "table-blackjack.webp", quality=86)
save(cover(load("set/baccarat_table.png"), 1376, 774), "table-baccarat.webp", quality=86)
# Green felt (its middle: the render has a lighter edge round it).
felt = load("set/felt_green.png")
fw, fh = felt.size
save(felt.crop((int(fw * 0.2), int(fh * 0.2), int(fw * 0.8), int(fh * 0.8))).resize((512, 512), Image.LANCZOS), "felt-green.webp", quality=80)
# The card back and a chip (grey, colored as it's drawn).
save(load("concepts/card_back.png").resize((300, 400), Image.LANCZOS), "card-back.webp", quality=88)
chip = load("set/chip.png").convert("L").convert("RGB")
save(key_out(chip.resize((256, 256), Image.LANCZOS), thresh=40), "chip.webp", quality=90)

# The floor's tiles.
TILES = {
    "tile-sevens": "concepts/tile_sevens.png",
    "tile-blackjack": "concepts/tile_blackjack.png",
    "tile-dragon": "concepts/tile_dragon.png",
    "tile-roulette": "set/tile_roulette.png",
    "tile-baccarat": "set/tile_baccarat.png",
    "tile-craps": "set/tile_craps.png",
    "tile-dice": "set/tile_dice.png",
    "tile-keno": "set/tile_keno.png",
    "tile-phoenix": "set/tile_phoenix.png",
    "tile-tables": "set/tile_tables.png",
    "tile-timebombs": "set/tile_timebombs.png",
    "tile-tourney": "set/tile_tourney.png",
}
for name, path in TILES.items():
    save(cover(load(path), 640, 360), f"{name}.webp", quality=80)
# (Pyr Friday Night's render has words in its lower right: cut above them.)
save(cover(load("set/tile_friday.png"), 640, 360, box=(0, 0, 1376, 676)), "tile-friday.webp", quality=80)

# Pyr Sevens and Ember Dragon: their symbols.
sheet("set/sevens_symbols.png", "sevens-symbols.webp")
sheet("set/dragon_symbols.png", "dragon-symbols.webp")
# The dragon as it ages (egg, hatchling, young, adult, elder).
for i in range(1, 6):
    save(key_out(load(f"set/dragon_age{i}.png").resize((512, 512), Image.LANCZOS), thresh=26), f"dragon-age{i}.webp", quality=86)

# Reel Row's machines: a tile each, and their symbols.
MACHINES = ["scoville", "deepsea", "cosmic", "sugar", "goldrush", "neon", "tomb", "frost", "cauldron", "hoard", "grind", "cabaret"]
# (Two renders came as a picture in a frame, the frame not black: their pictures, by hand.)
FRAMED = {"goldrush": (300, 110, 1090, 600), "neon": (300, 170, 1080, 594)}
for m in MACHINES:
    tile = load(f"reels/rr_{m}_tile.png")
    save(cover(tile, 640, 360, box=FRAMED.get(m) or content_box(tile)), f"reels-{m}-tile.webp", quality=80)
    # (Tomb of Ra's symbols are on gold plaques: kept as they are. Two sheets came with white
    # lines between and round their cells: cut further in.)
    inset = {"tomb": 0.07, "deepsea": 0.1, "hoard": 0.1}.get(m, 0.06)
    sheet(f"reels/rr_{m}_symbols.png", f"reels-{m}-symbols.webp", keep=(m == "tomb"), inset=inset)

# The cartoon hosts of the funny machines: each render has the host twice (pleased, then
# dismayed); each half has its background taken out and is set on a square of its own, the two
# side by side.
def host(path, name, side=384, thresh=24):
    im = load(path)
    w, h = im.size
    out = Image.new("RGBA", (side * 2, side), (0, 0, 0, 0))
    for k in range(2):
        half = key_out(im.crop((k * w // 2, 0, (k + 1) * w // 2, h)), thresh=thresh)
        box = half.getchannel("A").point(lambda a: 255 if a > 40 else 0).getbbox()
        if box:
            pad = 12
            box = (max(0, box[0] - pad), max(0, box[1] - pad), min(half.width, box[2] + pad), min(half.height, box[3] + pad))
            half = half.crop(box)
        half.thumbnail((side, side), Image.LANCZOS)
        out.alpha_composite(half, (k * side + (side - half.width) // 2, side - half.height))
    save(out, name, quality=88)


for m in ["scoville", "sugar", "grind", "frost"]:
    # (The tooth shines: a wider dark to take out round it.)
    host(f"hosts/host_{m}.png", f"host-{m}.webp", thresh=60 if m == "sugar" else 24)
