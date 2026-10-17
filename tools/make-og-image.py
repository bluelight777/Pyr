#!/usr/bin/env python3
"""Draws the website's link-preview picture, installer/site/assets/img/og.jpg (1200x630, what
chat apps and social sites show for a pyr.gg link), from tools/og-image.html, which puts the
site's chat screenshot (assets/img/chat.webp) beside Pyr's name. Run it after that screenshot
changes:

    python3 tools/make-og-image.py

Needs Playwright for Python with its Chromium, and Pillow."""
import functools
import http.server
import os
import tempfile
import threading

from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SITE = os.path.join(ROOT, "installer", "site")
TEMPLATE = os.path.join(ROOT, "tools", "og-image.html")
OUT = os.path.join(SITE, "assets", "img", "og.jpg")


class Site(http.server.SimpleHTTPRequestHandler):
    """The site's folder, plus the template at /og-image.html (so its pictures and fonts load)."""

    def log_message(self, *args):
        pass

    def translate_path(self, path):
        if path.split("?")[0] == "/og-image.html":
            return TEMPLATE
        return super().translate_path(path)


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Site, directory=SITE))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory() as tmp:
            png = os.path.join(tmp, "og.png")
            with sync_playwright() as p:
                browser = p.chromium.launch()
                page = browser.new_page(viewport={"width": 1200, "height": 630}, device_scale_factor=1)
                page.goto(f"http://127.0.0.1:{server.server_address[1]}/og-image.html")
                page.wait_for_load_state("networkidle")
                page.evaluate("document.fonts.ready")
                page.wait_for_timeout(500)
                page.screenshot(path=png)
                browser.close()
            Image.open(png).convert("RGB").save(OUT, quality=86, optimize=True, progressive=True)
    finally:
        server.shutdown()
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
