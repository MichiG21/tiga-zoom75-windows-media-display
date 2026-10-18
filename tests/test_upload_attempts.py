"""Offline tests for tools/upload_attempts.py (the upload-attempt table).

Synthetic log snippets only; nothing here opens Bluetooth.
"""
from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "tools"))

import upload_attempts as ua  # noqa: E402

BLE = """\
===== 2026-10-18 13:41:20 +02:00 ble run=ble-A start =====
2026-10-18T11:41:20Z TIGA flow-controlled stream uploader starting (run_id=ble-A)
Intra-block pace: 0 ms/write
Block timeout   : 2500 ms
2026-10-18T11:41:20Z Scanning for Zoom75 Tiga...
2026-10-18T11:41:20Z CANDIDATE name="Zoom75 Tiga" address=07:01:88:B3:66:1B rssi=-55 (cached)
2026-10-18T11:41:21Z Found keyboard at 07:01:88:B3:66:1B
2026-10-18T11:41:21Z Connected once. Notifications active.

2026-10-18T11:41:21Z UPLOAD START frame=r-f0001 crc32=a3f02116
  block  5/53 ACKed  writes= 115/1239  elapsed=0.84s
2026-10-18T11:41:30Z UPLOAD FAILED frame=r-f0001 crc32=a3f02116 after 8.50s: timed out waiting for PROGRESS after 4K block 37/53 (last ACKed block 36/53)
2026-10-18T11:41:30Z TIMING frame=r-f0001 blocks_acked=36 median=154ms p90=172ms max=193ms slowest=block 1 control1_ack=26ms control2_ack=66ms
2026-10-18T11:41:30Z EXIT code=20 (transfer interrupted after CONTROL2)
===== 2026-10-18 13:42:16 +02:00 ble run=ble-B start =====
Intra-block pace: 0 ms/write
2026-10-18T11:42:16Z Scanning for Zoom75 Tiga...
2026-10-18T11:42:18Z CANDIDATE name="Zoom75 Tiga" address=07:01:88:B3:66:1B rssi=-61
2026-10-18T11:42:19Z Found keyboard at 07:01:88:B3:66:1B
2026-10-18T11:42:19Z Connected once. Notifications active.

2026-10-18T11:42:19Z UPLOAD START frame=r-f0003 crc32=f72ad5c0
  block  5/53 ACKed  writes= 115/1239  elapsed=0.84s
  block 10/53 ACKed  writes= 230/1239  elapsed=1.60s
2026-10-18T11:42:28Z UPLOAD COMPLETE frame=r-f0003 crc32=f72ad5c0 in 8.71s

2026-10-18T11:48:40Z UPLOAD START frame=r-f0004 crc32=d3523165
  block  5/53 ACKed  writes= 115/1239  elapsed=1.50s
  block 10/53 ACKed  writes= 230/1239  elapsed=12.00s
2026-10-18T11:48:55Z UPLOAD FAILED frame=r-f0004 crc32=d3523165 after 14.90s: timed out waiting for PROGRESS after 4K block 11/53 (last ACKed block 10/53)
"""

SUPERVISOR = """\
2026-10-18 13:41:20 TIGA startup supervisor launched (pid 1).
2026-10-18 13:41:20 Started ble run=ble-A pid=2
2026-10-18 13:42:16 Halt cleared (x removed); starting the BLE uploader again.
2026-10-18 13:42:16 Started ble run=ble-B pid=3
"""

RENDERER = """\
2026-10-18T13:41:20+02:00 FRAME r-f0001 rendered (startup standby)
2026-10-18T13:41:20+02:00 FRAME r-f0001 installed crc32=a3f02116
2026-10-18T13:41:27+02:00 FRAME r-f0003 rendered (media changed)
2026-10-18T13:41:27+02:00 FRAME r-f0003 installed crc32=f72ad5c0
"""

CONTEXT = """\
install,stack_event_local,start_kind,battery_disconnect,source
t,2026-10-18 13:41:20,STOP_STACK + START_STACK,about 1.5 min,test
"""


class UploadAttemptTableTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        d = Path(cls.tmp.name)
        (d / "ble.log").write_text(BLE, encoding="utf-8")
        (d / "startup_supervisor.log").write_text(SUPERVISOR, encoding="utf-8")
        (d / "renderer.log").write_text(RENDERER, encoding="utf-8")
        (d / "ctx.csv").write_text(CONTEXT, encoding="utf-8")
        cls.rows = ua.build({"t": str(d)}, str(d / "ctx.csv"))

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_one_row_per_upload_start_in_local_time(self):
        self.assertEqual([r["start_local"] for r in self.rows],
                         ["2026-10-18 13:41:21", "2026-10-18 13:42:19", "2026-10-18 13:48:40"])
        self.assertEqual([r["upload_no"] for r in self.rows], [1, 1, 2])

    def test_how_started_and_context(self):
        a, b, c = self.rows
        self.assertEqual(a["how_started"], "STOP_STACK + START_STACK")
        self.assertEqual(a["battery_disconnect_before"], "about 1.5 min")
        self.assertEqual(b["how_started"], "halt cleared on a running supervisor")
        self.assertEqual(b["battery_disconnect_before"], "")
        self.assertEqual(c["how_started"], "later upload on an open connection")

    def test_failure_fields(self):
        a = self.rows[0]
        self.assertEqual((a["outcome"], a["duration_s"], a["exit_code"]), ("failed", "8.50", "20"))
        self.assertEqual((a["last_acked_block"], a["failed_waiting_for_block"]), (36, 37))
        self.assertEqual(a["signature"], "sudden")
        self.assertEqual((a["control1_ack_ms"], a["control2_ack_ms"]), ("26", "66"))
        self.assertEqual(a["rssi"], "-55")
        self.assertEqual(a["trigger"], "startup standby")
        self.assertEqual(a["failed_block_region"], "frame 2")

    def test_gaps_and_slowing_signature(self):
        b, c = self.rows[1], self.rows[2]
        self.assertEqual(b["since_prev_failed_upload_s"], "49")
        self.assertEqual(c["since_prev_upload_on_conn_s"], "372")
        self.assertEqual(c["signature"], "slowing")
        self.assertEqual(c["latency_source"], "5-block averages")

    def test_frame_boundary_block(self):
        self.assertEqual(ua.block_region(27), "contains frame-1/frame-2 boundary")
        self.assertEqual(ua.block_region(26), "frame 1")
        self.assertEqual(ua.block_region(28), "frame 2")


if __name__ == "__main__":
    unittest.main()
