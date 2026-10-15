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
        self.r.COMMAND_ACK_FILE = self.tmp / "tiga_display_command.ack"
        self.r.RUN_ID = "renderer-test"


class FrameCorrelationTests(SandboxedRenderer):
    """Issue #9: every installed frame carries an ID the uploader can log."""

    def read_sidecar(self):
        lines = self.r.FRAME_SIDECAR.read_text(encoding="utf-8").splitlines()
        return dict(line.split("=", 1) for line in lines)

    def test_sidecar_matches_installed_stream(self):
        frame_id, crc = self.r.write_display(self.r.render_black(), "test")
        stream = self.r.ACTIVE_STREAM.read_bytes()
        sidecar = self.read_sidecar()

        self.assertEqual(frame_id, "renderer-test-f0001")
        self.assertEqual(sidecar["frame_id"], frame_id)
        self.assertEqual(sidecar["crc32"], f"{zlib.crc32(stream) & 0xFFFFFFFF:08x}")
        self.assertEqual(sidecar["crc32"], crc)
        self.assertEqual(sidecar["reason"], "test")
        self.r.validate_stream(stream)

    def test_frame_ids_increase(self):
        first, _ = self.r.write_display(self.r.render_black(), "a")
        second, _ = self.r.write_display(self.r.render_standby("x"), "b")
        self.assertNotEqual(first, second)
        self.assertEqual(self.read_sidecar()["frame_id"], second)


class ControlCommandTests(SandboxedRenderer):
    """Issue #6: a repeated command must register as new."""

    def send(self, text, last):
        self.r.COMMAND_FILE.write_text(text + "\n", encoding="utf-8")
        return self.r.read_control_command(last)

    def test_same_command_twice_with_new_ids_registers_twice(self):
        cmd, last = self.send("standby 133700000000000000-42-1", None)
        self.assertEqual(cmd, "standby")
        cmd, last = self.send("standby 133700000000000001-42-2", last)
        self.assertEqual(cmd, "standby")

    def test_unchanged_file_is_not_reprocessed(self):
        cmd, last = self.send("black 1-1-1", None)
        self.assertEqual(cmd, "black")
        self.assertEqual(self.r.read_control_command(last), (None, last))

    def test_command_left_from_previous_session_is_ignored(self):
        # main() records the file's text at startup as already handled.
        self.r.COMMAND_FILE.write_text("standby 1-1-1\n", encoding="utf-8")
        startup_text = self.r.COMMAND_FILE.read_text(encoding="utf-8").strip().lower()
        self.assertEqual(self.r.read_control_command(startup_text), (None, startup_text))

    def test_legacy_bare_command_still_works(self):
        cmd, last = self.send("auto", None)
        self.assertEqual((cmd, last), ("auto", "auto"))

    def test_unknown_command_ignored(self):
        cmd, _ = self.send("reboot 1-1-1", None)
        self.assertIsNone(cmd)


class TextLayoutTests(unittest.TestCase):
    """Issue #10: text must stay inside its box."""

    MAX_W = 174

    def setUp(self):
        from PIL import Image, ImageDraw

        self.r = load_renderer()
        self.draw = ImageDraw.Draw(Image.new("RGB", (8, 8)))
        self.font = self.r.find_font(18, bold=True)

    def fit(self, text, lines=3):
        out = self.r.fit_text(self.draw, text, self.font, self.MAX_W, lines)
        for line in out:
            self.assertLessEqual(self.draw.textbbox((0, 0), line, font=self.font)[2], self.MAX_W, line)
        self.assertLessEqual(len(out), lines)
        return out

    def test_long_unbroken_title_is_wrapped(self):
        out = self.fit("A" * 60)
        self.assertEqual(len(out), 3)
        self.assertTrue(out[-1].endswith("…"))

    def test_cjk_title_without_spaces_is_wrapped(self):
        self.fit("日本語のとても長いタイトルがここにありますよ本当に長いです")

    def test_long_word_that_fits_in_lines_has_no_ellipsis(self):
        out = self.fit("Supercalifragilisticexpialidocious")
        self.assertEqual("".join(out), "Supercalifragilisticexpialidocious")

    def test_short_title_unchanged(self):
        self.assertEqual(self.fit("Short title"), ["Short title"])

    def test_many_words_truncated_with_ellipsis(self):
        out = self.fit("Word " * 20)
        self.assertEqual(len(out), 3)
        self.assertTrue(out[-1].endswith("…"))

    def test_exactly_fitting_text_has_no_ellipsis(self):
        out = self.fit("one two", lines=1)
        self.assertEqual(out, ["one two"])

    def layout(self, title, artist, album):
        r = self.r
        fonts = (r.find_font(18, True), r.find_font(18), r.find_font(16))
        return r.layout_text_block(self.draw, title, artist, album, *fonts, self.MAX_W)

    def role(self, item):
        return {242: "title", 224: "artist", 180: "album"}[item[3][0]]

    def assert_no_overlap(self, items):
        step = {"title": self.r.TITLE_STEP, "artist": self.r.ARTIST_STEP, "album": self.r.ALBUM_STEP}
        prev_bottom = 0
        for item in items:
            self.assertGreaterEqual(item[0], prev_bottom, items)
            prev_bottom = item[0] + step[self.role(item)]
        self.assertLessEqual(prev_bottom, self.r.DIVIDER_Y, items)

    def count(self, items, role):
        return sum(1 for i in items if self.role(i) == role)

    def test_album_does_not_overlap_two_line_artist(self):
        items = self.layout("Title", "A very long list of featured artists on this track", "The Album Name")
        self.assertEqual(self.count(items, "artist"), 2)
        self.assertEqual(self.count(items, "album"), 1)
        self.assert_no_overlap(items)

    def test_worst_case_layout_stays_above_divider(self):
        items = self.layout(
            "Word " * 20,
            "A very long list of featured artists on this track",
            "A rather long album title that needs two lines",
        )
        self.assertEqual(self.count(items, "title"), 3)
        self.assertEqual(self.count(items, "artist"), 2)
        self.assertEqual(self.count(items, "album"), 1)
        self.assert_no_overlap(items)

    def test_album_gets_two_lines_when_artist_is_short(self):
        items = self.layout("Title", "Artist", "A rather long album title that needs two lines")
        self.assertEqual(self.count(items, "album"), 2)
        self.assert_no_overlap(items)


