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


FONTS = Path(__import__("os").environ.get("WINDIR", r"C:\Windows")) / "Fonts"


@unittest.skipUnless((FONTS / "segoeuib.ttf").exists(), "needs Windows fonts")
class FontFallbackTests(unittest.TestCase):
    """CJK titles rendered as empty boxes on hardware; so did the no-artwork "♪"."""

    def setUp(self):
        from PIL import Image, ImageDraw

        self.Image, self.ImageDraw = Image, ImageDraw
        self.r = load_renderer()
        self.r.log = lambda message: None
        self.chain = self.r.find_text_font(18, bold=True)

    def test_primary_font_lacks_cjk_so_the_test_means_something(self):
        self.assertFalse(self.r.has_glyph(self.chain.primary, "ト"))
        self.assertTrue(self.r.has_glyph(self.chain.primary, "A"))

    @unittest.skipUnless(any((FONTS / f).exists() for f in ("YuGothB.ttc", "meiryob.ttc", "msyhbd.ttc")), "no CJK font")
    def test_japanese_title_gets_a_font_for_every_character(self):
        title = "トーキョーレギー 東京"
        for ch in title:
            self.assertTrue(self.r.has_glyph(self.chain.font_for(ch), ch), ch)
        self.assertGreater(len(self.chain.runs("Takanaka トーキョー")), 1)

    @unittest.skipUnless((FONTS / "malgunbd.ttf").exists(), "no Korean font")
    def test_hangul_gets_a_font(self):
        self.assertTrue(self.r.has_glyph(self.chain.font_for("한"), "한"))

    def test_note_symbol_is_not_a_box(self):
        font = self.r.find_text_font(42, bold=True).font_for("♪")
        self.assertTrue(self.r.has_glyph(font, "♪"))

    def test_latin_text_draws_exactly_as_before(self):
        def render(draw_fn):
            img = self.Image.new("RGB", (200, 30))
            draw_fn(self.ImageDraw.Draw(img))
            return img.tobytes()

        text = "Supercalifragilistic"
        old = render(lambda d: d.text((2, 3), text, font=self.chain.primary, fill=(255, 255, 255)))
        new = render(lambda d: self.r.draw_text(d, (2, 3), text, self.chain, (255, 255, 255)))
        self.assertEqual(old, new)

    def test_mixed_title_wraps_inside_the_box(self):
        draw = self.ImageDraw.Draw(self.Image.new("RGB", (8, 8)))
        lines = self.r.fit_text(draw, "高中正義 Masayoshi Takanaka 渚・モデラート", self.chain, 174, 3)
        for line in lines:
            self.assertLessEqual(self.r.text_width(draw, line, self.chain), 174, line)

    def test_media_render_with_cjk_does_not_crash(self):
        img = self.r.render_media({"title": "トーキョーレギー", "artist": "高中正義", "album": "한국어", "thumbnail": False})
        self.assertEqual(img.size, (self.r.WIDTH, self.r.HEIGHT))


def fit_text_before_fallback(draw, text, font, max_width, max_lines):
    """fit_text() as it was on main before the font fallback (128745d), verbatim."""
    words = (text or "").split()
    if not words:
        return [""]

    def width(s: str) -> int:
        return draw.textbbox((0, 0), s, font=font)[2]

    tokens = []
    for word in words:
        if width(word) <= max_width:
            tokens.append(word)
            continue
        piece = ""
        for ch in word:
            if piece and width(piece + ch) > max_width:
                tokens.append(piece)
                piece = ch
            else:
                piece += ch
        tokens.append(piece)

    lines = []
    current = ""
    truncated = False

    for token in tokens:
        test = token if not current else current + " " + token
        if width(test) <= max_width:
            current = test
            continue
        lines.append(current)
        current = token
        if len(lines) >= max_lines:
            truncated = True
            current = ""
            break

    if current:
        if len(lines) < max_lines:
            lines.append(current)
        else:
            truncated = True

    if truncated:
        last = lines[-1]
        ell = "…"
        while last and width(last + ell) > max_width:
            last = last[:-1]
        lines[-1] = last.rstrip() + ell

    return lines


# Read from tiga-fresh-test\logs\renderer.log, 2026-10-15: the Morse band's dot and dash.
MORSE_LOGGED = "･－"
# Other Morse-like characters a title could use.
MORSE_LIKE = "·•∙−–—▬⸺"
MORSE_TITLE = "･－･･ ･ － ･･ － －－･ －－－"
HAVE_CJK = any((FONTS / f).exists() for f in ("YuGothB.ttc", "meiryo.ttc", "msyhbd.ttc"))


