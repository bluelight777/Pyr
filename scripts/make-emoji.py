#!/usr/bin/env python3
"""Builds Pyr's colour emoji assets from Twemoji (graphics CC-BY 4.0, by Twitter/X and
contributors) and Emojibase (names, groups and shortcodes, MIT).

  npm pack @twemoji/svg emojibase-data   # then unpack both
  python3 scripts/make-emoji.py <twemoji-svg-dir> <emojibase-data-dir> crates/app/assets/emoji

Writes atlas.png (every emoji in one picture, 48px cells), index.json (where each one is,
the picker's groups, names and shortcodes) and two tiny fonts the text layout uses to hold
the space an emoji picture is drawn into (blank.ttf: 1em wide; zero.ttf: no width).
"""
import io, json, os, sys
import cairosvg
from PIL import Image
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

svg_dir, eb_dir, out = sys.argv[1:4]
CELL, COLS = 48, 64
os.makedirs(out, exist_ok=True)

files = sorted(f[:-4] for f in os.listdir(svg_dir) if f.endswith(".svg"))
cells = {k: i for i, k in enumerate(files)}
rows = (len(files) + COLS - 1) // COLS
atlas = Image.new("RGBA", (COLS * CELL, rows * CELL))
for k, i in cells.items():
    png = cairosvg.svg2png(url=os.path.join(svg_dir, k + ".svg"), output_width=CELL, output_height=CELL)
    atlas.paste(Image.open(io.BytesIO(png)).convert("RGBA"), ((i % COLS) * CELL, (i // COLS) * CELL))
atlas.save(os.path.join(out, "atlas.png"), optimize=True)

def key(hexcode):
    """Twemoji's file name for a sequence: lowercase, '-' joined, without FE0F."""
    return "-".join(format(int(p, 16), "x") for p in hexcode.split("-") if p.upper() != "FE0F")

data = json.load(open(os.path.join(eb_dir, "en", "data.json")))
meta = json.load(open(os.path.join(eb_dir, "meta", "groups.json")))
sc = {}
for src in ["emojibase", "github", "iamcal"]:
    for h, v in json.load(open(os.path.join(eb_dir, "en", "shortcodes", src + ".json"))).items():
        for s in ([v] if isinstance(v, str) else v):
            sc.setdefault(h, [])
            if s not in sc[h]:
                sc[h].append(s)

GROUPS = [(0, "Smileys & Emotion"), (1, "People & Body"), (3, "Animals & Nature"), (4, "Food & Drink"),
          (6, "Activities"), (5, "Travel & Places"), (7, "Objects"), (8, "Symbols"), (9, "Flags")]
groups = {g: [] for g, _ in GROUPS}
text_default = []
for d in sorted(data, key=lambda d: d.get("order", 1 << 30)):
    k = key(d["hexcode"])
    if k not in cells or d.get("group") not in groups:
        continue
    if d.get("type") == 0 and "-" not in d["hexcode"]:
        text_default.append(k)
    item = {"e": d["emoji"], "n": d["label"], "k": k, "s": sc.get(d["hexcode"], [])}
    tones = {}
    for s in d.get("skins", []):
        t = s.get("tone")
        if isinstance(t, int) and key(s["hexcode"]) in cells:
            tones[t] = s
    if len(tones) == 5:
        item["t"] = [[tones[t]["emoji"], key(tones[t]["hexcode"])] for t in range(1, 6)]
    groups[d["group"]].append(item)
# Also text-presentation singles that only show as emoji with FE0F (from all of Emojibase).
index = {
    "cell": CELL, "cols": COLS, "files": files, "text_default": sorted(set(text_default)),
    "groups": [{"name": n, "items": groups[g]} for g, n in GROUPS],
}
json.dump(index, open(os.path.join(out, "index.json"), "w"), ensure_ascii=False, separators=(",", ":"))

# The two placeholder fonts: every code point found in an emoji, plus ASCII (keycaps, custom
# emoji text) and the replacement characters egui falls back to.
cps = {int(p, 16) for f in files for p in f.split("-")} | {0xFE0F} | set(range(0x20, 0x7F)) | {0x25FB, 0xFFFD}
for name, adv in [("blank", 1000), ("zero", 0)]:
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder([".notdef", "blank"])
    fb.setupCharacterMap({cp: "blank" for cp in sorted(cps)})
    pen = TTGlyphPen(None)
    empty = pen.glyph()
    fb.setupGlyf({".notdef": empty, "blank": empty})
    fb.setupHorizontalMetrics({".notdef": (adv, 0), "blank": (adv, 0)})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "Pyr Emoji " + name, "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
    fb.setupPost()
    fb.save(os.path.join(out, name + ".ttf"))
print(len(files), "emoji;", sum(len(g["items"]) for g in index["groups"]), "in the picker")
