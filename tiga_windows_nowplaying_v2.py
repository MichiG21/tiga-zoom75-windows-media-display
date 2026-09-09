#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import io
import json
import os
import subprocess
import time
from datetime import datetime
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont, ImageOps

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

HERE = Path(__file__).resolve().parent
EXTRACTED = HERE / "extracted"
ACTIVE_STREAM = EXTRACTED / "session_01_bulk_stream.bin"
BACKUP_STREAM = EXTRACTED / "session_01_bulk_stream.redgreen-backup.bin"
OUTPUT_STREAM = EXTRACTED / "session_01_bulk_stream.windows_nowplaying.bin"
PREVIEW = HERE / "windows_nowplaying_preview.png"

SNAPSHOT_EXE = HERE / "tiga_windows_media_snapshot.exe"
THUMBNAIL_FILE = HERE / "windows_media_thumbnail.bin"

POLL_INTERVAL = 1.0
IDLE_TO_STANDBY_SECONDS = 60.0
FRESH_POSITION_ADVANCE_MS = 1500
COMMAND_FILE = HERE / "tiga_display_command.txt"
COVER_SIZE = 122


def find_font(size: int, bold: bool = False):
    candidates = [
        Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts" / ("segoeuib.ttf" if bold else "segoeui.ttf"),
        Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts" / ("arialbd.ttf" if bold else "arial.ttf"),
    ]
    for p in candidates:
        if p.exists():
            return ImageFont.truetype(str(p), size=size)
    return ImageFont.load_default()


def fit_text(draw: ImageDraw.ImageDraw, text: str, font, max_width: int, max_lines: int):
    words = (text or "").split()
    if not words:
        return [""]

    lines = []
    current = ""

    for word in words:
        test = word if not current else current + " " + word
        if draw.textbbox((0, 0), test, font=font)[2] <= max_width:
            current = test
        else:
            if current:
                lines.append(current)
            current = word
            if len(lines) >= max_lines:
                break

    if len(lines) < max_lines and current:
        lines.append(current)

    lines = lines[:max_lines]
    shown = " ".join(lines)

    if shown != text and lines:
        last = lines[-1]
        ell = "…"
        while last and draw.textbbox((0, 0), last + ell, font=font)[2] > max_width:
            last = last[:-1]
        lines[-1] = last.rstrip() + ell

    return lines


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
    font = find_font(42, bold=True)
    text = "♪"
    bbox = draw.textbbox((0, 0), text, font=font)
    x = (COVER_SIZE - (bbox[2] - bbox[0])) // 2
    y = (COVER_SIZE - (bbox[3] - bbox[1])) // 2 - 4
    draw.text((x, y), text, font=font, fill=(190, 190, 200))
    return img


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


def media_fingerprint(media: dict) -> str:
    fields = [
        media.get("source", ""),
        media.get("title", ""),
        media.get("artist", ""),
        media.get("album_artist", ""),
        media.get("album", ""),
        media.get("subtitle", ""),
        str(media.get("track_number", 0)),
    ]
    return hashlib.sha256("\0".join(fields).encode("utf-8", "replace")).hexdigest()


def write_display(img: Image.Image, preview: bool = True):
    if preview:
        img.save(PREVIEW)
    install_image(img)


def read_control_command(last_text: str | None):
    try:
        raw = COMMAND_FILE.read_text(encoding="utf-8").strip().lower()
    except FileNotFoundError:
        return None, last_text
    except Exception:
        return None, last_text

    if not raw or raw == last_text:
        return None, last_text

    if raw in {"auto", "standby", "black"}:
        return raw, raw

    return None, raw


