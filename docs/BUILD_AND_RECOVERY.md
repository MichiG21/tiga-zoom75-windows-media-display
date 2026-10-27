# Build and recovery notes

## Build from source

The installer is the recommended path for normal users. The following is only needed if you want to develop, modify, or build the project yourself.

### Developer requirements

- **Windows 10 or Windows 11**
- **Meletrix / Wuque Studio Zoom75 TIGA**
- Bluetooth enabled
- **Visual Studio 2022 C++ Build Tools**
- **Developer PowerShell for VS 2022**
- **Rust + Cargo**
- **Python** with the `py.exe` launcher
- **Pillow**

### Clone

```powershell
git clone https://github.com/MichiG21/tiga-zoom75-windows-media-display.git
cd tiga-zoom75-windows-media-display
```

### Build and install the development stack

Open **Developer PowerShell for VS 2022** in the repository folder and run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\BUILD_AND_INSTALL.ps1
```

Then start it with:

```powershell
.\START_STACK.ps1
```

Other development commands:

```powershell
.\STOP_STACK.ps1
.\STATUS_STACK.ps1
```

### Manual BLE test

From the `windows_ble_probe` folder after building. `extracted\session_01_bulk_stream.bin` is generated at runtime and is not in Git; on a fresh checkout, run the renderer once or copy `extracted\session_01_bulk_stream.redgreen-backup.bin` to that name first.

```powershell
.\target\release\tiga_stream_watch_flow.exe --stream "..\extracted\session_01_bulk_stream.bin" --intra-block-ms 0 --confirm-flow-watch
```

Leave out `--confirm-flow-watch` for a dry run: it validates the stream and opens no Bluetooth connection.

A stream with no `.frame` sidecar next to it (the copied template, for example) is held until the renderer writes a frame (`UPLOAD HELD` in the log). Add `--allow-no-sidecar` to upload it anyway.

### Installing from source: troubleshooting

Problems hit on a from-scratch install. None of them means the project is broken.

**PowerShell refuses to run the scripts** (`UnauthorizedAccess`, "running scripts is disabled on this system").
Windows blocks `.ps1` files by default. Allow them for your user once (no admin needed):

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
```

or prefix each call with `powershell -ExecutionPolicy Bypass -File`.

**"… is not digitally signed"**, even after that.
Files extracted from a downloaded ZIP carry Windows' mark-of-the-web. Unblock them from the repository folder:

```powershell
Get-ChildItem -Recurse -File | Unblock-File
```

A `git clone` doesn't have this problem.

