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

## USB/HID observations

USB command `FC` behaves as persistent custom-media staging rather than a useful live framebuffer. Opening an incomplete `FC` media session can temporarily return the custom slot to the factory tiger/roar animation, and committing incomplete data can produce a white/invalid custom slot. The working runtime design therefore uses BLE bulk media upload rather than USB `FC` for Now Playing images.

Runtime HID commands such as time/date and telemetry are a separate path and have coexisted with the custom image slot in testing.
