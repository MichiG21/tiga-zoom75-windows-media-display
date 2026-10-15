"""Offline structure tests for the DLX container model (tiga_dlx.py).

Nothing here is sent anywhere. A one-frame container can only be validated on
hardware; these tests check that it is internally consistent and that the model
reproduces the known-good capture exactly.
"""
from __future__ import annotations

import struct
import sys
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO))

import tiga_dlx as dlx  # noqa: E402
from test_renderer import SandboxedRenderer  # noqa: E402

TEMPLATE = (REPO / "extracted" / "session_01_bulk_stream.redgreen-backup.bin").read_bytes()
RED = bytes.fromhex("f8 00") * (dlx.FRAME_BYTES // 2)
GREEN = bytes.fromhex("07 e0") * (dlx.FRAME_BYTES // 2)
BLUE = bytes.fromhex("00 1f") * (dlx.FRAME_BYTES // 2)


class CapturedContainerTests(unittest.TestCase):
    def test_capture_fields(self):
        p = dlx.parse(TEMPLATE)
        self.assertEqual(p["stream_len"], 220_649)
        self.assertEqual(p["file_len"], 220_624)
        self.assertEqual(p["header_len"], 452)
        self.assertEqual(p["section_table"], 352)
        self.assertEqual(p["record_count"], 3)
        self.assertEqual(p["frame_count"], 2)
        self.assertEqual((p["records_at"], p["frame_index_at"]), (368, 452))
        self.assertEqual((p["pixel_data_at"], p["pixel_data_end"]), (460, 220_620))
        self.assertEqual(p["frame_index"], [0, dlx.FRAME_BYTES])
        self.assertEqual([r["count"] for r in p["records"]], [1, 2, 1])
        self.assertEqual({(r["width"], r["height"]) for r in p["records"]}, {(320, 172)})
        self.assertEqual(p["frames"], [RED, GREEN])

    def test_model_rebuilds_capture_byte_for_byte(self):
        for layout in dlx.LAYOUTS:
            with self.subTest(layout=layout):
                self.assertEqual(dlx.build_stream(TEMPLATE, [RED, GREEN], layout), TEMPLATE)

    def test_bad_frame_size_rejected(self):
        with self.assertRaises(ValueError):
            dlx.build_stream(TEMPLATE, [RED[:-2]])


class SingleFrameContainerTests(unittest.TestCase):
    def check_common(self, stream):
        p = dlx.parse(stream)  # envelope length, prefix and trailer are checked here
        f = stream[dlx.FILE_START:]
        self.assertEqual(p["frame_count"], 1)
        self.assertEqual(p["records"][1]["count"], 1)
        self.assertEqual(p["pixel_data_end"], p["file_len"] - 4)
        self.assertEqual(f[p["pixel_data_at"]:p["pixel_data_end"]], BLUE)
        # Nothing outside the frame-dependent fields moved.
        self.assertEqual(stream[dlx.ENVELOPE:dlx.FILE_START], TEMPLATE[dlx.ENVELOPE:dlx.FILE_START])
        t = TEMPLATE[dlx.FILE_START:]
        self.assertEqual(f[:dlx.F_FRAME_COUNT], t[:dlx.F_FRAME_COUNT])
        self.assertEqual(f[dlx.F_UNKNOWN_14:dlx.SECTION_TABLE], t[dlx.F_UNKNOWN_14:dlx.SECTION_TABLE])
        rec2_count = dlx.RECORDS + dlx.RECORD_LEN + dlx.RECORD_COUNT_FIELD
        self.assertEqual(f[dlx.RECORDS:rec2_count], t[dlx.RECORDS:rec2_count])
        self.assertEqual(f[rec2_count + 2:dlx.FRAME_INDEX], t[rec2_count + 2:dlx.FRAME_INDEX])
        return p

    def test_index_layout(self):
        s = dlx.build_stream(TEMPLATE, [BLUE], "index")
        p = self.check_common(s)
        self.assertEqual((p["stream_len"], p["file_len"], p["header_len"]), (110_565, 110_540, 448))
        self.assertEqual(p["frame_index"], [0])
        self.assertEqual(struct.unpack_from("<4I", s, dlx.FILE_START + 352), (368, 452, 456, 110_536))

    def test_fixed_layout(self):
        s = dlx.build_stream(TEMPLATE, [BLUE], "fixed")
        p = self.check_common(s)
        self.assertEqual((p["stream_len"], p["file_len"], p["header_len"]), (110_569, 110_544, 452))
        self.assertEqual(p["frame_index"], [0, dlx.FRAME_BYTES])
        self.assertEqual(struct.unpack_from("<4I", s, dlx.FILE_START + 352), (368, 452, 460, 110_540))

    def test_upload_block_count_halves(self):
        # The uploader waits for one PROGRESS per complete 4096-byte block of the file.
        self.assertEqual(dlx.CAPTURED_FILE_LEN // 4096, 53)
        self.assertEqual(110_540 // 4096, 26)


class RendererFlagTests(SandboxedRenderer):
    def test_default_output_unchanged_and_matches_model(self):
        self.r.write_display(self.r.render_black(), "test")
        out = self.r.ACTIVE_STREAM.read_bytes()
        black = bytes(dlx.FRAME_BYTES)
        self.assertEqual(out, dlx.build_stream(TEMPLATE, [black, black]))
        self.assertEqual(len(out), dlx.CAPTURED_STREAM_LEN)

    def test_single_frame_flag(self):
        self.r.DLX_FRAMES = 1
        self.r.write_display(self.r.render_black(), "test")
        out = self.r.ACTIVE_STREAM.read_bytes()
        p = dlx.parse(out)
        self.assertEqual((p["frame_count"], p["stream_len"]), (1, 110_565))
        sidecar = self.r.FRAME_SIDECAR.read_text(encoding="utf-8")
        self.assertIn(f"crc32={self.r.stream_crc32(out)}", sidecar)


if __name__ == "__main__":
    unittest.main()