def render_media(media: dict) -> Image.Image:
    bg = (12, 12, 14)
    canvas = Image.new("RGB", (WIDTH, HEIGHT), bg)
    draw = ImageDraw.Draw(canvas)

    cover = load_thumbnail(bool(media.get("thumbnail")))
    canvas.paste(cover, (8, 7))

    title_font = find_font(18, bold=True)
    artist_font = find_font(18, bold=False)
    album_font = find_font(16, bold=False)
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

    title_lines = fit_text(draw, title, title_font, max_text_w, 3)
    y = 9
    for line in title_lines:
        draw.text((text_x, y), line, font=title_font, fill=(242, 242, 245))
        y += 20

    artist_lines = fit_text(draw, secondary, artist_font, max_text_w, 2)
    y = max(y + 4, 72)
    for line in artist_lines:
        draw.text((text_x, y), line, font=artist_font, fill=(224, 224, 230))
        y += 21

    if tertiary:
        album_lines = fit_text(draw, tertiary, album_font, max_text_w, 2)
        album_y = min(y + 1, 110)
        for line in album_lines:
            draw.text((text_x, album_y), line, font=album_font, fill=(180, 180, 188))
            album_y += 17

    draw.line((8, 135, 312, 135), fill=(45, 45, 50), width=1)
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


def install_image(img: Image.Image):
    if not BACKUP_STREAM.exists():
        raise RuntimeError(f"Known-good stream backup missing: {BACKUP_STREAM}")

    base = BACKUP_STREAM.read_bytes()
    validate_stream(base)

    frame = rgb565_be_bytes(img)
    modified = bytearray(base)
    modified[FRAME1_START:FRAME1_START + FRAME_BYTES] = frame
    modified[FRAME2_START:FRAME2_START + FRAME_BYTES] = frame
    modified = bytes(modified)
    validate_stream(modified)

    OUTPUT_STREAM.write_bytes(modified)

    tmp = ACTIVE_STREAM.with_suffix(".bin.tmp")
    tmp.write_bytes(modified)
    os.replace(tmp, ACTIVE_STREAM)


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


def media_key(media: dict) -> str:
    # Deliberately ignore play/pause state so pausing does not trigger a ~7.7 s image upload.
    fields = [
        media.get("source", ""),
        media.get("title", ""),
        media.get("artist", ""),
        media.get("album_artist", ""),
        media.get("album", ""),
        media.get("subtitle", ""),
        str(media.get("track_number", 0)),
        str(media.get("duration_ms", 0)),
        str(media.get("thumbnail_bytes", 0)),
    ]
    return hashlib.sha256("\0".join(fields).encode("utf-8", "replace")).hexdigest()


