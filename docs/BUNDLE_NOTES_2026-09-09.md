# TIGA Windows Media Display — complete working bundle

Snapshot date: **2026-09-09**

This archive is a clean backup/shareable development bundle for the custom Zoom75 TIGA Windows Now Playing display.

## Current behavior

- Reads the active Windows Global System Media Transport Controls session (GSMTC), not Spotify-specific APIs.
- Works with Spotify and other apps/browsers that publish Windows media-session metadata.
- Renders a 320×172 RGB565 big-endian custom display image with cover art, title, artist/creator, album and static full duration where Windows provides it.
- Firefox/YouTube may publish title/thumbnail but no duration; the renderer falls back gracefully.
- On startup it renders a large clock/date standby page first and gates stale remembered media until playback is proven fresh.
- After media is paused/stopped for 60 seconds, it returns to standby to avoid leaving cover art static indefinitely.
- Flow-controlled BLE uploader waits for the keyboard's 4096-byte progress notifications and typically completes a full image upload in about 8 seconds on the original test system.
- During a custom upload the TIGA may briefly show the stock roar/tiger animation before the new image is committed. That is expected.

## Production filenames

The renderer is the newest standby/fresh-session implementation, but it deliberately keeps the historical filename `tiga_windows_nowplaying_v2.py` because the startup supervisor points to that name.

Main files:

- `tiga_windows_nowplaying_v2.py` — current Python renderer/controller
- `tiga_windows_media_snapshot.cpp` — current C++/WinRT GSMTC snapshot helper source
- `tiga_display_control.cpp` — manual standby/black/auto/safe-shutdown GUI source
- `tiga_startup_supervisor.ps1` — current known-good supervisor (BLE launch mirrors the tested manual invocation)
- `windows_ble_probe/src/bin/tiga_stream_watch_flow.rs` — current flow-controlled BLE uploader
- `extracted/session_01_bulk_stream.bin` — known-good 220,649-byte DLX upload stream template

## Quick build/install

Prerequisites:

- Windows 10/11
- Visual Studio 2022 C++ Build Tools / Developer PowerShell
- Rust + Cargo
- Python (`py.exe`) and Pillow
- Bluetooth enabled; keyboard visible as `Zoom75 Tiga`

Open **Developer PowerShell for VS 2022**, `cd` into this folder, then run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\BUILD_AND_INSTALL.ps1
```

Then:

```powershell
.\START_STACK.ps1
```

Stop everything cleanly with:

```powershell
.\STOP_STACK.ps1
```

Check task/process state with:

```powershell
.\STATUS_STACK.ps1
```

## Manual BLE test

From `windows_ble_probe`:

```powershell
.\target\release\tiga_stream_watch_flow.exe --stream "..\extracted\session_01_bulk_stream.bin" --intra-block-ms 0 --confirm-flow-watch
```

The process intentionally remains running in watch mode after a successful upload.

**Do not intentionally kill/power off the keyboard halfway through an upload.** If a flow upload times out after partially sending data, fully power-cycle the keyboard before another media test.

## Security / Meletrix HID note

The custom media pipeline does **not** require re-enabling the previously blocked `meletrixid.sys` kernel driver. Normal Meletrix HID runtime updates such as clock/telemetry have coexisted with the custom BLE media uploader in testing. Avoid running a second custom-media upload from official software at the same time as this uploader.

## Sharing with another developer

The `reference` folder contains reverse-engineering probes, diagnostic assets and protocol notes. It intentionally excludes large raw packet captures and personal runtime logs from the clean bundle.
