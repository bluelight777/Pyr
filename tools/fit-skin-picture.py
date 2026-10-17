#!/usr/bin/env python3
"""Fit the pictures made with Meshy to the skins they're for.

    python3 tools/fit-skin-picture.py <folder with the pictures from Meshy> [skin ...]

The skins in tools/skin-pictures.json have pictures made with Meshy's text-to-image (from the
prompt kept there, with the task it came from). This takes each one from the folder (by its
"source" name), mirrors it if asked, covers 2560x1440 with it (cut evenly from both sides of the
longer way), sharpens it a little after enlarging, sets its brightness ("exposure", in light,
not in pixel values) and rolls off its brightest light ("cap"), so messages stay easy to read on
top of it. It's written to skins/<id>/background.webp, as good as fits in 640 KB (a skin's file
can be 700 KB), keeping the picture's XMP: the record that it was made with AI (Meshy's terms
say that has to stay with it).

Needs numpy and Pillow with WebP.
"""

import json
import os
import sys

import numpy as np
from PIL import Image, ImageFilter

W, H = 2560, 1440
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, ".."))
BUDGET = 640_000


def to_linear(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def to_srgb(c):
    c = np.clip(c, 0.0, 1.0)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * np.power(c, 1 / 2.4) - 0.055)


def cover(im):
    """Scaled to cover W x H, the overflow cut evenly from both sides."""
    s = max(W / im.width, H / im.height)
    w, h = round(im.width * s), round(im.height * s)
    im = im.resize((w, h), Image.LANCZOS)
    x, y = (w - W) // 2, (h - H) // 2
    return im.crop((x, y, x + W, y + H))


def grade(im, exposure, cap):
    """Brightness in linear light, then a soft roll-off of the brightest light towards `cap`
    (keeping each color's hue: the three channels are scaled together)."""
    a = to_linear(np.asarray(im, np.float32) / 255.0) * exposure
    lum = a.max(axis=2, keepdims=True)
    knee = cap * 0.55
    over = np.maximum(lum - knee, 0.0)
    rolled = np.where(lum > knee, knee + (cap - knee) * (1 - np.exp(-over / (cap - knee))), lum)
    a = a * np.where(lum > 1e-6, rolled / np.maximum(lum, 1e-6), 1.0)
    out = to_srgb(a) * 255.0
    # A little dither, so enlarged dark skies don't band.
    rng = np.random.default_rng(11)
    out += (rng.random(out.shape[:2]) - rng.random(out.shape[:2]))[..., None] * 0.8
    return Image.fromarray(np.clip(np.round(out), 0, 255).astype(np.uint8), "RGB")


def fit(skin_id, spec, folder):
    src = os.path.join(folder, spec["source"])
    original = Image.open(src)
    xmp = original.info.get("xmp") or original.info.get("XML:com.adobe.xmp", "")
    if isinstance(xmp, str):
        xmp = xmp.encode("utf-8")
    im = original.convert("RGB")
    if spec.get("mirror"):
        im = im.transpose(Image.FLIP_LEFT_RIGHT)
    im = cover(im)
    im = im.filter(ImageFilter.UnsharpMask(radius=1.6, percent=45, threshold=2))
    im = grade(im, spec.get("exposure", 1.0), spec.get("cap", 1.0))
    out = os.path.join(ROOT, "skins", skin_id, "background.webp")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    q = 90
    while True:
        im.save(out, "WEBP", quality=q, method=6, xmp=xmp)
        size = os.path.getsize(out)
        if size <= BUDGET or q <= 50:
            break
        q -= 3
    print(f"{skin_id}: {spec['source']} -> {os.path.relpath(out, ROOT)}, {size // 1000} KB (quality {q})")


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    folder = sys.argv[1]
    specs = json.load(open(os.path.join(HERE, "skin-pictures.json")))
    want = sys.argv[2:] or list(specs)
    for skin_id in want:
        fit(skin_id, specs[skin_id], folder)


if __name__ == "__main__":
    main()
