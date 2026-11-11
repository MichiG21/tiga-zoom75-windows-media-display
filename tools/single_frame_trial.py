"""Build the test streams for the one-frame hardware trial (docs/DLX_FORMAT.md).

Writes three upload streams into --out, each showing its own name on the LCD, so a
photo of the keyboard says which container it accepted:

  1_baseline_2frame.bin   the captured 2-frame layout (what production sends)
  2_single_index.bin      one frame, frame index [0]          (--dlx-layout index)
  3_single_fixed.bin      one frame, frame index [0, 110080]  (--dlx-layout fixed)

Nothing here opens Bluetooth. It only writes files; the hardware steps are in
docs/DLX_FORMAT.md, "Hardware trial, step by step".

    py tools\\single_frame_trial.py --out <empty folder>
"""
from __future__ import annotations

import argparse
import sys
import zlib
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

import tiga_dlx  # noqa: E402
from tiga_render_stream import rgb565_be_bytes  # noqa: E402

TEMPLATE = ROOT / "extracted" / "session_01_bulk_stream.redgreen-backup.bin"

# name, frames, layout, background (RGB), label
VARIANTS = [
    ("1_baseline_2frame.bin", 2, "index", (90, 90, 90), "BASELINE 2 FRAMES"),
    ("2_single_index.bin", 1, "index", (0, 70, 160), "1 FRAME / INDEX"),
    ("3_single_fixed.bin", 1, "fixed", (120, 0, 120), "1 FRAME / FIXED"),
]


def label_image(background: tuple[int, int, int], label: str, step: str) -> Image.Image:
    img = Image.new("RGB", (tiga_dlx.WIDTH, tiga_dlx.HEIGHT), background)
    draw = ImageDraw.Draw(img)
    big = ImageFont.load_default(size=30)
    small = ImageFont.load_default(size=16)
    draw.text((tiga_dlx.WIDTH // 2, 70), label, font=big, fill=(255, 255, 255), anchor="mm")
    draw.text((tiga_dlx.WIDTH // 2, 120), f"trial step {step}", font=small, fill=(255, 255, 255), anchor="mm")
    # Corner marks: a half-drawn frame would lose the bottom ones first.
    for x, y in ((0, 0), (tiga_dlx.WIDTH - 12, 0), (0, tiga_dlx.HEIGHT - 12), (tiga_dlx.WIDTH - 12, tiga_dlx.HEIGHT - 12)):
        draw.rectangle((x, y, x + 11, y + 11), fill=(255, 220, 0))
    return img


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, type=Path, help="folder for the streams (created if missing)")
    args = ap.parse_args()

    template = TEMPLATE.read_bytes()
    args.out.mkdir(parents=True, exist_ok=True)
    for i, (name, frames, layout, bg, label) in enumerate(VARIANTS, start=1):
        frame = rgb565_be_bytes(label_image(bg, label, str(i)))
        stream = tiga_dlx.build_stream(template, [frame] * frames, layout)
        info = tiga_dlx.parse(stream)  # the same checks the renderer's output gets
        path = args.out / name
        path.write_bytes(stream)
        print(f"{name:24} {len(stream):>7} bytes  crc32 {zlib.crc32(stream) & 0xFFFFFFFF:08x}  frames {len(info['frames'])}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