class ArtworkChangeTests(SandboxedRenderer):
    """Issue #10: artwork changes are detected by content, not byte count."""

    def setUp(self):
        super().setUp()
        self.r.THUMBNAIL_FILE = self.tmp / "thumb.bin"

    def key_with_art(self, data: bytes):
        self.r.THUMBNAIL_FILE.write_bytes(data)
        return self.r.media_key(media(thumbnail=True, thumbnail_bytes=len(data)))

    def test_same_size_different_artwork_changes_key(self):
        self.assertNotEqual(self.key_with_art(b"\x01" * 1000), self.key_with_art(b"\x02" * 1000))

    def test_same_artwork_keeps_key(self):
        self.assertEqual(self.key_with_art(b"\x01" * 1000), self.key_with_art(b"\x01" * 1000))

    def test_no_thumbnail_ignores_stale_file(self):
        self.r.THUMBNAIL_FILE.write_bytes(b"old art")
        a = self.r.media_key(media(thumbnail=False))
        self.r.THUMBNAIL_FILE.write_bytes(b"other old art")
        self.assertEqual(a, self.r.media_key(media(thumbnail=False)))


def media(state="Playing", position=30_000, title="Song", **extra):
    m = {
        "present": True,
        "source": "Spotify.exe",
        "state": state,
        "title": title,
        "artist": "Artist",
        "album": "Album",
        "position_ms": position,
        "duration_ms": 200_000,
    }
    m.update(extra)
    return m


class FreshnessGateTests(unittest.TestCase):
    """Issue #5: the four cases from TASKS.md, plus the escape's own limits."""

    def setUp(self):
        self.r = load_renderer()

    def run_polls(self, gate, snapshots, start=0.0, step=1.0):
        """Feed snapshots one second apart; return (seconds_elapsed, reason) of the first release."""
        for i, snap in enumerate(snapshots):
            reason = gate.observe(snap, start + i * step)
            if reason:
                return i * step, reason
        return None, None

    def test_already_playing_frozen_timeline_releases_after_bound(self):
        gate = self.r.FreshnessGate(media(position=30_000))
        bound = self.r.STEADY_PLAYING_RELEASE_SECONDS
        polls = [media(position=30_000)] * int(bound + 5)
        elapsed, reason = self.run_polls(gate, polls)
        self.assertEqual(reason, "steady playback")
        self.assertEqual(elapsed, bound)

    def test_already_playing_frozen_timeline_holds_before_bound(self):
        gate = self.r.FreshnessGate(media(position=30_000))
        polls = [media(position=30_000)] * int(self.r.STEADY_PLAYING_RELEASE_SECONDS)
        self.assertEqual(self.run_polls(gate, polls), (None, None))

    def test_already_playing_advancing_timeline_releases_quickly(self):
        gate = self.r.FreshnessGate(media(position=30_000))
        polls = [media(position=30_000 + i * 1000) for i in range(10)]
        elapsed, reason = self.run_polls(gate, polls)
        self.assertEqual(reason, "timeline advancing")
        self.assertLessEqual(elapsed, 2.0)

    def test_pause_then_resume_releases_immediately(self):
        gate = self.r.FreshnessGate(media(position=30_000))
        self.assertIsNone(gate.observe(media(state="Paused", position=30_000), 0.0))
        self.assertEqual(gate.observe(media(position=30_000), 1.0), "playback started")

    def test_genuine_stale_session_stays_on_standby(self):
        # A remembered session after reboot/login: same item, not playing, never moves.
        stale = media(state="Paused", position=95_000)
        gate = self.r.FreshnessGate(stale)
        self.assertEqual(self.run_polls(gate, [stale] * 600), (None, None))

    def test_stale_playing_session_without_metadata_stays_on_standby(self):
        stale = media(position=0, title="")
        gate = self.r.FreshnessGate(stale)
        self.assertEqual(self.run_polls(gate, [stale] * 600), (None, None))

    def test_steady_clock_restarts_when_playback_is_interrupted(self):
        gate = self.r.FreshnessGate(media(position=30_000))
        bound = self.r.STEADY_PLAYING_RELEASE_SECONDS
        self.assertIsNone(gate.observe(media(position=30_000), 0.0))
        self.assertIsNone(gate.observe(media(position=30_000), bound - 1))
        gate.interrupt()  # e.g. the session briefly disappeared
        self.assertIsNone(gate.observe(media(position=30_000), bound + 1))
        self.assertEqual(gate.observe(media(position=30_000), 2 * bound + 1), "steady playback")

    def test_new_media_releases_immediately(self):
        gate = self.r.FreshnessGate(media(title="Old"))
        self.assertEqual(gate.observe(media(title="New"), 0.0), "new media")

    def test_no_startup_media_releases_on_first_playing(self):
        gate = self.r.FreshnessGate(None)
        self.assertEqual(gate.observe(media(), 0.0), "new media")


