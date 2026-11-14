#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import subprocess
import sys
import time  # noqa: F401  (the tests replace time.sleep and time.monotonic through this name)
from datetime import datetime
from pathlib import Path

from PIL import Image, ImageDraw, ImageOps

import tiga_dlx
import tiga_render_loop
import tiga_render_stream

# Paths, flags and the frame counter stay in this file, and the modules get them passed
# in: --dlx-frames and the like set them here, and PyInstaller's HERE must come from this
# file. Names unused below are for tests/test_renderer.py and tests/test_dlx.py.
from tiga_render_freshness import STEADY_PLAYING_RELEASE_SECONDS, FreshnessGate  # noqa: F401
from tiga_render_loop import IDLE_TO_STANDBY_SECONDS
from tiga_render_stream import HEIGHT, WIDTH, stream_crc32, validate_stream  # noqa: F401
from tiga_render_text import (  # noqa: F401
    ALBUM_STEP,
    ARTIST_STEP,
    DIVIDER_Y,
    FALLBACK_FONT_FILES,
    MORSE_STEP,
    TITLE_STEP,
    FontLibrary,
    decode_morse,
    draw_text,
    find_font,
    fit_text,
    layout_text_block,
    text_width,
)

HERE = Path(__file__).resolve().parent
EXTRACTED = HERE / "extracted"
ACTIVE_STREAM = EXTRACTED / "session_01_bulk_stream.bin"
BACKUP_STREAM = EXTRACTED / "session_01_bulk_stream.redgreen-backup.bin"
OUTPUT_STREAM = EXTRACTED / "session_01_bulk_stream.windows_nowplaying.bin"
PREVIEW = HERE / "windows_nowplaying_preview.png"

SNAPSHOT_EXE = HERE / "tiga_windows_media_snapshot.exe"
THUMBNAIL_FILE = HERE / "windows_media_thumbnail.bin"

COMMAND_FILE = HERE / "tiga_display_command.txt"
# Written after a command's frame is installed; the control utility waits for it
# before "Standby + Shut Down" (issue #7).
COMMAND_ACK_FILE = HERE / "tiga_display_command.ack"
COVER_SIZE = 122

# Experimental (PROJECT_HISTORY.md section 3.4): a single-frame DLX container instead
# of the captured 2-frame one. Never sent to a keyboard yet; enable only with
# --dlx-frames 1, and the uploader must also be given --allow-single-frame.
DLX_FRAMES = 2
DLX_LAYOUT = "index"

# The decoded line under a field written entirely in Morse code. Off: --no-morse-subtitle.
MORSE_SUBTITLE = True

# Frame correlation (issue #9). The sidecar is written before the stream is replaced,
# so the uploader always finds a sidecar at least as new as the stream it reads. The
# CRC-32 of the full stream is the join key between renderer.log and ble.log.
FRAME_SIDECAR = ACTIVE_STREAM.with_name(ACTIVE_STREAM.name + ".frame")
RUN_ID = os.environ.get("TIGA_RUN_ID") or f"renderer-{datetime.now():%Y%m%d-%H%M%S}-{os.getpid()}"
_frame_seq = 0


def log(message: str):
    stamp = datetime.now().astimezone().isoformat(timespec="seconds")
    print(f"{stamp} {message}", flush=True)


def next_frame_id() -> str:
    global _frame_seq
    _frame_seq += 1
    return f"{RUN_ID}-f{_frame_seq:04d}"


# The lambdas look `log` and FALLBACK_FONT_FILES up when they are needed, not now.
_fonts = FontLibrary(log=lambda message: log(message), fallback_files=lambda: FALLBACK_FONT_FILES)
has_glyph = _fonts.has_glyph
find_text_font = _fonts.find_text_font


