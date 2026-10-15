# DLX upload stream: what is known

Everything here comes from **one** capture: the PocketWuque red/green 2-frame upload
(`extracted/session_01_bulk_stream.redgreen-backup.bin`, SHA-256 `a4be0a5d…`). The two
capture sessions are byte-identical, and `reference/diagnostics/tiga_known_good_red_green_media.bin`
is the same DLX body without its prefix and trailer, so there is no independent
second sample.

`tiga_dlx.py` encodes this model. Its offline tests check that the model rebuilds the
captured stream **byte-for-byte** from its fields plus the two frames. That proves the
model is self-consistent with this capture. It does **not** prove what the firmware
reads.

## Outer stream (220,649 bytes)

| Stream offset | Bytes | Meaning | Status |
|---:|---|---|---|
| 0 | `88 00 00 00 03 5d e1 00` | envelope; `03 5d e1` = 220,641 = bytes after the envelope (u24 BE) | established |
| 8 | `08 05 01 00 09 00 ff ff ff 00 ff ff ff 02 02 ff ff` | upload preamble | unknown; the `02 02` *might* relate to frame count |
| 25 | DLX file (220,624 bytes) | | |

CONTROL1 (`09 03 04 00 08 00 00 ff fc 00 03 5d d0 05 00 00` after its 8-byte
envelope) carries `03 5d d0` = 220,624 = DLX file length (u24 BE). Established.

## DLX file: offsets below are file-relative (stream offset − 25)

| File offset | Bytes | Value | Meaning | Status |
|---:|---|---|---|---|
| 0 | `00 44 4c 58 fc ff 00 00` | `\0DLX` | prefix | established |
| 8 | `60 01` | 352 | u16 LE: file offset of the section table | **new**; consistent |
| 10 | `03 00` | 3 | u16 LE: number of 28-byte records | **new**; consistent |
| 12 | `02 00` | 2 | u16 LE: frame count | hypothesis |
| 14 | `00 01` | | unknown | unknown |
| 16–27 | `00` × 12 | | | |
| 28–351 | `ff` × 324 | | padding / unused | |
| 352 | `70 01 00 00` | 368 | u32 LE: records start | **new**; consistent |
| 356 | `c4 01 00 00` | 452 | u32 LE: frame index start | consistent |
| 360 | `cc 01 00 00` | 460 | u32 LE: pixel data start | established (transcript) |
| 364 | `cc 5d 03 00` | 220,620 | u32 LE: pixel data end (= trailer offset) | established (transcript) |
| 368 | 3 × 28 bytes | | records, see below | partly |
| 452 | `00 00 00 00` `00 ae 01 00` | 0, 110,080 | see "frame index" | **two readings** |
| 460 | 2 × 110,080 | | RGB565 big-endian frames | established |
| 220,620 | `fc ff 00 00` | | trailer | established |

The transcript recorded the 452 / 460 / 220,620 / 320 / 172 / 110,080 fields. It had
them as header-relative offsets (`0x15C`…). The file-relative reading above also
explains the first u16 (352): it points at the table holding those values. So every
offset in the header is a file offset.

Note: "452" is two different things that happen to be equal. It is the header length
(460 − 8), and it is the file offset of the frame index (368 + 3 × 28). For a
one-frame container under the "index" reading, the header becomes 448 bytes while the
frame index stays at 452.

### Records (file offset 368, 28 bytes each)

```
15 01 00 00 00 00 00 01 00 00 01 40 00 ac 00…   type 0x15 id 1  count 1
14 03 00 01 0a 00 00 02 00 00 01 40 00 ac 00…   type 0x14 id 3  flags 1  delay? 0x0a  count 2
14 04 00 00 00 00 00 01 00 00 01 40 00 ac 00…   type 0x14 id 4  count 1
```

Bytes 10–13 are `01 40` / `00 ac` = 320 × 172 (BE) in all three. Record 2 is the
only one carrying a 2 and a non-zero byte 4 (`0x0a` = 10, plausibly a frame delay in
10 ms units → 100 ms), so it looks like the animation layer. The other records' roles
are unknown. Note that the counts (1 + 2 + 1) don't add up to the 2 frames present,
so the records are not "one per frame".

### Frame index (file offset 452, 8 bytes)

Two readings fit the capture equally well:

- **index**: one u32 per frame, the offset from pixel-data start: `[0, 110080]`.
  The block's size (452 → 460 = 8 bytes = 2 × 4) then depends on the frame count.
  This reading explains *why* the section table lists a start for this block.
- **fixed**: one `[offset, frame size]` descriptor: `[0, 110080]`, the same for any
  frame count.

The first hardware test should tell them apart (see below).

## A one-frame container (behind flags, never sent)

`py tiga_windows_nowplaying_v2.py --dlx-frames 1 [--dlx-layout index|fixed]` builds
one, and the uploader accepts it only with `--allow-single-frame`. Changes from the
capture:

| Field | 2-frame | 1-frame `index` | 1-frame `fixed` |
|---|---:|---:|---:|
| frame count (file 12) | 2 | 1 | 1 |
| record 2 count | 2 | 1 | 1 |
| frame index | `[0, 110080]` | `[0]` | `[0, 110080]` |
| pixel data start / end | 460 / 220,620 | 456 / 110,536 | 460 / 110,540 |
| DLX file length (CONTROL1) | 220,624 | 110,540 | 110,544 |
| stream length (envelope + 8) | 220,649 | 110,565 | 110,569 |
| BLE writes / 4K PROGRESS blocks | 1,239 / 53 | 621 / 26 | 621 / 26 |

Left unchanged (unknown, so not touched): the preamble's `02 02`, record 2's
`0x0a`, the `00 01` at file offset 14.

## Suggested hardware sequence (with Michi present)

Each step changes one variable. Power-cycle after any failure before the next step
(PROJECT_HISTORY §9.2, §2.4).

1. Baseline: current 2-frame stream; confirm a normal upload and note the time.
2. `--dlx-frames 1 --dlx-layout index` + `--allow-single-frame`. Watch for PROGRESS
   count (expect 26), COMPLETE, and whether the image shows.
3. If 2 fails cleanly, `--dlx-layout fixed`.
4. If both fail, keep `index` and change the preamble `02 02` → `01 01` (not
   implemented as a flag yet: a one-line change in `tiga_dlx.build_stream`).

The uploader's dry-run mode (no `--confirm-flow-watch`) prints the write and block
counts for a stream without opening Bluetooth. Use it to check a generated stream
first.
