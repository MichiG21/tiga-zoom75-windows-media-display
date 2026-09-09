# TIGA Windows Media Display

![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?logo=windows)
![License](https://img.shields.io/badge/License-MIT-green)
![Hardware](https://img.shields.io/badge/Hardware-Zoom75%20TIGA-7A5AF8)
![Status](https://img.shields.io/badge/Status-Early%20Community%20Release-orange)

Turn the **Meletrix / Wuque Studio Zoom75 TIGA** LCD into a Windows-wide **Now Playing** display.

The project reads the active Windows media session, renders artwork and metadata for the TIGA's **320×172** screen, and uploads the result using a reverse-engineered BLE media-transfer path.

> **Unofficial community project.** This project is not affiliated with or endorsed by Meletrix / Wuque Studio.

---

## Preview

### Now Playing

Displays cover art, title, artist / creator, album, and total duration when the active Windows media source exposes it.

![TIGA Now Playing](Now-Playing.jpeg)

### Standby

When no fresh media is playing, the display switches to a clean clock-and-date standby screen.

![TIGA Standby](Standby.jpeg)

### Upload transition

While a new custom frame is being committed, the TIGA may briefly show its built-in roar animation before the new image appears.

![TIGA upload transition](Intermission-Screen.jpeg)

---

## Features

- Uses Windows **Global System Media Transport Controls (GSMTC)** instead of a Spotify-specific API.
- Works with Spotify and other applications that expose a Windows media session.
- Displays:
  - artwork / thumbnail
  - title
  - artist / creator
  - album
  - static total duration when available
- Large clock-and-date standby screen.
- Ignores stale remembered media at login until playback is proven fresh.
- Returns to standby after playback has been paused or stopped for **60 seconds**.
- Uses keyboard progress notifications as BLE flow-control instead of blindly pacing writes.
- Full custom-frame upload typically takes about **8 seconds** on the original development unit.
- Includes a small Windows display-control utility with:
  - automatic mode
  - manual standby
  - black screen
  - standby + safe shutdown
- Starts automatically at user logon through a Windows Scheduled Task.
- Does **not** require the blocked / vulnerable `meletrixid.sys` kernel driver.

---

## Current status

The project is functional on the original Windows development system and Zoom75 TIGA unit.

This is still an **early community release**. Testing on additional TIGA firmware revisions and Windows systems is very welcome.

### Known behavior / limitations

- Firefox + YouTube may expose title and thumbnail but no usable total duration.
- The TIGA can briefly show its built-in roar animation during a custom-image upload.
- If a BLE upload is interrupted partway through, fully power-cycle the keyboard before attempting another media upload.
- The current renderer keeps the historical filename `tiga_windows_nowplaying_v2.py` for compatibility with the existing startup stack.
- A normal one-click Windows installer is planned; the current release still uses the build/install script described below.

---

## Requirements

- **Windows 10 or Windows 11**
- **Meletrix / Wuque Studio Zoom75 TIGA**
- Bluetooth enabled
- Keyboard visible to Windows as `Zoom75 Tiga`
- **Visual Studio 2022 C++ Build Tools**
- **Developer PowerShell for VS 2022**
- **Rust + Cargo**
- **Python** with the `py.exe` launcher
- **Pillow**

The build script can install the Python package requirement from `requirements.txt`.

---

## Installation

### 1. Download the project

Either clone the repository:

```powershell
git clone https://github.com/MichiG21/tiga-zoom75-windows-media-display.git
cd tiga-zoom75-windows-media-display
```

or use GitHub's **Code → Download ZIP** option and extract it.

### 2. Open Developer PowerShell for VS 2022

Open **Developer PowerShell for VS 2022** inside the repository folder.

### 3. Build and install

Run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\BUILD_AND_INSTALL.ps1
```

The script builds the native helpers, builds the Rust BLE uploader, checks the Python dependency, and installs the startup task.

### 4. Start the stack

```powershell
.\START_STACK.ps1
```

After installation, the project is also configured to start automatically at user logon.

---

## Everyday usage

Once installed, normal use should not require Developer PowerShell.

### Start

```powershell
.\START_STACK.ps1
```

### Stop

```powershell
.\STOP_STACK.ps1
```

### Check status

```powershell
.\STATUS_STACK.ps1
```

The Scheduled Task is named:

```text
TIGA Windows Media Display
```

It can also be started manually with:

```powershell
Start-ScheduledTask -TaskName "TIGA Windows Media Display"
```

---

## Display Control

The project includes a small Windows control utility:

```text
tiga_display_control.exe
```

It can request:

- **Automatic** — follow fresh Windows media sessions
- **Standby** — show the clock/date standby screen
- **Black Screen**
- **Standby + Shut Down** — requests standby, waits for the BLE upload, then shuts Windows down

The renderer always returns to automatic startup behavior when launched normally.

---

## How it works

```text
Windows media session (GSMTC)
        │
        ▼
tiga_windows_media_snapshot.exe
        │
        ▼
tiga_windows_nowplaying_v2.py
        │
        ├── artwork
        ├── title
        ├── artist / creator
        ├── album
        ├── duration
        └── standby clock / date
        │
        ▼
extracted/session_01_bulk_stream.bin
        │
        ▼
tiga_stream_watch_flow.exe
        │
        ├── BLE bulk upload
        ├── 4096-byte progress flow-control
        └── completion acknowledgement
        │
        ▼
Zoom75 TIGA LCD
```

More detail is available in:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- [`docs/PROTOCOL_NOTES.md`](docs/PROTOCOL_NOTES.md)

---

## BLE uploader

The uploader uses the keyboard's own progress notifications as flow-control.

Instead of sleeping a fixed amount between every write, it sends data quickly inside a 4096-byte block and waits for the keyboard's progress notification before continuing.

This reduced a full upload on the original development unit from roughly **39 seconds** to about **8 seconds**.

### Manual BLE test

From the `windows_ble_probe` folder after building:

```powershell
.\target\release\tiga_stream_watch_flow.exe --stream "..\extracted\session_01_bulk_stream.bin" --intra-block-ms 0 --confirm-flow-watch
```

The uploader intentionally remains active in **watch mode** after a successful upload.

---

## Repository layout

```text
.
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
└── reference/
```

### `windows_ble_probe/`

Rust source for the current BLE uploader and earlier BLE investigation tools.

### `extracted/`

Contains the known-good media stream template used by the renderer.

### `docs/`

Architecture, protocol notes, build notes, and recovery information.

### `reference/`

Reverse-engineering probes and supporting material from earlier development.

Large raw packet captures, private transcripts, credentials, and personal runtime logs are intentionally not included.

---

## Media-session behavior

The project relies on what each application publishes through Windows GSMTC.

Examples:

- **Spotify** normally exposes title, artist, album, thumbnail, playback state, and duration.
- **Browser media** can expose title, creator, thumbnail, and playback state.
- Some browser / website combinations may omit duration or other timeline information.

The renderer falls back gracefully when a field is unavailable.

---

## Standby and stale-session handling

Windows can keep old media-session metadata around after playback has stopped or after a reboot.

To avoid showing an old track immediately at login, the renderer:

1. shows standby first,
2. records the media state Windows exposes at startup,
3. waits until playback is proven fresh,
4. then switches to Now Playing.

Fresh playback can be detected through a new media item, a playback-state transition, or an advancing timeline when the source provides one.

---

## Privacy

The active implementation reads the **local Windows media session**.

It does not require:

- a Spotify developer account
- Spotify OAuth
- a cloud service for Now Playing metadata
- uploading your media history anywhere

Runtime logs stay local under `logs/` and are ignored by Git.

---

## Troubleshooting

### TIGA stays on the stock roar animation

Wait for the current upload to finish first.

If the custom frame never appears, check:

```powershell
.\STATUS_STACK.ps1
```

and inspect:

```text
logs\renderer.log
logs\renderer_error.log
logs\ble.log
logs\ble_error.log
logs\startup_supervisor.log
```

### BLE upload was interrupted

If an upload is stopped halfway through and later uploads time out, fully power-cycle the keyboard before trying again.

### Media is detected but duration is missing

This is usually caused by the application / browser not publishing useful timeline data through Windows GSMTC.

The project will still render the available metadata.

---

## Contributing

Issues and pull requests are welcome.

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

Useful contribution areas include:

- testing additional TIGA firmware revisions
- browser / media-session compatibility
- one-click Windows packaging
- UI / layout improvements
- uploader robustness
- faster uploads without overrunning keyboard flow-control
- protocol documentation

---

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

See [`docs/PROTOCOL_NOTES.md`](docs/PROTOCOL_NOTES.md) for details.

---

## Disclaimer

This is an unofficial community project.

Use it at your own risk. No warranty is provided. Firmware, protocol behavior, and compatibility can change between keyboard revisions or software versions.

This project is not affiliated with, sponsored by, or endorsed by Meletrix or Wuque Studio.

---

## License

TIGA Windows Media Display is released under the **MIT License**.

See [`LICENSE`](LICENSE).

Some reference material has its own copyright or license terms. See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
