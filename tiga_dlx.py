"""Model of the DLX upload stream used by the TIGA BLE uploader.

Offsets below are relative to the DLX file (the "00 DLX FC FF 00 00" prefix is at
file offset 0; the file starts at stream offset 25). Field names marked
"hypothesis" are inferred from a single 2-frame capture and are not proven; see
docs/DLX_FORMAT.md.

The production path is still the captured 2-frame container with its pixels
replaced. build_stream() with the captured red and green frames reproduces it byte-for-byte, which is what the
offline tests check. A one-frame stream is experimental and has never been sent to a keyboard.
"""
from __future__ import annotations

import struct

WIDTH = 320
HEIGHT = 172
FRAME_BYTES = WIDTH * HEIGHT * 2

ENVELOPE = 8           # 88 00 00 00 <u24 BE length of everything after it> 00
PREAMBLE = 17          # 08 05 01 00 09 00 ff ff ff 00 ff ff ff 02 02 ff ff
FILE_START = ENVELOPE + PREAMBLE

DLX_PREFIX = bytes.fromhex("00 44 4c 58 fc ff 00 00")
TRAILER = bytes.fromhex("fc ff 00 00")

# Header fields (file offsets).
F_SECTION_TABLE_PTR = 8    # u16 LE = 352: where the section table starts
F_RECORD_COUNT = 10        # u16 LE = 3: number of 28-byte records
F_FRAME_COUNT = 12         # u16 LE = 2 in the capture (hypothesis: frame count)
F_UNKNOWN_14 = 14          # 00 01, unknown
SECTION_TABLE = 352        # 4 x u32 LE, all file offsets:
#                            records start, frame index start, pixel data start, pixel data end
RECORDS = 368
RECORD_LEN = 28
RECORD_COUNT_FIELD = 6     # within a record: u16 BE; record 2 holds 2 (hypothesis: frame count)
FRAME_INDEX = 452          # u32 LE per frame (hypothesis: offset from pixel data start)

CAPTURED_FILE_LEN = 8 + 452 + 2 * FRAME_BYTES + 4      # 220,624
CAPTURED_STREAM_LEN = FILE_START + CAPTURED_FILE_LEN   # 220,649

LAYOUTS = ("index", "fixed")


def u16le(data: bytes, off: int) -> int:
    return struct.unpack_from("<H", data, off)[0]


def u32le(data: bytes, off: int) -> int:
    return struct.unpack_from("<I", data, off)[0]


def parse(stream: bytes) -> dict:
    """Decode the known fields of a stream; raises ValueError on a malformed one."""
    if stream[:4] != b"\x88\x00\x00\x00" or stream[7] != 0:
        raise ValueError("outer 0x88 envelope mismatch")
    envelope_len = int.from_bytes(stream[4:7], "big")
    if envelope_len != len(stream) - ENVELOPE:
        raise ValueError(f"envelope length {envelope_len} != {len(stream) - ENVELOPE}")
    f = stream[FILE_START:]
    if f[:8] != DLX_PREFIX:
        raise ValueError("DLX prefix mismatch")
    if f[-4:] != TRAILER:
        raise ValueError("DLX trailer mismatch")

    table = u16le(f, F_SECTION_TABLE_PTR)
    records_at, index_at, data_at, data_end = struct.unpack_from("<4I", f, table)
    n_records = u16le(f, F_RECORD_COUNT)
    records = []
    for i in range(n_records):
        r = f[records_at + i * RECORD_LEN: records_at + (i + 1) * RECORD_LEN]
        records.append({
            "raw": r.hex(" "),
            "type": r[0],
            "id": r[1],
            "flags": int.from_bytes(r[2:4], "big"),
            "delay": r[4],
            "count": int.from_bytes(r[6:8], "big"),
            "width": int.from_bytes(r[10:12], "big"),
            "height": int.from_bytes(r[12:14], "big"),
        })
    index = [u32le(f, off) for off in range(index_at, data_at, 4)]
    return {
        "stream_len": len(stream),
        "file_len": len(f),
        "header_len": data_at - 8,
        "section_table": table,
        "record_count": n_records,
        "frame_count": u16le(f, F_FRAME_COUNT),
        "unknown_14": f[F_UNKNOWN_14:F_UNKNOWN_14 + 2].hex(" "),
        "records_at": records_at,
        "frame_index_at": index_at,
        "pixel_data_at": data_at,
        "pixel_data_end": data_end,
        "records": records,
        "frame_index": index,
        "frames": [
            f[data_at + off: data_at + off + FRAME_BYTES]
            for off in index
            if data_at + off + FRAME_BYTES <= data_end
        ],
    }


def build_stream(template: bytes, frame_data: list[bytes], layout: str = "index") -> bytes:
    """Build an upload stream from one or two RGB565-BE frames.

    The captured header (records, 0xFF fill, preamble) is reused; only the fields
    that depend on the frame count and sizes are rewritten.

    layout="index": the block at FRAME_INDEX is one u32 offset per frame
                    (the capture's [0, 110080] read as offsets).
    layout="fixed": the block is kept as the capture's [0, 110080] whatever the
                    frame count (the same bytes read as [offset, frame size]).
    Both give the captured bytes for two frames.
    """
    frames = len(frame_data)
    if any(len(fr) != FRAME_BYTES for fr in frame_data):
        raise ValueError(f"every frame must be {FRAME_BYTES} bytes")
    if frames not in (1, 2):
        raise ValueError("only 1 or 2 frames are modelled")
    if layout not in LAYOUTS:
        raise ValueError(f"layout must be one of {LAYOUTS}")

    parse(template)  # validates the template
    f_tpl = template[FILE_START:]

    head = bytearray(f_tpl[:FRAME_INDEX])  # prefix + header up to the frame index
    if layout == "index":
        index = b"".join(struct.pack("<I", k * FRAME_BYTES) for k in range(frames))
    else:
        index = struct.pack("<2I", 0, FRAME_BYTES)
    data_at = FRAME_INDEX + len(index)
    data_end = data_at + frames * FRAME_BYTES

    struct.pack_into("<H", head, F_FRAME_COUNT, frames)
    rec2 = RECORDS + RECORD_LEN + RECORD_COUNT_FIELD
    head[rec2:rec2 + 2] = frames.to_bytes(2, "big")
    struct.pack_into("<4I", head, SECTION_TABLE, RECORDS, FRAME_INDEX, data_at, data_end)

    dlx = bytes(head) + index + b"".join(frame_data) + TRAILER
    body = template[ENVELOPE:FILE_START] + dlx
    envelope = b"\x88\x00\x00\x00" + len(body).to_bytes(3, "big") + b"\x00"
    stream = envelope + body
    parse(stream)
    return stream
