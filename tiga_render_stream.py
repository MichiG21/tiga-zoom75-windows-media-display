"""The DLX stream: a rendered image into the known-good container, installed for the uploader."""
from __future__ import annotations

import os
import time
import zlib
from datetime import datetime
from pathlib import Path

from PIL import Image

import tiga_dlx

WIDTH = 320
HEIGHT = 172
FRAME_BYTES = WIDTH * HEIGHT * 2

FILE_START_IN_STREAM = 25
DLX_PREFIX_BYTES = 8
HEADER_BYTES = 452
FRAME1_START = FILE_START_IN_STREAM + DLX_PREFIX_BYTES + HEADER_BYTES
FRAME2_START = FRAME1_START + FRAME_BYTES
TRAILER_START = FRAME2_START + FRAME_BYTES
EXPECTED_STREAM_LEN = TRAILER_START + 4


def stream_crc32(data: bytes) -> str:
    return f"{zlib.crc32(data) & 0xFFFFFFFF:08x}"


def rgb565_be_bytes(img: Image.Image) -> bytes:
    img = img.convert("RGB")
    out = bytearray(WIDTH * HEIGHT * 2)
    p = 0

    pixels = img.get_flattened_data() if hasattr(img, "get_flattened_data") else img.getdata()
    for r, g, b in pixels:
        value = ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3)
        out[p] = (value >> 8) & 0xFF
        out[p + 1] = value & 0xFF
        p += 2

    return bytes(out)


def validate_stream(data: bytes):
    if len(data) != EXPECTED_STREAM_LEN:
        raise RuntimeError(f"Unexpected stream size {len(data)}; expected {EXPECTED_STREAM_LEN}")
    if data[25:33] != bytes.fromhex("00 44 4c 58 fc ff 00 00"):
        raise RuntimeError("DLX prefix mismatch")
    if data[-4:] != bytes.fromhex("fc ff 00 00"):
        raise RuntimeError("DLX trailer mismatch")


def replace_with_retry(src: Path, dst: Path, *, attempts: int = 40, delay: float = 0.025,
                       sleep=time.sleep) -> bool:
    """os.replace(src, dst), retried while a reader holds dst open; False if it never lands.

    On Windows a file can't be replaced while another handle has it open without
    delete sharing, and Python's open(), .NET's ReadAllLines (the supervisor, the soak,
    STATUS_STACK) and C++'s ifstream (Display Control) all open it that way. Readers
    hold these files for milliseconds, so about 1 s of retries is plenty. Only that
    error (PermissionError) is retried; anything else still raises. On False, src is
    left in place and dst is unchanged.

    `sleep` is bound when this is defined, so tests that patch time.sleep to drive the
    main loop's clock don't count these waits as polls.
    """
    for attempt in range(1, attempts + 1):
        try:
            os.replace(src, dst)
            return True
        except PermissionError:
            if attempt == attempts:
                return False
            sleep(delay)
    return False


def replace_or_raise(src: Path, dst: Path):
    """For files that must not silently go stale: the frame record and the stream."""
    if not replace_with_retry(src, dst):
        raise RuntimeError(f"could not replace {dst}: another program kept it open")


def write_frame_sidecar(frame_id: str, crc: str, reason: str, frame_sidecar: Path):
    text = (
        f"frame_id={frame_id}\n"
        f"crc32={crc}\n"
        f"rendered_at={datetime.now().astimezone().isoformat(timespec='seconds')}\n"
        f"reason={reason}\n"
    )
    tmp = frame_sidecar.with_suffix(".frame.tmp")
    tmp.write_text(text, encoding="utf-8")
    replace_or_raise(tmp, frame_sidecar)


def install_image(
    img: Image.Image,
    frame_id: str,
    reason: str,
    *,
    backup_stream: Path,
    output_stream: Path,
    active_stream: Path,
    frame_sidecar: Path,
    dlx_frames: int,
    dlx_layout: str,
) -> str:
    if not backup_stream.exists():
        raise RuntimeError(f"Known-good stream backup missing: {backup_stream}")

    base = backup_stream.read_bytes()
    validate_stream(base)

    frame = rgb565_be_bytes(img)
    if dlx_frames == 2:
        modified = bytearray(base)
        modified[FRAME1_START:FRAME1_START + FRAME_BYTES] = frame
        modified[FRAME2_START:FRAME2_START + FRAME_BYTES] = frame
        modified = bytes(modified)
        validate_stream(modified)
    else:
        modified = tiga_dlx.build_stream(base, [frame], dlx_layout)

    output_stream.write_bytes(modified)

    crc = stream_crc32(modified)
    write_frame_sidecar(frame_id, crc, reason, frame_sidecar)

    tmp = active_stream.with_suffix(".bin.tmp")
    tmp.write_bytes(modified)
    replace_or_raise(tmp, active_stream)
    return crc
