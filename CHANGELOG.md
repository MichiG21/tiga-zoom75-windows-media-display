# Bundle changelog

## v0.3.0-beta.1 (not yet tagged)

### License

- **The project is now licensed GPL-2.0-only** (previously MIT), matching
  [meletricks](https://github.com/athdesk/meletricks), which the planned work on the
  keyboard's display MCU builds on. v0.1.0 and v0.2.0-beta.1 remain under MIT. `LICENSE`
  and `THIRD_PARTY_NOTICES.md` updated (#27).

### Uploads and recovery

- An interrupted upload no longer leads to a retry every 8 s against a keyboard stuck
  mid-transfer. The uploader exits with a class (codes 10, 11, 20, 21, 30), writes a
  session marker while a transfer is open, and the supervisor halts instead of restarting;
  `logs\uploader_halted.json` records why and what to do. `START_STACK.ps1` clears a halt;
  `STATUS_STACK.ps1` shows it (#4, PR #13).
- **Automatic retry after a sudden failure:** a code 20 whose transfer ran at a normal pace
  and then got no PROGRESS (`SIGNATURE sudden`) is retried after 120 s, up to 3 times per
  incident, without a power-off. Slowing or unclassified failures, code 21 and a killed or
  crashed uploader still halt at once (PR #28).
- An unreachable keyboard (codes 10 and 11) is retried after 8, 16, 32 and 60 s, then every
  5 minutes, and never halts (PR #16).
- Device selection by the display's GATT service instead of by name, rejecting
  `Zoom75 Tiga BT1`, with the last working address cached (#14, PR #16).
- The BLE scan stops once the keyboard is selected and connected (PR #26).
- **Clean stop:** `STOP_STACK.ps1` asks the uploader to stop and waits up to 30 s; an upload
  in progress is finished before it disconnects. Only then does it force-kill. The
  supervisor doesn't restart an uploader that stopped on request (PR #20).
- The block timeout is back to 2500 ms (it was 10000 ms for a day on `main`; it never
  rescued an upload) (PR #18).

### Renderer and display

- Japanese, Chinese, Korean and symbol characters in titles, artists and albums now
  display instead of empty boxes. Each character is drawn with the first Windows font
  that has it. Japanese and Chinese titles wrap between characters. Plain Latin titles
  render exactly as before (#10, PR #17).
- A title, artist or album written entirely in Morse code gets its decoded text as a
  smaller second line. Turn it off with `--no-morse-subtitle`.
- The renderer logs which fallback fonts it found, and any character no font has.
- Unbroken long titles wrap inside the text area; a long album no longer overlaps a
  two-line artist; new artwork with the same file size is noticed; repeated media-query
  failures fall back to standby after a minute (#10).
- Media already playing at startup releases standby within about 15 s, even if the
  player never reports its position moving (#5).
- Non-Latin titles no longer crash the renderer.

### Display Control

- Reissuing the same command works (#6).
- Every command is checked by read-back and the renderer's acknowledgement; a write that
  never arrived is reported instead of shown as sent (#15, PR #16).
- **Standby + Shut Down** waits for the keyboard to confirm the standby frame before
  shutting down, asks if it doesn't come in time, and cancels if the command never reached
  the renderer (#7, PR #12).

### Diagnostics

- Logs keep their history across restarts and rotate at 5 MB; every frame has an ID shared
  by `renderer.log` and `ble.log`; `STATUS_STACK.ps1` shows the last delivered frame (#9).
- `ble.log` records how long every block took: a `SLOW` line for any block over 1 s, a
  warning when the first upload after connecting is slow by block 5, and a `TIMING`
  summary (median, p90, max, slowest block, CONTROL1/CONTROL2 ACK times) after every
  upload. The same figures go into `<stream>.delivered` (PR #19, #22).
- `ble.log` logs every notification that isn't PROGRESS or COMPLETE, with what the
  uploader was waiting for (PR #25). An identical repeat is counted and summarised in one
  line instead of logged again (PR #30).

### Other fixes

- Development startup works when the checkout path contains spaces (#8).

### Documentation

- Quick start: the keyboard is used wired over USB-C; the display is reached over
  Bluetooth LE without pairing (PR #27).
- README shortened; details moved to `docs/ARCHITECTURE.md` and
  `docs/BUILD_AND_RECOVERY.md`; notes, checklist, CONTRIBUTING and SECURITY moved to
  `docs/` and `.github/` (PR #29).
- `docs/PROJECT_HISTORY.md` records the hardware sessions of 2026-10-15 to 2026-10-22,
  including the upload-failure investigation (PR #24) and the live clean-stop and
  retry results (PR #31).

## 2026-09-09 complete snapshot

- Windows GSMTC generic media source
- static full-duration display when available
- startup stale-session gate
- large standby clock/date screen
- 60-second idle-to-standby behavior
- manual display control utility
- safe standby-then-shutdown option
- flow-controlled BLE uploader using 53 block progress ACKs
- startup supervisor fixed to mirror the proven-good relative-path BLE invocation
- portable scheduled-task installer for this archive
- convenience start/stop/status/build scripts
