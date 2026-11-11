"""The one-frame trial streams (tools/single_frame_trial.py): sizes and parse checks only."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

import tiga_dlx  # noqa: E402


class SingleFrameTrialTest(unittest.TestCase):
    def test_writes_the_three_variants(self):
        with tempfile.TemporaryDirectory() as out:
            subprocess.run([sys.executable, str(ROOT / "tools" / "single_frame_trial.py"), "--out", out],
                           check=True, capture_output=True)
            expected = {  # docs/DLX_FORMAT.md, "A one-frame container"
                "1_baseline_2frame.bin": (tiga_dlx.CAPTURED_STREAM_LEN, 2),
                "2_single_index.bin": (110_565, 1),
                "3_single_fixed.bin": (110_569, 1),
            }
            self.assertEqual(sorted(p.name for p in Path(out).iterdir()), sorted(expected))
            for name, (size, frames) in expected.items():
                data = (Path(out) / name).read_bytes()
                self.assertEqual(len(data), size, name)
                self.assertEqual(len(tiga_dlx.parse(data)["frames"]), frames, name)
            # Each variant carries its own picture, so the LCD shows which one arrived.
            pixels = {(Path(out) / n).read_bytes()[-4 - tiga_dlx.FRAME_BYTES:-4] for n in expected}
            self.assertEqual(len(pixels), 3)


if __name__ == "__main__":
    unittest.main()
