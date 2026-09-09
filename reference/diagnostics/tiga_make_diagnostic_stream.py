#!/usr/bin/env python3
from pathlib import Path
import argparse, hashlib, shutil, sys

WIDTH = 320
HEIGHT = 172
FRAME_BYTES = WIDTH * HEIGHT * 2

# Layout proven from the PocketWuque red/green capture.
FILE_START_IN_STREAM = 25
DLX_PREFIX_BYTES = 8
HEADER_BYTES = 452
FRAME1_START = FILE_START_IN_STREAM + DLX_PREFIX_BYTES + HEADER_BYTES
FRAME2_START = FRAME1_START + FRAME_BYTES
TRAILER_START = FRAME2_START + FRAME_BYTES
EXPECTED_STREAM_LEN = TRAILER_START + 4

KNOWN_GOOD_SHA256 = "a4be0a5df3e785a7cc4a13969d1b28b72312eda684627e487859ef11bdfae7d0"

def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

def rgb565_be(r: int, g: int, b: int) -> bytes:
    value = ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3)
    return bytes([(value >> 8) & 0xFF, value & 0xFF])

def make_pattern_frame() -> bytes:
    # Same frame is used twice so the TIGA animation appears visually static.
    # This makes any fixed horizontal/vertical artifact much easier to inspect.
    colors_top = [
        (255, 0, 0),       # red
        (0, 255, 0),       # green
        (0, 0, 255),       # blue
        (255, 255, 255),   # white
    ]
    colors_bottom = [
        (0, 0, 0),         # black
        (128, 128, 128),   # gray
        (255, 255, 0),     # yellow
        (0, 255, 255),     # cyan
    ]

    out = bytearray(FRAME_BYTES)
    p = 0
    for y in range(HEIGHT):
        palette = colors_top if y < HEIGHT // 2 else colors_bottom
        for x in range(WIDTH):
            idx = min(3, (x * 4) // WIDTH)
            out[p:p+2] = rgb565_be(*palette[idx])
            p += 2
    return bytes(out)

def validate_layout(data: bytes) -> None:
    if len(data) != EXPECTED_STREAM_LEN:
        raise SystemExit(f"Unexpected stream size: {len(data)}; expected {EXPECTED_STREAM_LEN}")
    if data[0:8] != bytes.fromhex("88 00 00 00 03 5d e1 00"):
        raise SystemExit("Outer 0x88 envelope does not match the known PocketWuque stream.")
    if data[25:33] != bytes.fromhex("00 44 4c 58 fc ff 00 00"):
        raise SystemExit("DLX prefix is not at the expected offset.")
    if data[-4:] != bytes.fromhex("fc ff 00 00"):
        raise SystemExit("DLX trailer does not match.")

def main():
    parser = argparse.ArgumentParser(
        description="Build/install a static diagnostic pattern inside the proven 2-frame TIGA/PocketWuque stream template."
    )
    parser.add_argument("--install", action="store_true",
                        help="Replace session_01_bulk_stream.bin with the diagnostic stream after creating a backup.")
    parser.add_argument("--restore", action="store_true",
                        help="Restore the exact captured red/green stream from the backup.")
    parser.add_argument("--stream", type=Path, default=None,
                        help="Path to session_01_bulk_stream.bin. Default: ../extracted/session_01_bulk_stream.bin relative to this script.")
    args = parser.parse_args()

    here = Path(__file__).resolve().parent
    stream = args.stream or (here / "extracted" / "session_01_bulk_stream.bin")
    backup = stream.with_name("session_01_bulk_stream.redgreen-backup.bin")
    generated = stream.with_name("session_01_bulk_stream.diagnostic.bin")

    if args.restore:
        if not backup.exists():
            raise SystemExit(f"No backup found: {backup}")
        restored = backup.read_bytes()
        validate_layout(restored)
        stream.write_bytes(restored)
        print(f"Restored: {stream}")
        print(f"SHA-256 : {sha256(restored)}")
        return

    if not stream.exists():
        raise SystemExit(f"Stream not found: {stream}")

    template = stream.read_bytes()
    validate_layout(template)

    # Prefer the pristine backup as template once it exists.
    if backup.exists():
        pristine = backup.read_bytes()
        validate_layout(pristine)
    else:
        pristine = template
        # Only create the permanent backup if the source is exactly the known successful capture.
        digest = sha256(pristine)
        if digest != KNOWN_GOOD_SHA256:
            raise SystemExit(
                "The current stream is not the exact captured red/green reference and no backup exists.\n"
                f"Current SHA-256: {digest}\n"
                "Refusing to overwrite anything until the known-good stream is restored."
            )
        shutil.copy2(stream, backup)
        print(f"Created permanent known-good backup: {backup}")

    frame = make_pattern_frame()
    modified = bytearray(pristine)
    modified[FRAME1_START:FRAME1_START + FRAME_BYTES] = frame
    modified[FRAME2_START:FRAME2_START + FRAME_BYTES] = frame
    modified = bytes(modified)
    validate_layout(modified)

    generated.write_bytes(modified)
    print("Diagnostic stream generated.")
    print(f"Output     : {generated}")
    print(f"Stream size: {len(modified)} bytes")
    print(f"Frame 1    : offset {FRAME1_START}, {FRAME_BYTES} bytes")
    print(f"Frame 2    : offset {FRAME2_START}, {FRAME_BYTES} bytes")
    print("Pattern    : TOP red | green | blue | white")
    print("             BOTTOM black | gray | yellow | cyan")
    print("Both frames are identical, so playback should LOOK STATIC.")
    print(f"SHA-256    : {sha256(modified)}")

    if args.install:
        stream.write_bytes(modified)
        print()
        print(f"INSTALLED into: {stream}")
        print("Cargo's include_bytes! should notice the changed file and rebuild on the next cargo run.")
        print("After the display test, restore with:")
        print(f'  python "{Path(__file__).name}" --restore')
    else:
        print()
        print("DRY GENERATION ONLY: the active replay stream was NOT replaced.")
        print("Run again with --install when ready.")

if __name__ == "__main__":
    main()
