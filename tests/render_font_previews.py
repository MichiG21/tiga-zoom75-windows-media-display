"""Render preview frames for the font fallback into previews/font-fallback/.

    py tests/render_font_previews.py

Offline: it calls render_media() and saves PNGs, and writes nothing else. It never
touches extracted/ (the live uploader watches the stream file there) or Bluetooth.
"""
from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_renderer import REPO, load_renderer  # noqa: E402

sys.path.insert(0, str(REPO))  # for tiga_dlx

OUT = REPO / "previews" / "font-fallback"

CASES = {
    # Read from tiga-fresh-test\logs\renderer.log, 2026-10-15
    "morse-band-logged": {"title": "･－･･ ･ － ･･ － －－･ －－－", "artist": "･ ･－･ ･－ ･･･ ･ －･･",
                          "album": "･－･･ ･ － ･･ － －－･ －－－", "duration_ms": 214000},
    "takanaka-logged": {"title": "トーキョーレギー", "artist": "Masayoshi Takanaka", "album": "SEYCHELLES",
                        "duration_ms": 263000},
    "morse-like-symbols": {"title": "· • ∙ − – — ▬ ⸺", "artist": "·−·· · −   ·· −   −−· −−−",
                           "album": "U+00B7 2022 2219 2212 2013 2014 25AC 2E3A"},
    "japanese": {"title": "渚・モデラート", "artist": "高中正義", "album": "JOLLY JIVE", "duration_ms": 290000},
    "chinese": {"title": "月亮代表我的心", "artist": "鄧麗君", "album": "島國之情歌第三集", "duration_ms": 209000},
    "korean": {"title": "밤양갱", "artist": "비비 (BIBI)", "album": "밤양갱", "duration_ms": 146000},
    "mixed-latin-japanese": {"title": "高中正義 — Blue Lagoon", "artist": "Masayoshi Takanaka",
                             "album": "An Insatiable High", "duration_ms": 255000},
    "long-japanese-no-spaces": {"title": "ラブ・ストーリーは突然に東京ラブストーリー主題歌シングルバージョン",
                                "artist": "小田和正", "album": "Oh! Yeah!", "duration_ms": 274000},
    "latin-only": {"title": "Blue Lagoon", "artist": "Masayoshi Takanaka", "album": "Seychelles",
                   "duration_ms": 245000},
}


def main():
    r = load_renderer()
    OUT.mkdir(parents=True, exist_ok=True)
    for name, media in CASES.items():
        start = time.perf_counter()
        img = r.render_media(dict(media, thumbnail=False))
        ms = (time.perf_counter() - start) * 1000
        img.save(OUT / f"{name}.png")
        print(f"{name}.png  {ms:.1f} ms")


if __name__ == "__main__":
    main()
