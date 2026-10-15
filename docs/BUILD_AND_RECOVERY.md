# Build and recovery notes

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
- `extracted\session_01_bulk_stream.bin` (the active upload stream; seeded from `session_01_bulk_stream.redgreen-backup.bin`, then rewritten on every frame)
- `extracted\session_01_bulk_stream.windows_nowplaying.bin`
- `extracted\session_01_bulk_stream.bin.frame` (ID of the last rendered frame)
- `extracted\session_01_bulk_stream.bin.delivered` (last frame the keyboard confirmed)
- `extracted\session_01_bulk_stream.bin.device` (address of the keyboard that completed the last upload; the uploader tries it first. Delete it to force a full scan)

## Full stack stop

```powershell
.\STOP_STACK.ps1
```

## BLE recovery

If an upload is interrupted after CONTROL2 (the point where the keyboard shows its upload animation), the supervisor halts the uploader instead of retrying, and writes `logs\uploader_halted.json`. `STATUS_STACK.ps1` shows why. Power-cycle the keyboard with its power switch (unplugging USB does not power-cycle it: the TIGA has a battery; if the display stays stuck, disconnect the battery), then run `START_STACK.ps1`, which clears the halt. Do not flash firmware or factory-reset the board as a first-line recovery step. See `docs/design/RETRY_RECOVERY.md` for the exit codes and the full policy.