@unittest.skipUnless((FONTS / "segoeuib.ttf").exists(), "needs Windows fonts")
class FontFallbackCoverageTests(unittest.TestCase):
    """TASKS_D: every character is drawn with a font that has it."""

    def setUp(self):
        from PIL import Image, ImageDraw

        self.r = load_renderer()
        self.logged = []
        self.r.log = self.logged.append
        self.draw = ImageDraw.Draw(Image.new("RGB", (8, 8)))
        self.Image, self.ImageDraw = Image, ImageDraw

    def assert_every_char_has_a_font(self, text, size=18, bold=True):
        chain = self.r.find_text_font(size, bold)
        for ch in text:
            self.assertTrue(self.r.has_glyph(chain.font_for(ch), ch), f"U+{ord(ch):04X}")

    @unittest.skipUnless(HAVE_CJK, "no CJK font")
    def test_morse_code_points_from_the_log(self):
        self.assert_every_char_has_a_font(MORSE_LOGGED)
        self.assert_every_char_has_a_font(MORSE_LOGGED, 16, False)

    def test_morse_like_symbols(self):
        self.assert_every_char_has_a_font(MORSE_LIKE)

    @unittest.skipUnless(any((FONTS / f).exists() for f in ("YuGothB.ttc", "meiryo.ttc")), "no Japanese font")
    def test_japanese(self):
        self.assert_every_char_has_a_font("渚・モデラート")
        self.assert_every_char_has_a_font("トーキョーレギー", 18, False)

    @unittest.skipUnless((FONTS / "msyhbd.ttc").exists(), "no Chinese font")
    def test_chinese(self):
        self.assert_every_char_has_a_font("月亮代表我的心")

    @unittest.skipUnless((FONTS / "malgunbd.ttf").exists(), "no Korean font")
    def test_korean(self):
        self.assert_every_char_has_a_font("밤양갱")

    @unittest.skipUnless(HAVE_CJK, "no CJK font")
    def test_mixed_latin_and_japanese_uses_both_fonts_on_one_baseline(self):
        text = "高中正義 — Blue Lagoon"
        chain = self.r.find_text_font(18, True)
        self.assert_every_char_has_a_font(text)
        fonts = {id(f) for _, f in chain.runs(text)}
        self.assertIn(id(chain.primary), fonts)
        self.assertGreater(len(fonts), 1)

    def test_long_japanese_title_without_spaces_wraps_inside_the_box(self):
        chain = self.r.find_text_font(18, True)
        title = "ラブ・ストーリーは突然に東京ラブストーリー主題歌"
        lines = self.r.fit_text(self.draw, title, chain, 174, 3)
        self.assertGreater(len(lines), 1)
        for line in lines:
            self.assertLessEqual(self.r.text_width(self.draw, line, chain), 174, line)
        self.assertEqual("".join(lines), title)  # nothing lost, no spaces inserted
        for line in lines[1:]:
            self.assertNotIn(line[0], "ー・、。ッャュョ", line)

    def test_cjk_after_a_latin_word_fills_the_first_line(self):
        chain = self.r.find_text_font(18, True)
        lines = self.r.fit_text(self.draw, "ABC ラブ・ストーリーは突然に", chain, 174, 3)
        self.assertTrue(lines[0].startswith("ABC ラ"), lines)

    def test_morse_letters_are_not_split_across_lines(self):
        chain = self.r.find_text_font(18, True)
        lines = self.r.fit_text(self.draw, MORSE_TITLE, chain, 174, 3)
        self.assertEqual(" ".join(lines).split(), MORSE_TITLE.split())

    def test_missing_fallback_font_is_skipped(self):
        self.r.FALLBACK_FONT_FILES = [("no-such-font.ttc", "no-such-font-bold.ttc")] + self.r.FALLBACK_FONT_FILES
        chain = self.r.find_text_font(17, bold=True)
        self.assertGreater(len(chain.fonts), 1)
        self.assertEqual(len(self.logged), 1)
        self.assertIn("not installed: no-such-font.ttc", self.logged[0])
        self.r.find_text_font(17, bold=False)
        self.assertEqual(len(self.logged), 1)  # one line per process, not per font size

    def test_character_no_font_has_is_logged_once_and_drawn(self):
        chain = self.r.find_text_font(18, True)
        hieroglyph = "\U00013000"  # Egyptian; none of the fallback fonts has it
        self.assertIs(chain.font_for(hieroglyph), chain.primary)
        self.r.render_media({"title": hieroglyph * 3, "artist": "", "album": "", "thumbnail": False})
        self.assertEqual([m for m in self.logged if "U+13000" in m], [self.logged[-1]])

    def test_emoji_does_not_crash(self):
        img = self.r.render_media({"title": "Song 🎵😀", "artist": "👍", "album": "", "thumbnail": False})
        self.assertEqual(img.size, (self.r.WIDTH, self.r.HEIGHT))

    def test_latin_frame_is_pixel_identical_to_before_the_fallback(self):
        old = load_renderer()
        old.log = self.logged.append
        old.find_text_font = old.find_font
        old.fit_text = fit_text_before_fallback
        old.load_thumbnail = self.r.load_thumbnail  # the "♪" tile changed on purpose in 7c93850
        for media in (
            {"title": "Blue Lagoon", "artist": "Masayoshi Takanaka", "album": "Seychelles", "duration_ms": 245000},
            {"title": "Alte Sünder", "artist": "SALÒ", "album": "Rabatt", "duration_ms": 180000},
            {"title": "A Very Long Title That Has To Wrap Over Several Lines Because It Is Long",
             "artist": "Some Artist With A Long Name As Well", "album": "An Album", "source": "Spotify"},
            {"title": "Supercalifragilisticexpialidocious-and-more-unbroken-text", "artist": "X", "album": "Y"},
        ):
            self.assertEqual(self.r.render_media(dict(media)).tobytes(), old.render_media(dict(media)).tobytes(), media["title"])


if __name__ == "__main__":
    unittest.main()
