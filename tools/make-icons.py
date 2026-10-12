#!/usr/bin/env python3
"""Draw the Ingle icon (a campfire on a warm dark tile) and export every size we ship.

    python3 tools/make-icons.py

Writes into crates/app/assets/icon/:
    ingle.ico          Windows icon (16-256 px) for the .exe, shortcuts and installer
    ingle-<N>.png      16-512 px for the Linux app menu (hicolor theme)
    ingle-128.rgba     raw RGBA the app uses as its window icon
and installer/windows/welcome.bmp (the side picture of the Windows setup wizard).
Needs Pillow (pip install pillow).
"""
import math
import os

from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "crates", "app", "assets", "icon")
S = 1024  # draw big, then scale down for smooth edges


def lerp(a, b, t):
    return tuple(int(round(x + (y - x) * t)) for x, y in zip(a, b))


def teardrop(cx, cy, w, h, m=1.8, lean=0.0, n=400):
    """A flame-shaped drop: round at the bottom, pointed at the top."""
    pts = []
    for i in range(n):
        t = 2 * math.pi * i / n
        x = math.sin(t) * math.sin(t / 2) ** m
        y = math.cos(t)  # +1 = tip
        # lean the tip a little, like a flame in a breeze
        x += lean * max(y, 0) ** 2
        pts.append((cx + x * w, cy - y * h))
    return pts


def draw_icon():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # Tile: a rounded square with a warm vertical gradient.
    tile = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    grad = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(grad)
    top, bottom = (0x30, 0x25, 0x1f, 255), (0x1a, 0x14, 0x11, 255)
    for y in range(S):
        gd.line([(0, y), (S, y)], fill=lerp(top, bottom, y / S))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([40, 40, S - 40, S - 40], radius=220, fill=255)
    tile.paste(grad, (0, 0), mask)
    img = Image.alpha_composite(img, tile)

    # Warm glow behind the fire.
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse([S * 0.18, S * 0.22, S * 0.82, S * 0.86], fill=(0xff, 0x7a, 0x3d, 110))
    glow = glow.filter(ImageFilter.GaussianBlur(S * 0.07))
    glow_masked = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    glow_masked.paste(glow, (0, 0), mask)
    img = Image.alpha_composite(img, glow_masked)

    d = ImageDraw.Draw(img)
    # Logs: two crossed bars with light end grain.
    log = (0x8a, 0x5a, 0x3c, 255)
    end = (0xd0, 0x9a, 0x6a, 255)
    for sign in (-1, 1):
        a = math.radians(14 * sign)
        cx, cy, half, r = S / 2, S * 0.775, S * 0.27, S * 0.052
        dx, dy = math.cos(a) * half, math.sin(a) * half
        x1, y1, x2, y2 = cx - dx, cy - dy, cx + dx, cy + dy
        nx, ny = -math.sin(a) * r, math.cos(a) * r
        d.polygon([(x1 + nx, y1 + ny), (x2 + nx, y2 + ny), (x2 - nx, y2 - ny), (x1 - nx, y1 - ny)], fill=log)
        ex, ey = (x2, y2) if sign > 0 else (x1, y1)
        d.ellipse([ex - r, ey - r, ex + r, ey + r], fill=end)

    # Flames: outer ember, middle orange, inner gold.
    d.polygon(teardrop(S * 0.5, S * 0.47, S * 0.235, S * 0.30, m=1.6, lean=0.10), fill=(0xff, 0x6a, 0x2e, 255))
    d.polygon(teardrop(S * 0.5, S * 0.535, S * 0.165, S * 0.215, m=1.7, lean=-0.06), fill=(0xff, 0x9a, 0x45, 255))
    d.polygon(teardrop(S * 0.5, S * 0.60, S * 0.095, S * 0.13, m=1.8, lean=0.05), fill=(0xff, 0xdc, 0x8a, 255))
    return img


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
    d.text((w / 2, 226), "Ingle", font=font, fill=(0xf4, 0xe9, 0xdf), anchor="mm")
    d.text((w / 2, 254), "gather round", font=small, fill=(0xc9, 0x9a, 0x78), anchor="mm")
    out = os.path.join(HERE, "..", "installer", "windows", "welcome.bmp")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    img.convert("RGB").save(out)


def main():
    os.makedirs(OUT, exist_ok=True)
    big = draw_icon()
    for n in (16, 24, 32, 48, 64, 128, 256, 512):
        im = big.resize((n, n), Image.LANCZOS)
        if n in (16, 32, 48, 64, 128, 256, 512):
            im.save(os.path.join(OUT, f"ingle-{n}.png"))
    big.resize((256, 256), Image.LANCZOS).save(
        os.path.join(OUT, "ingle.ico"), sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    )
    with open(os.path.join(OUT, "ingle-128.rgba"), "wb") as f:
        f.write(big.resize((128, 128), Image.LANCZOS).tobytes())
    welcome_bitmap(big)
    print("icons written to", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