class _StopLoop(Exception):
    pass


class MainLoopGateTests(SandboxedRenderer):
    """Drives main() with a fake clock and fake snapshots to check the gate wiring."""

    def run_main(self, snapshots, max_polls, on_poll=None):
        clock = {"t": 0.0, "polls": 0}
        rendered = []
        snaps = iter(snapshots)
        last = {"snap": None}

        def fake_snapshot():
            try:
                last["snap"] = next(snaps)
            except StopIteration:
                pass
            if isinstance(last["snap"], Exception):
                raise last["snap"]
            return last["snap"]

        def fake_sleep(seconds):
            clock["t"] += seconds
            clock["polls"] += 1
            if on_poll:
                on_poll(clock["polls"])
            if clock["polls"] >= max_polls:
                raise _StopLoop

        self.r.snapshot = fake_snapshot

        def fake_write_display(img, reason, preview=True):
            rendered.append(reason)
            return f"f{len(rendered)}", f"{len(rendered):08x}"

        self.r.write_display = fake_write_display
        self.r.time.sleep = fake_sleep
        self.r.time.monotonic = lambda: clock["t"]
        self.addCleanup(setattr, self.r.time, "sleep", __import__("time").sleep)
        self.addCleanup(setattr, self.r.time, "monotonic", __import__("time").monotonic)
        with self.assertRaises(_StopLoop):
            self.r.main()
        return rendered

    def test_already_playing_frozen_timeline_reaches_now_playing(self):
        frozen = media(position=30_000)
        rendered = self.run_main([frozen] * 40, max_polls=40)
        self.assertEqual(rendered[0], "startup standby")
        self.assertIn("fresh playback: steady playback", rendered)

    def test_command_is_acknowledged_with_its_frame(self):
        # Issue #7: the control utility matches the ack to its own command text.
        def send(poll):
            if poll == 3:
                self.r.COMMAND_FILE.write_text("standby 99-1-1\n", encoding="utf-8")

        self.r.COMMAND_FILE.write_text("auto 1-1-1\n", encoding="utf-8")  # left from last session
        rendered = self.run_main([media(state="Paused")] * 10, max_polls=6, on_poll=send)
        self.assertEqual(rendered, ["startup standby", "control: standby"])
        ack = dict(
            line.split("=", 1)
            for line in self.r.COMMAND_ACK_FILE.read_text(encoding="utf-8").splitlines()
        )
        self.assertEqual(ack["command_text"], "standby 99-1-1")
        self.assertEqual(ack["command"], "standby")
        self.assertEqual((ack["frame_id"], ack["crc32"]), ("f2", "00000002"))

    def test_persistent_query_failure_falls_back_to_standby(self):
        # Issue #10: playing, then the media query starts failing for good.
        playing = [{"present": False}] + [media(title="New")] * 3  # first one arms the gate
        idle = int(self.r.IDLE_TO_STANDBY_SECONDS)
        rendered = self.run_main(playing + [RuntimeError("helper crashed")] * (idle + 10), max_polls=idle + 20)
        self.assertIn("fresh playback: new media", rendered)
        self.assertEqual(rendered[-1], "idle: no media")

    def test_stale_paused_session_never_leaves_standby(self):
        stale = media(state="Paused", position=95_000)
        rendered = self.run_main([stale] * 120, max_polls=120)
        self.assertEqual(rendered, ["startup standby"])


if __name__ == "__main__":
    unittest.main()
