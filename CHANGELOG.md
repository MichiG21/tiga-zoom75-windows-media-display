# Bundle changelog

## Unreleased

- Japanese, Chinese, Korean and symbol characters in titles, artists and albums now
  display instead of empty boxes. Each character is drawn with the first Windows font
  that has it. Japanese and Chinese titles wrap between characters. Plain Latin titles
  render exactly as before (#10).
- A title, artist or album written entirely in Morse code gets its decoded text as a
  smaller second line. Turn it off with `--no-morse-subtitle`.
- The renderer logs which fallback fonts it found, and any character no font has.

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
