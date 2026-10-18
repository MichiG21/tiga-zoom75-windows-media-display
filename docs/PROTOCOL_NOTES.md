# Reverse-engineered protocol notes (working subset)

## Current BLE media transport

Device name: `Zoom75 Tiga`

Service UUID:

`1f40eaf8-aab4-14a3-f1ba-f61f35cddbaa`

Characteristics:

- `1f400001-...` control write
- `1f400002-...` status notify
- `1f400003-...` bulk media write
- `1f400004-...` second notify

Observed effective bulk write payload is normally 181 data bytes + 1 XOR checksum byte. Chunking is aligned so a write never crosses a 4096-byte boundary in the embedded DLX media file.

Known-good stream length: **220,649 bytes**.

Stream layout:

- 8-byte outer envelope
- 17-byte upload preamble
- 220,624-byte DLX body
  - 8-byte DLX prefix: `00 44 4C 58 FC FF 00 00`
  - 452-byte media header
  - frame 1: 110,080 bytes
  - frame 2: 110,080 bytes
  - trailer: `FC FF 00 00`

Image: **320×172 RGB565 big-endian**.

Uploader sends 1,239 BLE writes and receives **53 progress notifications**, one after each complete 4096-byte DLX block. Waiting on those progress notifications, instead of blind per-write pacing, reduced full upload time from roughly 39 seconds to about 8 seconds on the original setup.

Progress notification:

`88 00 00 00 00 00 03 0B 08 06 05`

Completion notification:

`88 00 00 00 00 00 03 0C 08 06 02`

### Other notifications (observations, meaning unknown)

Seen once the uploader started logging every notification (`NOTIFY` lines in `ble.log`,
#25). Source: the live install's `ble.log`, 2026-10-18 15:11–15:21 local: **one
connection**, 8 uploads. Everything below is what was seen, not what it means.

**Framing**, shared by every notification seen so far: `88 00 00 00`, a 24-bit
big-endian length, a type byte, then exactly that many payload bytes. Progress is length
3, type `0B`, payload `08 06 05`; completion is length 3, type `0C`; the CONTROL1/CONTROL2
ACKs are types `0E` and `0F`. The outer envelope of the upload stream starts the same way
(`88 00 00 00` + u24 length).

**`char2_ntf_data`** (14 bytes, on `1f400002`):

`63 68 61 72 32 5F 6E 74 66 5F 64 61 74 61` = ASCII `char2_ntf_data`

- Arrived once, right after connecting and subscribing: in the 150 ms drain before the
  first CONTROL1, 0 ms into it.
- It doesn't use the `88` framing. It reads like a firmware debug or "notifications on
  characteristic 2 enabled" string, but that is a guess.
- n = 1 connection. Whether it comes on every connection, or only on the first after
  the keyboard powers up, is unknown.

**The 28-byte status message** (on `1f400002`):

`88 00 00 00 00 00 14 2A 03 0D 00 00 01 40 00 00 01 40 00 00 01 40 00 00 01 64 00 00`

- Length `0x14` = 20, type `0x2A`, payload of 20 bytes, which falls naturally into five
  4-byte groups: `03 0D 00 00` · `01 40 00 00` · `01 40 00 00` · `01 40 00 00` ·
  `01 64 00 00`.
- Read as big-endian 16-bit values at the start of each group: `0x030D` = 781,
  `0x0140` = **320** three times, and `0x0164` = 356. 320 is the LCD's width; whether
  that is a coincidence is unknown. Nothing in the payload reads as 172 (`0x00AC`), the
  height.
- **It never changed:** 323 copies over 9 minutes were byte-for-byte identical. That
  covered idle time, uploads in progress (including between PROGRESS notifications),
  standby frames and Now Playing frames.
- **Cadence:** about every 2 s (gaps of 0–3 s at one-second log resolution, mostly
  2 s), starting about 1 s after connecting.
- Nothing has been seen on `1f400004`.
- Because it repeats, the uploader logs it once and then counts it: a `NOTIFY repeated N
  more times … first=… last=…` line is written when a different notification arrives,
  before each `UPLOAD START` / `COMPLETE` / `FAILED`, and before the uploader exits.

Open: what the four values describe, whether the message changes in any keyboard state
(during a stall, after a halt, in other display modes), and whether it was already
being sent before 2026-10-18 (earlier uploaders discarded it unseen).

## USB/HID observations

USB command `FC` behaves as persistent custom-media staging rather than a useful live framebuffer. Opening an incomplete `FC` media session can temporarily return the custom slot to the factory tiger/roar animation, and committing incomplete data can produce a white/invalid custom slot. The working runtime design therefore uses BLE bulk media upload rather than USB `FC` for Now Playing images.

Runtime HID commands such as time/date and telemetry are a separate path and have coexisted with the custom image slot in testing.