def format_duration_ms(duration_ms: int) -> str:
    try:
        total_seconds = max(0, int(duration_ms) // 1000)
    except (TypeError, ValueError):
        return ""

    if total_seconds <= 0:
        return ""

    hours, rem = divmod(total_seconds, 3600)
    minutes, seconds = divmod(rem, 60)

    if hours:
        return f"{hours}:{minutes:02d}:{seconds:02d}"
    return f"{minutes}:{seconds:02d}"


def source_name(source: str) -> str:
    s = (source or "").lower()
    if "spotify" in s:
        return "Spotify"
    if "firefox" in s:
        return "Firefox"
    if "chrome" in s:
        return "Chrome"
    if "msedge" in s or "edge" in s:
        return "Edge"
    if "vlc" in s:
        return "VLC"
    if "musicbee" in s:
        return "MusicBee"
    if not source:
        return "Media"

    raw = source.split("!")[-1]
    raw = Path(raw).stem

    # Some browsers expose an opaque hex-ish session ID instead of a friendly
    # application name. Never print that ugly identifier on the keyboard.
    compact = raw.replace("-", "").replace("_", "")
    if compact and all(c in "0123456789abcdefABCDEF" for c in compact):
        return "Browser Media"

    return raw[:24] or "Media"


def load_thumbnail(has_thumbnail: bool) -> Image.Image:
    if has_thumbnail and THUMBNAIL_FILE.exists():
        try:
            data = THUMBNAIL_FILE.read_bytes()
            img = Image.open(io.BytesIO(data)).convert("RGB")
            return ImageOps.fit(img, (COVER_SIZE, COVER_SIZE), method=Image.Resampling.LANCZOS)
        except Exception:
            pass

    # Neutral fallback tile.
    img = Image.new("RGB", (COVER_SIZE, COVER_SIZE), (35, 35, 40))
    draw = ImageDraw.Draw(img)
    # Segoe UI Bold has no "♪" (it drew a missing-glyph box); take it from a fallback.
    text = "♪"
    font = find_text_font(42, bold=True).font_for(text)
    bbox = draw.textbbox((0, 0), text, font=font)
    x = (COVER_SIZE - (bbox[2] - bbox[0])) // 2 - bbox[0]
    y = (COVER_SIZE - (bbox[3] - bbox[1])) // 2 - bbox[1]
    draw.text((x, y), text, font=font, fill=(190, 190, 200))
    return img


def render_standby(reason: str = "Waiting for media") -> Image.Image:
    """Render a calm standby screen.

    The clock/date are a snapshot from when standby is entered. We intentionally
    do not refresh it every minute, because that would force another full BLE
    image upload every minute.
    """
    bg = (10, 10, 13)
    canvas = Image.new("RGB", (WIDTH, HEIGHT), bg)
    draw = ImageDraw.Draw(canvas)

    now = datetime.now()

    clock_font = find_font(48, bold=True)
    date_font = find_font(18, bold=False)
    label_font = find_font(13, bold=True)
    reason_font = find_font(14, bold=False)

    clock = now.strftime("%H:%M")
    date_text = now.strftime("%A  •  %d.%m.%Y")

    bbox = draw.textbbox((0, 0), clock, font=clock_font)
    draw.text(
        ((WIDTH - (bbox[2] - bbox[0])) // 2, 23),
        clock,
        font=clock_font,
        fill=(238, 238, 242),
    )

    bbox = draw.textbbox((0, 0), date_text, font=date_font)
    draw.text(
        ((WIDTH - (bbox[2] - bbox[0])) // 2, 83),
        date_text,
        font=date_font,
        fill=(184, 184, 194),
    )

    draw.line((24, 119, WIDTH - 24, 119), fill=(48, 48, 56), width=1)

    label = "TIGA MEDIA DISPLAY"
    bbox = draw.textbbox((0, 0), label, font=label_font)
    draw.text(
        ((WIDTH - (bbox[2] - bbox[0])) // 2, 128),
        label,
        font=label_font,
        fill=(206, 206, 214),
    )

    bbox = draw.textbbox((0, 0), reason, font=reason_font)
    draw.text(
        ((WIDTH - (bbox[2] - bbox[0])) // 2, 148),
        reason,
        font=reason_font,
        fill=(142, 142, 153),
    )

    return canvas


def render_black() -> Image.Image:
    return Image.new("RGB", (WIDTH, HEIGHT), (0, 0, 0))


def install_image(img: Image.Image, frame_id: str, reason: str) -> str:
    return tiga_render_stream.install_image(
        img,
        frame_id,
        reason,
        backup_stream=BACKUP_STREAM,
        output_stream=OUTPUT_STREAM,
        active_stream=ACTIVE_STREAM,
        frame_sidecar=FRAME_SIDECAR,
        dlx_frames=DLX_FRAMES,
        dlx_layout=DLX_LAYOUT,
    )


def write_display(img: Image.Image, reason: str, preview: bool = True) -> tuple[str, str]:
    """Render, install, and return (frame_id, crc32) of the installed stream."""
    frame_id = next_frame_id()
    log(f"FRAME {frame_id} rendered ({reason})")
    if preview:
        img.save(PREVIEW)
    crc = install_image(img, frame_id, reason)
    log(f"FRAME {frame_id} installed crc32={crc}")
    return frame_id, crc


def write_command_ack(command_text: str, command: str, frame: tuple[str, str]) -> bool:
    """Tell the control utility which frame (by CRC) answers its command.

    It then waits for <stream>.delivered to show the same CRC, which the uploader
    writes only after the keyboard's completion notification.

    Returns False if a reader kept the ack open through every retry; the main loop
    then tries again on its next poll. Raising here used to crash the renderer, and
    at restart it treats the command file as stale, so the command was lost.
    """
    frame_id, crc = frame
    text = (
        f"command_text={command_text}\n"
        f"command={command}\n"
        f"frame_id={frame_id}\n"
        f"crc32={crc}\n"
        f"handled_at={datetime.now().astimezone().isoformat(timespec='seconds')}\n"
    )
    tmp = COMMAND_ACK_FILE.with_suffix(".ack.tmp")
    tmp.write_text(text, encoding="utf-8")
    return tiga_render_stream.replace_with_retry(tmp, COMMAND_ACK_FILE)


def read_control_command(last_text: str | None):
    """Return (command, text_to_remember).

    The control utility writes "<command> <unique id>", so pressing the same button
    twice still changes the file (issue #6). The bare form "<command>" written by older
    builds is still accepted, with its old limitation.
    """
    try:
        raw = COMMAND_FILE.read_text(encoding="utf-8").strip().lower()
    except Exception:
        return None, last_text

    if not raw or raw == last_text:
        return None, last_text

    command = raw.split()[0]
    if command in {"auto", "standby", "black"}:
        return command, raw

    return None, raw


def render_media(media: dict) -> Image.Image:
    bg = (12, 12, 14)
    canvas = Image.new("RGB", (WIDTH, HEIGHT), bg)
    draw = ImageDraw.Draw(canvas)

    cover = load_thumbnail(bool(media.get("thumbnail")))
    canvas.paste(cover, (8, 7))

    title_font = find_text_font(18, bold=True)
    artist_font = find_text_font(18, bold=False)
    album_font = find_text_font(16, bold=False)
    morse_font = find_text_font(14, bold=True) if MORSE_SUBTITLE else None
    label_font = find_font(12, bold=True)
    duration_font = find_font(13, bold=True)

    text_x = 138
    max_text_w = 174

    title = media.get("title") or "Unknown media"

    # For Spotify this is usually the artist.
    # For browser media we gracefully fall back to Subtitle or source application.
    secondary = (
        media.get("artist")
        or media.get("subtitle")
        or media.get("album_artist")
        or source_name(media.get("source", ""))
    )

    tertiary = media.get("album") or source_name(media.get("source", ""))

    for y, line, font, fill in layout_text_block(
        draw, title, secondary, tertiary, title_font, artist_font, album_font, max_text_w, morse_font, fit_text
    ):
        draw_text(draw, (text_x, y), line, font, fill)

    draw.line((8, DIVIDER_Y, 312, DIVIDER_Y), fill=(45, 45, 50), width=1)
    draw.text((8, 146), "NOW PLAYING", font=label_font, fill=(210, 210, 216))

    duration = format_duration_ms(media.get("duration_ms", 0))

    if duration:
        bbox = draw.textbbox((0, 0), duration, font=duration_font)
        draw.text(
            (312 - (bbox[2] - bbox[0]), 145),
            duration,
            font=duration_font,
            fill=(190, 190, 198),
        )
    else:
        # Live streams / sessions without a timeline fall back to the source label.
        src = source_name(media.get("source", ""))
        if src == "Browser Media" and not media.get("album"):
            src = "Web Media"
        fallback_font = find_font(12, bold=False)
        bbox = draw.textbbox((0, 0), src, font=fallback_font)
        draw.text(
            (312 - (bbox[2] - bbox[0]), 146),
            src,
            font=fallback_font,
            fill=(170, 170, 178),
        )

    return canvas


def snapshot() -> dict:
    if not SNAPSHOT_EXE.exists():
        raise RuntimeError(
            f"Missing {SNAPSHOT_EXE.name}. Compile the C++ snapshot helper first."
        )

    result = subprocess.run(
        [str(SNAPSHOT_EXE), "--thumbnail", str(THUMBNAIL_FILE)],
        cwd=str(HERE),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=10,
    )

    stdout = result.stdout.strip()
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or stdout or f"snapshot helper exited {result.returncode}")

    if not stdout:
        raise RuntimeError("snapshot helper returned no JSON")

    return json.loads(stdout)


def thumbnail_digest(media: dict) -> str:
    # Hash the artwork itself: different artwork of identical byte size used to be
    # treated as unchanged (issue #10). The snapshot helper rewrites the file on
    # every query, so it always belongs to this snapshot when "thumbnail" is true.
    if not media.get("thumbnail"):
        return ""
    try:
        return hashlib.sha256(THUMBNAIL_FILE.read_bytes()).hexdigest()
    except OSError:
        return f"unreadable:{media.get('thumbnail_bytes', 0)}"


def media_key(media: dict) -> str:
    # Deliberately ignore play/pause state so pausing does not trigger a full image upload.
    fields = [
        media.get("source", ""),
        media.get("title", ""),
        media.get("artist", ""),
        media.get("album_artist", ""),
        media.get("album", ""),
        media.get("subtitle", ""),
        str(media.get("track_number", 0)),
        str(media.get("duration_ms", 0)),
        thumbnail_digest(media),
    ]
    return hashlib.sha256("\0".join(fields).encode("utf-8", "replace")).hexdigest()


def print_media(media: dict):
    print(f"Source   : {source_name(media.get('source', ''))}")
    print(f"Title    : {media.get('title') or '(empty)'}")
    print(f"Artist   : {media.get('artist') or '(empty)'}")
    print(f"Album    : {media.get('album') or '(empty)'}")
    print(f"Duration : {format_duration_ms(media.get('duration_ms', 0)) or '(unknown)'}")
    print(f"Thumbnail: {'YES' if media.get('thumbnail') else 'NO'}")


def configure_output():
    # Redirected output defaults to the ANSI code page and block buffering; a title the
    # code page cannot encode would crash the renderer, and an abrupt exit would lose
    # the tail of the log.
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace", line_buffering=True)
        except (AttributeError, ValueError):
            pass


def main():
    configure_output()
    print()
    print("=" * 72)
    print("TIGA - WINDOWS NOW PLAYING RENDERER + STANDBY")
    print("=" * 72)
    print("Startup     : always shows standby first")
    print("Fresh gate  : ignores stale startup media until playback is proven fresh")
    print(f"Idle timeout: {int(IDLE_TO_STANDBY_SECONDS)}s paused/stopped -> standby")
    print(f"Control file: {COMMAND_FILE}")
    print(f"Preview     : {PREVIEW}")
    print(f"Stream      : {ACTIVE_STREAM}")
    print(f"Run ID      : {RUN_ID}")
    if DLX_FRAMES != 2:
        print(f"DLX         : {DLX_FRAMES}-frame container, layout={DLX_LAYOUT} (trial 2026-11-11, docs/DLX_FORMAT.md)")
    print("Ctrl+C to stop.")
    print()

    tiga_render_loop.run(
        log=log,
        snapshot=snapshot,
        write_display=write_display,
        write_command_ack=write_command_ack,
        read_control_command=read_control_command,
        render_standby=render_standby,
        render_black=render_black,
        render_media=render_media,
        media_key=media_key,
        print_media=print_media,
        command_file=COMMAND_FILE,
    )


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="TIGA Windows Now Playing renderer")
    parser.add_argument(
        "--dlx-frames", type=int, choices=(1, 2), default=2,
        help="frames per DLX container (default 2, the captured layout; 1 halves the upload, "
             "passed its hardware trial on 2026-11-11, and needs the uploader's --allow-single-frame; "
             "the supervisors set it from $DlxFrames)",
    )
    parser.add_argument(
        "--dlx-layout", choices=tiga_dlx.LAYOUTS, default="index",
        help="frame-index interpretation for --dlx-frames 1 (see docs/DLX_FORMAT.md)",
    )
    parser.add_argument(
        "--no-morse-subtitle", dest="morse_subtitle", action="store_false",
        help="do not add a decoded line under titles written entirely in Morse code",
    )
    return parser.parse_args(argv)


if __name__ == "__main__":
    _args = parse_args(sys.argv[1:])
    DLX_FRAMES = _args.dlx_frames
    DLX_LAYOUT = _args.dlx_layout
    MORSE_SUBTITLE = _args.morse_subtitle
    main()
