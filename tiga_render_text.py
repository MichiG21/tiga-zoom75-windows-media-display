"""Fonts and text layout for the renderer: per-character font fallback, wrapping, Morse."""
from __future__ import annotations

import os
import re
import unicodedata
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def find_font(size: int, bold: bool = False):
    candidates = [
        Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts" / ("segoeuib.ttf" if bold else "segoeui.ttf"),
        Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts" / ("arialbd.ttf" if bold else "arial.ttf"),
    ]
    for p in candidates:
        if p.exists():
            return ImageFont.truetype(str(p), size=size)
    return ImageFont.load_default()


# Segoe UI has no CJK or Hangul glyphs and no "♪", and Pillow does no font fallback of
# its own: a Japanese title rendered as empty boxes on hardware. Text that comes from
# media metadata is therefore drawn with a chain of fonts, one character at a time.
FALLBACK_FONT_FILES = [
    # (regular, bold); the first that exists is used for each slot
    ("YuGothR.ttc", "YuGothB.ttc"),    # Japanese
    ("meiryo.ttc", "meiryob.ttc"),     # Japanese, older Windows
    ("msyh.ttc", "msyhbd.ttc"),        # Chinese (Microsoft YaHei)
    ("msjh.ttc", "msjhbd.ttc"),        # Traditional Chinese (Microsoft JhengHei)
    ("malgun.ttf", "malgunbd.ttf"),    # Korean
    ("seguisym.ttf", "seguisym.ttf"),  # symbols, including ♪
]
_NOTDEF_PROBE = "\U0010fffd"  # private use, plane 16: no font here maps it


def _glyph_mask(font, ch: str) -> bytes:
    size = getattr(font, "size", 16) * 2
    img = Image.new("L", (size, size))
    ImageDraw.Draw(img).text((0, 0), ch, font=font, fill=255)
    return img.tobytes()


class FontChain:
    """A primary font plus fallbacks, chosen per character."""

    def __init__(self, fonts: list, library: FontLibrary):
        self.fonts = fonts
        self.primary = fonts[0]
        self.size = getattr(self.primary, "size", 16)
        self.library = library

    def font_for(self, ch: str):
        for font in self.fonts:
            if self.library.has_glyph(font, ch):
                return font
        if ch not in self.library.missing_logged:
            self.library.missing_logged.add(ch)
            name = unicodedata.name(ch, "unnamed")
            self.library.log(f"FONT no font has U+{ord(ch):04X} {name}; drawn as a box")
        return self.primary

    def runs(self, text: str) -> list[tuple[str, object]]:
        """Split `text` into (substring, font) runs."""
        out: list[tuple[str, object]] = []
        for ch in text:
            font = self.font_for(ch)
            if out and out[-1][1] is font:
                out[-1] = (out[-1][0] + ch, font)
            else:
                out.append((ch, font))
        return out


class FontLibrary:
    """The font chains and what they have learned so far, one per renderer.

    `log` takes a message and `fallback_files` returns the list of font files; both are
    called when needed, so they always see the renderer's current ones.
    """

    def __init__(self, log, fallback_files):
        self.log = log
        self.fallback_files = fallback_files
        self.glyph_cache: dict[tuple[int, str], bool] = {}
        self.fonts_logged = False
        self.missing_logged: set[str] = set()
        self.chains: dict[tuple[int, bool], FontChain] = {}

    def has_glyph(self, font, ch: str) -> bool:
        """Whether `font` draws `ch` as something other than its missing-glyph box."""
        if ch.isspace() or not ch.isprintable():
            return True
        key = (id(font), ch)
        if key not in self.glyph_cache:
            notdef = (id(font), _NOTDEF_PROBE)
            if notdef not in self.glyph_cache:
                self.glyph_cache[notdef] = _glyph_mask(font, _NOTDEF_PROBE)
            self.glyph_cache[key] = _glyph_mask(font, ch) != self.glyph_cache[notdef]
        return self.glyph_cache[key]

    def find_text_font(self, size: int, bold: bool = False) -> FontChain:
        """find_font() with fallbacks, for text that comes from media metadata."""
        key = (size, bold)
        if key not in self.chains:
            fonts = [find_font(size, bold)]
            loaded, skipped = [], []
            folder = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts"
            for regular, bold_file in self.fallback_files():
                p = folder / (bold_file if bold else regular)
                if not p.exists():
                    p = folder / regular
                try:
                    # Index 0 of each .ttc is the non-UI family (Yu Gothic, not Yu Gothic UI).
                    fonts.append(ImageFont.truetype(str(p), size=size))
                    loaded.append(p.name)
                except OSError:
                    skipped.append(p.name)
            if not self.fonts_logged:
                self.fonts_logged = True
                self.log(f"FONT fallbacks: {', '.join(loaded) or 'none'}"
                         + (f"; not installed: {', '.join(skipped)}" if skipped else ""))
            self.chains[key] = FontChain(fonts, self)
        return self.chains[key]


