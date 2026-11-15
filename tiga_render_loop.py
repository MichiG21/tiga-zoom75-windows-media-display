"""The renderer's main loop: control commands, the freshness gate and idle handling."""
from __future__ import annotations

import time

from tiga_render_freshness import FreshnessGate, media_fingerprint

POLL_INTERVAL = 1.0
IDLE_TO_STANDBY_SECONDS = 60.0
# On a track change Spotify reports the new length about a poll before the new title
# (105 of 1,026 changes in renderer.log, 2026-10-15..11-15; the title followed 1-2 s
# later every time). Holding a length-only change this long skips that frame.
DURATION_HOLD_SECONDS = 2.5


def duration_change_action(same_otherwise: bool, old_ms: int, new_ms: int, held_for: float) -> str:
    """'render', 'hold' or 'ignore' for a media change. `same_otherwise`: only the
    length differs from the frame on screen; `held_for`: seconds this change has waited."""
    if not same_otherwise:
        return "render"
    if not new_ms:
        return "ignore"  # known -> unknown: browser videos flicker between the two
    if not old_ms:
        return "render"  # unknown -> known is shown at once (PROJECT_HISTORY.md:1805)
    return "hold" if held_for < DURATION_HOLD_SECONDS else "render"


def run(
    *,
    log,
    snapshot,
    write_display,
    write_command_ack,
    read_control_command,
    render_standby,
    render_black,
    render_media,
    media_key,
    print_media,
    command_file,
):
    """Runs until Ctrl+C. The renderer passes in everything that reads or writes a file."""
    write_display(render_standby("Waiting for fresh playback"), "startup standby")
    log("STARTUP -> standby rendered + installed")

    gate = FreshnessGate()

    try:
        gate.arm(snapshot())
    except Exception as exc:
        log(f"Initial media query warning: {exc}")

    mode = "auto"

    # Ignore a stale manual command left over from a previous Windows session.
    # The renderer always boots into automatic mode; the control utility must
    # write a new command after startup to override it.
    try:
        last_command_text = command_file.read_text(encoding="utf-8").strip().lower()
    except Exception:
        last_command_text = None

    active_fp = None
    active_render_key = None
    # The frame on screen: its key with the length blanked, its length, and since when a
    # length-only change has been held back (None: nothing held).
    active_base_key = None
    active_duration = 0
    held_since = None
    nonplaying_since = None
    query_error = None
    # An ack that a reader blocked: (command text, command, frame) and the tries so far.
    # Retried on every poll until it lands; a newer command's ack replaces it, because
    # the control utility only waits for the ack naming its own command text.
    pending_ack = None
    ack_tries = 0

    try:
        while True:
            command, last_command_text = read_control_command(last_command_text)
            if command is not None:
                if command == "standby":
                    mode = "standby"
                    frame = write_display(render_standby("Manual standby"), "control: standby")
                    active_fp = None
                    active_render_key = None
                    log("CONTROL -> manual standby")

                elif command == "black":
                    mode = "black"
                    frame = write_display(render_black(), "control: black")
                    active_fp = None
                    active_render_key = None
                    log("CONTROL -> black screen")

                elif command == "auto":
                    mode = "auto"
                    frame = write_display(render_standby("Waiting for fresh playback"), "control: auto")
                    active_fp = None
                    active_render_key = None
                    nonplaying_since = None

                    try:
                        gate.arm(snapshot())
                    except Exception:
                        gate.arm(None)

                    log("CONTROL -> automatic mode re-armed")

                pending_ack = (last_command_text, command, frame)
                ack_tries = 0

            if pending_ack is not None:
                ack_tries += 1
                if write_command_ack(*pending_ack):
                    if ack_tries > 1:
                        log(f"CONTROL ack for '{pending_ack[0]}' written after {ack_tries} tries")
                    pending_ack = None
                elif ack_tries == 1:
                    log(f"CONTROL ack for '{pending_ack[0]}' could not be written (another program "
                        "kept it open); retrying every poll")

            if mode != "auto":
                time.sleep(POLL_INTERVAL)
                continue

            try:
                media = snapshot()
                if query_error is not None:
                    log("Windows media query recovered")
                    query_error = None
            except Exception as exc:
                # Treat a failed query like "no media" so idle handling still applies;
                # it used to `continue` here and leave the last track up forever
                # (issue #10). Log each distinct error once, not every second.
                if str(exc) != query_error:
                    log(f"Windows media query/render warning: {exc}")
                    query_error = str(exc)
                media = {"present": False}

            now_mono = time.monotonic()

            if not media.get("present"):
                if active_fp is not None:
                    if nonplaying_since is None:
                        nonplaying_since = now_mono
                    elif now_mono - nonplaying_since >= IDLE_TO_STANDBY_SECONDS:
                        write_display(render_standby("No media playing"), "idle: no media")
                        log("IDLE -> standby rendered + installed")
                        active_fp = None
                        active_render_key = None
                        gate.arm(None)
                else:
                    gate.interrupt()
                time.sleep(POLL_INTERVAL)
                continue

            state = str(media.get("state", ""))
            fp = media_fingerprint(media)

            if state != "Playing":
                if active_fp is not None:
                    if nonplaying_since is None:
                        nonplaying_since = now_mono
                    elif now_mono - nonplaying_since >= IDLE_TO_STANDBY_SECONDS:
                        write_display(render_standby("Playback stopped"), "idle: playback stopped")
                        log("IDLE -> standby rendered + installed")
                        active_fp = None
                        active_render_key = None
                        gate.arm(media)
                else:
                    gate.observe(media, now_mono)

                time.sleep(POLL_INTERVAL)
                continue

            nonplaying_since = None

            if active_fp is not None:
                key = media_key(media)
                if key == active_render_key:
                    held_since = None
                else:
                    base_key = media_key({**media, "duration_ms": 0})
                    new_duration = int(media.get("duration_ms") or 0)
                    action = duration_change_action(
                        base_key == active_base_key, active_duration, new_duration,
                        0.0 if held_since is None else now_mono - held_since,
                    )
                    if action == "hold" and held_since is None:
                        held_since = now_mono
                        log("Length-only change held (Spotify sends the new length before the new title)")
                    elif action == "render":
                        img = render_media(media)
                        write_display(img, "media changed")

                        log("NEW WINDOWS MEDIA -> rendered + installed")
                        print_media(media)

                        active_fp = fp
                        active_render_key = key
                        active_base_key = base_key
                        active_duration = new_duration
                        held_since = None

                time.sleep(POLL_INTERVAL)
                continue

            fresh_reason = gate.observe(media, now_mono)

            if fresh_reason:
                img = render_media(media)
                write_display(img, f"fresh playback: {fresh_reason}")
                active_fp = fp
                active_render_key = media_key(media)
                active_base_key = media_key({**media, "duration_ms": 0})
                active_duration = int(media.get("duration_ms") or 0)
                held_since = None

                log(f"FRESH PLAYBACK ({fresh_reason}) -> rendered + installed")
                print_media(media)

            time.sleep(POLL_INTERVAL)

    except KeyboardInterrupt:
        print("\nStopped Windows Now Playing renderer.")
