# Architecture

## How it works

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
  - Now Playing renderer: artwork, title, artist / creator, album, duration
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
  - BLE bulk upload
  - waits for one progress ACK per 4096-byte DLX block
  - completion acknowledgement
        |
        v
Zoom75 TIGA LCD
```

At logon, `tiga_startup_supervisor.ps1` starts and supervises both the Python renderer and Rust BLE uploader. The scheduled task is named `TIGA Windows Media Display`.

More detail on the protocol is in [`PROTOCOL_NOTES.md`](PROTOCOL_NOTES.md).

## Media-session behavior

The project relies on what each application publishes through Windows GSMTC.

Examples:

- **Spotify** normally exposes title, artist, album, thumbnail, playback state, and duration.
- **Browser media** can expose title, creator, thumbnail, and playback state.
- Some browser / website combinations may omit duration or other timeline information.

The renderer falls back gracefully when a field is unavailable.

### Standby and stale-session handling

Windows can keep old media-session metadata around after playback has stopped or after a reboot.

To avoid showing an old track immediately at login, the renderer:

1. shows standby first,
2. records the media state Windows exposes at startup,
3. waits until playback is proven fresh,
4. then switches to Now Playing.

Fresh playback can be detected through a new media item, a playback-state transition, or an advancing timeline when the source provides one.

Some sources report "Playing" but never advance their position. If the same media item, with a title, reports "Playing" continuously for 15 seconds, that also counts as fresh. A remembered session that reports Paused or Stopped still stays on standby.

## BLE uploader

The uploader uses the keyboard's own progress notifications as flow-control.

Instead of sleeping a fixed amount between every write, it sends data quickly inside a 4096-byte block and waits for the keyboard's progress notification before continuing.

This reduced a full upload on the original development unit from roughly **39 seconds** to about **8 seconds**.

The uploader intentionally remains active in **watch mode** after a successful upload so later media changes can be transferred without rebuilding the connection stack from scratch.

Failure classes, exit codes and the retry / halt policy: [`design/RETRY_RECOVERY.md`](design/RETRY_RECOVERY.md).

## Privacy

The active implementation reads the **local Windows media session**.

It does not require:

- a Spotify developer account
- Spotify OAuth
- a cloud service for Now Playing metadata
- uploading your media history anywhere

Runtime logs stay local and are ignored by Git.

## Repository layout

```text
.
├── .github/                 (workflows, CONTRIBUTING.md, SECURITY.md)
├── installer/
│   ├── runtime/
│   └── TIGA-Windows-Media-Display.iss
├── BUILD_AND_INSTALL.ps1
├── START_STACK.ps1
├── STOP_STACK.ps1
├── STATUS_STACK.ps1
├── install_tiga_startup.ps1
├── tiga_startup_supervisor.ps1
├── tiga_windows_nowplaying_v2.py
├── tiga_windows_media_snapshot.cpp
├── tiga_display_control.cpp
├── windows_ble_probe/
├── extracted/
├── docs/
├── tests/
├── tools/
└── reference/
```

### `installer/`

Windows installer definition and packaged-runtime helper scripts.

### `windows_ble_probe/`

Rust source for the current BLE uploader and earlier BLE investigation tools.

### `extracted/`

Contains the known-good media stream template used by the renderer.

### `docs/`

Architecture, protocol notes, build notes, and recovery information. Also the project history, the release checklist, the development notes (`NOTES_FOR_MICHI.md`), investigations, and the README screenshots (`docs/images/`).

### `tests/`, `tools/`

Offline tests (Python, PowerShell; `cargo test` for the uploader) and analysis tooling such as `tools/upload_attempts.py`.

### `reference/`

Reverse-engineering probes and supporting material from earlier development.

Large raw packet captures, private transcripts, credentials, and personal runtime logs are intentionally not included.

## Reverse-engineering

The media-upload protocol used here was reverse-engineered from the Zoom75 TIGA's BLE traffic and media-transfer behavior.

Documented areas include:

- GATT service / characteristics
- media envelope structure
- DLX frame layout
- RGB565 big-endian pixels
- chunking behavior
- XOR checksums
- progress notifications
- flow-control
- completion acknowledgement

See [`PROTOCOL_NOTES.md`](PROTOCOL_NOTES.md) for details.
