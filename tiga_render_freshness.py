"""The freshness gate: stays on standby until playback is proven fresh."""
from __future__ import annotations

import hashlib

FRESH_POSITION_ADVANCE_MS = 1500
# Some providers report Playing with a position that never moves. Without this escape,
# media that was already playing at startup stayed on standby indefinitely (issue #5).
STEADY_PLAYING_RELEASE_SECONDS = 15.0


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


class FreshnessGate:
    """Holds standby until playback is proven fresh.

    Windows keeps remembered media sessions around after playback stops or across a
    reboot. The gate records what was there when standby was entered and releases
    only on evidence of real playback:

    - a different media item,
    - a transition from a non-playing state to Playing,
    - the timeline advancing by FRESH_POSITION_ADVANCE_MS, or
    - the same valid media reporting Playing continuously for
      STEADY_PLAYING_RELEASE_SECONDS (for providers whose position never moves).

    A remembered session that reports Paused/Stopped, or Playing without a title,
    stays on standby.
    """

    def __init__(self, media: dict | None = None):
        self.arm(media)

    def arm(self, media: dict | None):
        if media and media.get("present"):
            self.fp = media_fingerprint(media)
            self.position = int(media.get("position_ms", 0) or 0)
            self.state = str(media.get("state", ""))
        else:
            self.fp = None
            self.position = None
            self.state = None
        self.interrupt()

    def interrupt(self):
        self.steady_fp = None
        self.steady_since = None

    def observe(self, media: dict, now: float) -> str | None:
        """Feed one snapshot taken while on standby; returns a release reason or None."""
        state = str(media.get("state", ""))
        fp = media_fingerprint(media)
        position = int(media.get("position_ms", 0) or 0)

        if state != "Playing":
            self.fp = fp
            self.position = position
            self.state = state
            self.interrupt()
            return None

        if self.fp is None or fp != self.fp:
            return "new media"

        if self.state is not None and self.state != "Playing":
            return "playback started"

        if self.position is not None and position > 0:
            if position - int(self.position or 0) >= FRESH_POSITION_ADVANCE_MS:
                return "timeline advancing"

        if (media.get("title") or "").strip():
            if self.steady_fp != fp:
                self.steady_fp = fp
                self.steady_since = now
            elif now - self.steady_since >= STEADY_PLAYING_RELEASE_SECONDS:
                return "steady playback"
        else:
            self.interrupt()

        if self.position is None:
            self.position = position
        self.state = state
        return None