**A freshly built `.exe` seems to do nothing, or its files never appear.**
Antivirus software (Norton CyberCapture, for example) may hold or sandbox an executable it hasn't seen before, so its writes don't reach the folder. The Display Control utility now says so if the renderer never confirms a command ([#15](https://github.com/MichiG21/tiga-zoom75-windows-media-display/issues/15)). Wait for the scan to finish, or allow the program in your antivirus.

**`tiga_stream_watch_flow.exe` "is not recognized"** after `cargo test`.
`cargo test` builds a separate test binary. Build the real one with `cargo build --release` in `windows_ble_probe`.

**Manual dry run fails with `EXIT code=30 … cannot find the file specified`.**
A fresh clone has no `extracted\session_01_bulk_stream.bin`; it is generated at runtime. The supervisors seed it automatically, but a manual run needs it copied first:

```powershell
Copy-Item .\extracted\session_01_bulk_stream.redgreen-backup.bin .\extracted\session_01_bulk_stream.bin
```

## Build current components manually

### C++ media snapshot helper

```powershell
cl /std:c++20 /EHsc .\tiga_windows_media_snapshot.cpp /Fe:tiga_windows_media_snapshot.exe /link windowsapp.lib
```

### Display control utility

```powershell
cl /std:c++20 /EHsc /DUNICODE /D_UNICODE .\tiga_display_control.cpp /Fe:tiga_display_control.exe /link /SUBSYSTEM:WINDOWS user32.lib shell32.lib ole32.lib
```

### BLE uploader

```powershell
cd .\windows_ble_probe
cargo build --release --locked --bin tiga_stream_watch_flow
```

## Runtime files created automatically

- `windows_media_thumbnail.bin`
- `windows_nowplaying_preview.png`
- `tiga_display_command.txt`
- `logs\*.log` (appended across restarts, rotated to `*.log.1`..`.3` at 5 MB)
- `logs\uploader_halted.json` (present while the uploader is halted) and `logs\uploader_retry.json` (automatic retries after a sudden failure)
- `extracted\session_01_bulk_stream.bin` (the active upload stream; seeded from `session_01_bulk_stream.redgreen-backup.bin`, then rewritten on every frame)
- `extracted\session_01_bulk_stream.windows_nowplaying.bin`
- `extracted\session_01_bulk_stream.bin.frame` (ID of the last rendered frame; until it exists, the uploader holds the stream instead of sending the seeded template)
- `extracted\session_01_bulk_stream.bin.delivered` (last frame the keyboard confirmed)
- `extracted\session_01_bulk_stream.bin.device` (address of the keyboard that completed the last upload; the uploader tries it first. Delete it to force a full scan)

## Full stack stop

```powershell
.\STOP_STACK.ps1
```

## BLE recovery

If an upload is interrupted after CONTROL2 (the point where the keyboard shows its upload animation), what happens depends on how it stopped. A *sudden* stop (a normal pace, then no answer) is retried automatically after 120 s, up to 3 times. A slowing one, or a sudden one after three failed retries, halts the uploader and writes `logs\uploader_halted.json`. `STATUS_STACK.ps1` shows which (`RETRYING` or `HALTED`) and why. To recover from a halt: the TIGA has no power switch, and unplugging USB doesn't reset it — it has a battery. Open the case, disconnect the battery from the PCB, and wait at least 30 seconds, pressing a few keys while it's disconnected. Reconnect, then run `START_STACK.ps1`, which clears the halt. A 10–15 second disconnect was not enough in testing. Do not flash firmware or factory-reset the board as a first-line recovery step. See `docs/design/RETRY_RECOVERY.md` for the exit codes and the full policy.

## Troubleshooting

The two most common cases (the TIGA staying on the roar, and an interrupted upload) are in the [README](../README.md#troubleshooting). The rest:

### Logs

Logs in `logs\` are appended across restarts and rotated at 5 MB (`renderer.log.1` ... `.3`). Every process start is stamped with a run ID, and every frame has a frame ID (for example `renderer-20261015-091500-a1b2c3-f0004`) that appears in `renderer.log` when it is rendered and in `ble.log` at `UPLOAD START`, `UPLOAD COMPLETE` or `UPLOAD FAILED`, so the two logs can be matched up.

### Keyboard unreachable

If the uploader cannot reach the keyboard (battery disconnected, asleep, out of range, or Bluetooth off), it does **not** halt: it retries after 8, 16, 32 and 60 seconds, then every 5 minutes until the keyboard is back. No **Start Display** is needed: once the keyboard is reachable again, the next retry picks it up, within about 5 minutes.

### Turning the keyboard off on purpose

The TIGA has no power switch, so "off" means the battery disconnected from the PCB. If you do that on purpose, do it only while no upload is running: check that **Status** says `OK`, or that the last line in `logs\ble.log` is `UPLOAD COMPLETE`. A disconnect in the middle of an upload interrupts it, and the uploader halts. This is not the halt recovery in the README: that one needs the battery disconnected for at least 30 seconds and then **Start Display**.

Stopping the stack (or shutting Windows down) in the middle of an upload counts as an interruption too. The next start will report it as halted.

### Media is detected but duration is missing

This is usually caused by the application / browser not publishing useful timeline data through Windows GSMTC. The project will still render the available metadata.

### Existing manual installation

The beta installer uses the Scheduled Task name:

```text
TIGA Windows Media Display
```

If an older manual version already uses that task name, the installer may replace that task with the packaged installation.