def main():
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
    print("Ctrl+C to stop.")
    print()

    write_display(render_standby("Waiting for fresh playback"))
    print("STARTUP -> standby rendered + installed")

    startup_fp = None
    startup_position = None
    startup_state = None

    try:
        startup_media = snapshot()
        if startup_media.get("present"):
            startup_fp = media_fingerprint(startup_media)
            startup_position = int(startup_media.get("position_ms", 0) or 0)
            startup_state = str(startup_media.get("state", ""))
    except Exception as exc:
        print(f"Initial media query warning: {exc}")

    mode = "auto"

    # Ignore a stale manual command left over from a previous Windows session.
    # The renderer always boots into automatic mode; the control utility must
    # write a new command after startup to override it.
    try:
        last_command_text = COMMAND_FILE.read_text(encoding="utf-8").strip().lower()
    except Exception:
        last_command_text = None

    active_fp = None
    active_render_key = None
    nonplaying_since = None

    gate_fp = startup_fp
    gate_position = startup_position
    gate_state = startup_state

    try:
        while True:
            command, last_command_text = read_control_command(last_command_text)
            if command is not None:
                if command == "standby":
                    mode = "standby"
                    write_display(render_standby("Manual standby"))
                    active_fp = None
                    active_render_key = None
                    print("\nCONTROL -> manual standby")

                elif command == "black":
                    mode = "black"
                    write_display(render_black())
                    active_fp = None
                    active_render_key = None
                    print("\nCONTROL -> black screen")

                elif command == "auto":
                    mode = "auto"
                    write_display(render_standby("Waiting for fresh playback"))
                    active_fp = None
                    active_render_key = None
                    nonplaying_since = None

                    try:
                        current = snapshot()
                        if current.get("present"):
                            gate_fp = media_fingerprint(current)
                            gate_position = int(current.get("position_ms", 0) or 0)
                            gate_state = str(current.get("state", ""))
                        else:
                            gate_fp = None
                            gate_position = None
                            gate_state = None
                    except Exception:
                        gate_fp = None
                        gate_position = None
                        gate_state = None

                    print("\nCONTROL -> automatic mode re-armed")

            if mode != "auto":
                time.sleep(POLL_INTERVAL)
                continue

            try:
                media = snapshot()
            except Exception as exc:
                print(f"\nWindows media query/render warning: {exc}")
                time.sleep(POLL_INTERVAL)
                continue

            now_mono = time.monotonic()

            if not media.get("present"):
                if active_fp is not None:
                    if nonplaying_since is None:
                        nonplaying_since = now_mono
                    elif now_mono - nonplaying_since >= IDLE_TO_STANDBY_SECONDS:
                        write_display(render_standby("No media playing"))
                        print("\nIDLE -> standby rendered + installed")
                        active_fp = None
                        active_render_key = None
                        gate_fp = None
                        gate_position = None
                        gate_state = None
                time.sleep(POLL_INTERVAL)
                continue

            state = str(media.get("state", ""))
            fp = media_fingerprint(media)
            position = int(media.get("position_ms", 0) or 0)

            if state != "Playing":
                if active_fp is not None:
                    if nonplaying_since is None:
                        nonplaying_since = now_mono
                    elif now_mono - nonplaying_since >= IDLE_TO_STANDBY_SECONDS:
                        write_display(render_standby("Playback stopped"))
                        print("\nIDLE -> standby rendered + installed")
                        active_fp = None
                        active_render_key = None
                        gate_fp = fp
                        gate_position = position
                        gate_state = state
                else:
                    gate_fp = fp
                    gate_position = position
                    gate_state = state

                time.sleep(POLL_INTERVAL)
                continue

            nonplaying_since = None

            if active_fp is not None:
                key = media_key(media)
                if key != active_render_key:
                    img = render_media(media)
                    write_display(img)

                    print()
                    print("NEW WINDOWS MEDIA -> rendered + installed")
                    print(f"Source   : {source_name(media.get('source', ''))}")
                    print(f"Title    : {media.get('title') or '(empty)'}")
                    print(f"Artist   : {media.get('artist') or '(empty)'}")
                    print(f"Album    : {media.get('album') or '(empty)'}")
                    print(f"Duration : {format_duration_ms(media.get('duration_ms', 0)) or '(unknown)'}")
                    print(f"Thumbnail: {'YES' if media.get('thumbnail') else 'NO'}")

                    active_fp = fp
                    active_render_key = key

                time.sleep(POLL_INTERVAL)
                continue

            fresh = False
            fresh_reason = ""

            if gate_fp is None or fp != gate_fp:
                fresh = True
                fresh_reason = "new media"

            elif gate_state is not None and gate_state != "Playing":
                fresh = True
                fresh_reason = "playback started"

            elif gate_position is not None and position > 0:
                if position - int(gate_position or 0) >= FRESH_POSITION_ADVANCE_MS:
                    fresh = True
                    fresh_reason = "timeline advancing"

            if fresh:
                img = render_media(media)
                write_display(img)
                active_fp = fp
                active_render_key = media_key(media)

                print()
                print(f"FRESH PLAYBACK ({fresh_reason}) -> rendered + installed")
                print(f"Source   : {source_name(media.get('source', ''))}")
                print(f"Title    : {media.get('title') or '(empty)'}")
                print(f"Artist   : {media.get('artist') or '(empty)'}")
                print(f"Album    : {media.get('album') or '(empty)'}")
                print(f"Duration : {format_duration_ms(media.get('duration_ms', 0)) or '(unknown)'}")
                print(f"Thumbnail: {'YES' if media.get('thumbnail') else 'NO'}")
            else:
                if gate_position is None:
                    gate_position = position
                gate_state = state

            time.sleep(POLL_INTERVAL)

    except KeyboardInterrupt:
        print("\nStopped Windows Now Playing renderer.")


if __name__ == "__main__":
    main()
