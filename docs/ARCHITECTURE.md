# Architecture

```text
Windows GSMTC session
        |
        v
`tiga_windows_media_snapshot.exe`
  - title / artist / album / state / timeline
  - writes current thumbnail
        |
        v
`tiga_windows_nowplaying_v2.py`
  - stale-session freshness gate
  - standby clock/date page
  - Now Playing renderer
  - patches both RGB565 frames in the DLX stream
        |
        v
`extracted/session_01_bulk_stream.bin`
        |
        v
`tiga_stream_watch_flow.exe`
  - watches stream for changes
  - BLE service 1f40eaf8-aab4-14a3-f1ba-f61f35cddbaa
  - control 0001, status 0002, bulk 0003, notify 0004
  - waits for one progress ACK per 4096-byte DLX block
        |
        v
Zoom75 TIGA LCD
```

At logon, `tiga_startup_supervisor.ps1` starts and supervises both the Python renderer and Rust BLE uploader. The scheduled task is named `TIGA Windows Media Display`.