def text_width(draw: ImageDraw.ImageDraw, text: str, font) -> int:
    if not isinstance(font, FontChain):
        return draw.textbbox((0, 0), text, font=font)[2]
    runs = font.runs(text)
    if len(runs) == 1:
        return draw.textbbox((0, 0), text, font=runs[0][1])[2]
    return int(round(sum(f.getlength(s) for s, f in runs)))


def draw_text(draw: ImageDraw.ImageDraw, xy, text: str, font, fill):
    """draw.text(), with per-character fallback when `font` is a FontChain."""
    if not isinstance(font, FontChain):
        draw.text(xy, text, font=font, fill=fill)
        return
    runs = font.runs(text)
    if len(runs) == 1 and runs[0][1] is font.primary:
        draw.text(xy, text, font=font.primary, fill=fill)  # unchanged for plain Latin text
        return
    # Share the primary font's baseline, so mixed scripts sit on one line.
    x, y = xy
    baseline = y + font.primary.getmetrics()[0]
    for s, f in runs:
        draw.text((x, baseline), s, font=f, fill=fill, anchor="ls")
        x += f.getlength(s)


# Japanese and Chinese are written without spaces, so a line may break between any two
# of these characters, not only at spaces. Hangul is left out: Korean uses spaces.
_BREAK_ANYWHERE = (
    (0x3040, 0x30FF),    # Hiragana, Katakana
    (0x31F0, 0x31FF),    # Katakana phonetic extensions
    (0x3400, 0x4DBF),    # CJK extension A
    (0x4E00, 0x9FFF),    # CJK unified ideographs
    (0xF900, 0xFAFF),    # CJK compatibility ideographs
    (0xFF66, 0xFF9F),    # halfwidth Katakana (not U+FF65, a Morse dot in the wild)
    (0x20000, 0x3FFFF),  # CJK extensions B and later
)
# Never start a line with these (a simple form of the Japanese kinsoku rule).
_NO_LINE_START = set("ー々ゝゞヽヾ・、。，．！？：；）」』】〕〉》ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ")


def _breaks_anywhere(ch: str) -> bool:
    cp = ord(ch)
    return any(lo <= cp <= hi for lo, hi in _BREAK_ANYWHERE)


def _break_units(word: str) -> list[str]:
    """Split one space-free word into the pieces a line may break between."""
    units: list[str] = []
    for ch in word:
        if units and (ch in _NO_LINE_START or not (_breaks_anywhere(ch) or _breaks_anywhere(units[-1][-1]))):
            units[-1] += ch
        else:
            units.append(ch)
    return units


def fit_text(draw: ImageDraw.ImageDraw, text: str, font, max_width: int, max_lines: int):
    words = (text or "").split()
    if not words:
        return [""]

    def width(s: str) -> int:
        return text_width(draw, s, font)

    # (separator, piece): pieces of one word join with "", words with " ".
    tokens: list[tuple[str, str]] = []
    for word in words:
        sep = " "
        for unit in _break_units(word):
            # A piece wider than the box (a long unbroken title) is broken between
            # characters; otherwise it was drawn as one over-wide line.
            if width(unit) <= max_width:
                tokens.append((sep, unit))
                sep = ""
                continue
            piece = ""
            for ch in unit:
                if piece and width(piece + ch) > max_width:
                    tokens.append((sep, piece))
                    sep = ""
                    piece = ch
                else:
                    piece += ch
            tokens.append((sep, piece))
            sep = ""

    lines = []
    current = ""
    truncated = False

    for sep, token in tokens:
        test = token if not current else current + sep + token
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


