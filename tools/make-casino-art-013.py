#!/usr/bin/env python3
"""Makes Pyr Casino's pictures from 0.13.0 (crates/app/assets/casino) from the Meshy renders.

  python3 tools/make-casino-art-013.py <renders>/v13 crates/app/assets/casino

<renders>/v13 holds, for each of Reel Row's twelve machines (text to image, nano-banana-pro):
<m>_back.png (the room its reels stand in), <m>_bonus.png (its mini game's scene),
<m>_pieces.png (a 3 x 3 sheet of the mini game's pieces) and <m>_features.png (a 3 x 2 sheet of
Play Feature's six badges: Bonus Chance, Wild Spin, Bonus, Super Bonus, Mystery and All or
nothing); and sevens_features.png and dragon_features.png for Pyr Sevens and Ember Dragon.

Backdrops and scenes are cut to 16:9; sheets are cut into cells, each cell's background taken out
(black; The Daily Grind's badges came on white), and packed into one picture. Everything is WebP.
"""
import os
import sys

import numpy as np
from PIL import Image
from scipy import ndimage

SRC, OUT = sys.argv[1], sys.argv[2]
os.makedirs(OUT, exist_ok=True)
MACHINES = ["scoville", "deepsea", "cosmic", "sugar", "goldrush", "neon", "tomb", "frost", "cauldron", "hoard", "grind", "cabaret"]


def load(name):
    return Image.open(os.path.join(SRC, name)).convert("RGB")


def save(im, name, quality=82):
    im.save(os.path.join(OUT, name), "WEBP", quality=quality, method=6)
    print(f"{name}: {im.size[0]}x{im.size[1]}, {os.path.getsize(os.path.join(OUT, name)) // 1024} KB")


def cover(im, w, h):
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


def key_out(im, thresh=30, white=False):
    """Its background taken out: dark (or, `white`, light) and joined to the edge is background,
    its glow kept as see-through color; the rest stays as it is."""
    a = np.asarray(im).astype(np.float32)
    if white:
        a = 255.0 - a
    lum = a.max(axis=2) if not white else a.max(axis=2)
    lab, _ = ndimage.label(lum < thresh)
    edge = set(np.unique(np.concatenate([lab[0], lab[-1], lab[:, 0], lab[:, -1]]))) - {0}
    bg = np.isin(lab, list(edge))
    ring = np.concatenate([lum[:4].ravel(), lum[-4:].ravel(), lum[:, :4].ravel(), lum[:, -4:].ravel()])
    floor = min(max(6.0, float(np.percentile(ring, 95)) + 3.0), float(thresh))
    glow = np.clip((lum - floor) / 70.0, 0.0, 1.0)
    alpha = np.where(bg, glow, 1.0)
    if white:
        # (On white, what's background is simply gone: a soft edge, no glow to keep.)
        rgb = 255.0 - a
        alpha = np.where(bg, np.clip((lum - floor) / 30.0, 0.0, 1.0), 1.0)
    else:
        safe = np.maximum(alpha, 0.02)[..., None]
        rgb = np.where(bg[..., None], np.clip(a / safe, 0, 255), a)
    out = np.dstack([rgb, alpha * 255.0]).astype(np.uint8)
    return Image.fromarray(out, "RGBA")


def sheet(name, out_name, cols, rows, cell=192, inset=0.05, thresh=30, white=False):
    """A sheet of `cols` x `rows` pictures, each cut out (its background taken out), packed into one
    picture of `cols` x `rows` square cells."""
    im = load(name)
    w, h = im.size
    out = Image.new("RGBA", (cell * cols, cell * rows), (0, 0, 0, 0))
    cw, ch = w / cols, h / rows
    for i in range(cols * rows):
        cx, cy = i % cols, i // cols
        x0, y0 = cx * cw, cy * ch
        # A square about the cell's middle.
        side = min(cw, ch) * (1 - 2 * inset)
        mx, my = x0 + cw / 2, y0 + ch / 2
        c = im.crop((round(mx - side / 2), round(my - side / 2), round(mx + side / 2), round(my + side / 2)))
        c = key_out(c, thresh, white)
        c = c.resize((cell, cell), Image.LANCZOS)
        out.alpha_composite(c, (cx * cell, cy * cell))
    save(out, out_name, quality=88)


for m in MACHINES:
    # The room behind the reels, and the mini game's scene (both dimmed and lit over in the app).
    save(cover(load(f"{m}_back.png"), 1376, 774), f"rr-{m}-back.webp", quality=74)
    save(cover(load(f"{m}_bonus.png"), 1376, 774), f"rr-{m}-bonus.webp", quality=76)
    sheet(f"{m}_pieces.png", f"rr-{m}-pieces.webp", 3, 3, inset=0.04)
    sheet(f"{m}_features.png", f"rr-{m}-features.webp", 3, 2, cell=160, inset=0.03, white=(m == "grind"), thresh=(40 if m == "grind" else 30))
for g in ["sevens", "dragon"]:
    sheet(f"{g}_features.png", f"{g}-features.webp", 3, 2, cell=160, inset=0.03)
