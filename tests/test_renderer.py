"""Offline tests for tiga_windows_nowplaying_v2.py.

Run from the repository root:

    py -m unittest discover -s tests -v

Nothing here touches Bluetooth or the keyboard: every file the renderer would write is
redirected into a temporary directory.
"""
from __future__ import annotations

import importlib.util
import shutil
import tempfile
import unittest
import zlib
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def load_renderer():
    spec = importlib.util.spec_from_file_location("renderer", REPO / "tiga_windows_nowplaying_v2.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SandboxedRenderer(unittest.TestCase):
    """Loads a fresh renderer module whose output paths point into a temp dir."""

    def setUp(self):
        self.r = load_renderer()
        self.tmp = Path(tempfile.mkdtemp(prefix="tiga_renderer_test_"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.r.ACTIVE_STREAM = self.tmp / "session_01_bulk_stream.bin"
        self.r.OUTPUT_STREAM = self.tmp / "session_01_bulk_stream.windows_nowplaying.bin"
        self.r.FRAME_SIDECAR = self.tmp / "session_01_bulk_stream.bin.frame"
        self.r.PREVIEW = self.tmp / "preview.png"
        self.r.COMMAND_FILE = self.tmp / "tiga_display_command.txt"
        self.r.RUN_ID = "renderer-test"


class FrameCorrelationTests(SandboxedRenderer):
    """Issue #9: every installed frame carries an ID the uploader can log."""

    def read_sidecar(self):
        lines = self.r.FRAME_SIDECAR.read_text(encoding="utf-8").splitlines()
        return dict(line.split("=", 1) for line in lines)

    def test_sidecar_matches_installed_stream(self):
        frame_id = self.r.write_display(self.r.render_black(), "test")
        stream = self.r.ACTIVE_STREAM.read_bytes()
        sidecar = self.read_sidecar()

        self.assertEqual(frame_id, "renderer-test-f0001")
        self.assertEqual(sidecar["frame_id"], frame_id)
        self.assertEqual(sidecar["crc32"], f"{zlib.crc32(stream) & 0xFFFFFFFF:08x}")
        self.assertEqual(sidecar["reason"], "test")
        self.r.validate_stream(stream)

    def test_frame_ids_increase(self):
        first = self.r.write_display(self.r.render_black(), "a")
        second = self.r.write_display(self.r.render_standby("x"), "b")
        self.assertNotEqual(first, second)
        self.assertEqual(self.read_sidecar()["frame_id"], second)


if __name__ == "__main__":
    unittest.main()