DIVIDER_Y = 135
TITLE_STEP, ARTIST_STEP, ALBUM_STEP = 20, 21, 17
MORSE_STEP = 17
MORSE_FILL = (120, 190, 255)

# A field written entirely in Morse code also gets its decoded text as a smaller line
# below it. A band writes its artist, album and track titles that way, with U+FF65 as
# the dot and U+FF0D as the dash (renderer.log, 2026-10-15). Off: --no-morse-subtitle.
MORSE_DOTS = set("\uff65.\u00b7\u2022\u2219")      # ･ . · • ∙
MORSE_DASHES = set("\uff0d-_\u2212\u2013\u2014")  # － - _ − – —
MORSE_CODE = {
    ".-": "A", "-...": "B", "-.-.": "C", "-..": "D", ".": "E", "..-.": "F", "--.": "G",
    "....": "H", "..": "I", ".---": "J", "-.-": "K", ".-..": "L", "--": "M", "-.": "N",
    "---": "O", ".--.": "P", "--.-": "Q", ".-.": "R", "...": "S", "-": "T", "..-": "U",
    "...-": "V", ".--": "W", "-..-": "X", "-.--": "Y", "--..": "Z",
    "-----": "0", ".----": "1", "..---": "2", "...--": "3", "....-": "4",
    ".....": "5", "-....": "6", "--...": "7", "---..": "8", "----.": "9",
    ".-.-.-": ".", "--..--": ",", "..--..": "?", ".----.": "'", "-.-.--": "!",
    "-..-.": "/", "-.--.": "(", "-.--.-": ")", ".-...": "&", "---...": ":",
    "-.-.-.": ";", "-...-": "=", ".-.-.": "+", "-....-": "-", ".-..-.": '"', ".--.-.": "@",
}


def decode_morse(text: str) -> str | None:
    """The text of a field written entirely in International Morse, else None.

    Letters are separated by one space, words by "/" or a wider gap. Nothing is
    returned unless every letter decodes and there are at least two of them, so
    "..." or "-" alone, or a title that merely contains dashes, never counts.
    """
    text = (text or "").strip()
    if not text or any(not (ch in MORSE_DOTS or ch in MORSE_DASHES or ch.isspace() or ch == "/") for ch in text):
        return None
    words = []
    for word in re.split(r"\s*/\s*|\s{2,}", text):
        letters = word.split()
        if not letters:
            return None
        decoded = ""
        for letter in letters:
            key = "".join("." if ch in MORSE_DOTS else "-" for ch in letter)
            if key not in MORSE_CODE:
                return None
            decoded += MORSE_CODE[key]
        words.append((decoded, len(letters)))
    if sum(n for _, n in words) < 2:
        return None
    return " ".join(w for w, _ in words)


def layout_text_block(draw, title, secondary, tertiary, title_font, artist_font, album_font, max_text_w,
                      morse_font=None, fit_text=fit_text):
    """Return (y, text, font, fill) for the title/artist/album lines.

    The album starts below the last artist line and gets only the lines that fit
    above the divider. It used to be clamped to y=110, which drew it over a
    two-line artist (issue #10). A field in Morse code gives up one of its lines
    to the decoded text, drawn in `morse_font` (None: no decoded lines). With room
    for one line only (the album, under a long title and artist), the decoded text
    replaces the Morse: a truncated Morse line says almost nothing.
    """
    items = []

    def field(y, text, font, fill, step, max_lines):
        decoded = decode_morse(text) if morse_font is not None else None
        if not decoded or max_lines > 1:
            for line in fit_text(draw, text, font, max_text_w, max_lines - 1 if decoded else max_lines):
                items.append((y, line, font, fill))
                y += step
        if decoded:
            for line in fit_text(draw, decoded, morse_font, max_text_w, 1):
                items.append((y, line, morse_font, MORSE_FILL))
            y += MORSE_STEP
        return y

    y = field(9, title, title_font, (242, 242, 245), TITLE_STEP, 3)

    y = max(y + 4, 72)
    y = field(y, secondary, artist_font, (224, 224, 230), ARTIST_STEP, 2)

    if tertiary:
        album_y = y + 1
        room = min(2, (DIVIDER_Y - album_y) // ALBUM_STEP)
        if room > 0:
            field(album_y, tertiary, album_font, (180, 180, 188), ALBUM_STEP, room)

    return items
