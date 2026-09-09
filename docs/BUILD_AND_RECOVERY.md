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
cargo build --release --bin tiga_stream_watch_flow
```

## Runtime files created automatically

- `windows_media_thumbnail.bin`
- `windows_nowplaying_preview.png`
- `tiga_display_command.txt`
- `logs\*.log`
- `extracted\session_01_bulk_stream.windows_nowplaying.bin`

## Full stack stop

```powershell
.\STOP_STACK.ps1
```

## BLE recovery

If an upload is interrupted in the middle and later attempts time out waiting for `PROGRESS`, stop the stack and fully power-cycle the keyboard before another upload attempt. Do not flash firmware or factory-reset the board as a first-line recovery step.
