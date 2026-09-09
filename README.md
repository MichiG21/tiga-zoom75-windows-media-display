# TIGA Windows Media Display

![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?logo=windows)
![License](https://img.shields.io/badge/License-MIT-green)
![Hardware](https://img.shields.io/badge/Hardware-Zoom75%20TIGA-7A5AF8)
![Status](https://img.shields.io/badge/Status-Public%20Beta-orange)
![Release](https://img.shields.io/github/v/release/MichiG21/tiga-zoom75-windows-media-display?include_prereleases&label=release)

Turn the **Meletrix / Wuque Studio Zoom75 TIGA** LCD into a Windows-wide **Now Playing** display.

TIGA Windows Media Display reads the active Windows media session, renders artwork and metadata for the TIGA's **320×172** screen, and uploads the frame over a reverse-engineered BLE media-transfer path.

**No Spotify API. No cloud account. No Meletrix kernel driver.**

> **Unofficial community project.** This project is not affiliated with or endorsed by Meletrix / Wuque Studio.

## Download

### [Download the Windows installer →](https://github.com/MichiG21/tiga-zoom75-windows-media-display/releases)

For normal use, download the newest file named similar to:

```text
TIGA-Windows-Media-Display-Setup-v0.2.0-beta.1.exe
```

from the **Assets** section of the newest release.

The installer bundles the application and its runtime dependencies, so normal users do **not** need to install:

- Python
- Pillow
- Rust or Cargo
- Visual Studio / C++ Build Tools
- Developer PowerShell

The current public installer is a **beta**. Testing and issue reports are very welcome.

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
- Displays artwork, title, artist / creator, album, and total duration when available.
- Large clock-and-date standby screen.
- Ignores stale remembered media at login until playback is proven fresh.
- Returns to standby after playback has been paused or stopped for **60 seconds**.
- Uses keyboard progress notifications as BLE flow-control instead of blindly pacing writes.
- Full custom-frame upload typically takes about **8 seconds** on the original development unit.
- Includes a Windows display-control utility for automatic mode, standby, black screen, and standby + shutdown.
- Starts automatically at user logon through a Windows Scheduled Task.
- Does **not** require the blocked / vulnerable `meletrixid.sys` kernel driver.
- Installs per-user under LocalAppData and does not require an administrator install.

---

## Quick start

### 1. Pair the keyboard

Make sure Bluetooth is enabled and the keyboard is visible to Windows as:

```text
Zoom75 Tiga
```

### 2. Install

Download the newest Windows installer from the [Releases page](https://github.com/MichiG21/tiga-zoom75-windows-media-display/releases) and run it.

The installer places the application under:

```text
%LOCALAPPDATA%\Programs\TIGA Windows Media Display
```

and creates a Scheduled Task named:

```text
TIGA Windows Media Display
```

The display stack is started automatically after installation and again at user logon.

### 3. Use the Start Menu shortcuts

The installer adds shortcuts for:

- **Display Control**
- **Start Display**
- **Stop Display**
- **Status**

That is all normal users need for everyday use.

> Windows SmartScreen may warn about early beta installers because the executable is not currently code-signed. Only download releases from this GitHub repository.

---

## Display Control

The included display-control utility can request:

- **Automatic** — follow fresh Windows media sessions
- **Standby** — show the clock/date standby screen
- **Black Screen**
- **Standby + Shut Down** — requests standby, waits for the BLE upload, then shuts Windows down

The renderer returns to normal automatic behavior when launched normally.

---

## Current status

The project is functional on the original Windows development system and Zoom75 TIGA unit and now has a public Windows installer.

The current release is still a **public beta**. Additional testing across Windows systems, Bluetooth adapters, TIGA firmware revisions, and media applications is welcome.

### Known behavior / limitations

- Firefox + YouTube may expose title and thumbnail but no usable total duration.
- The TIGA can briefly show its built-in roar animation during a custom-image upload.
- If a BLE upload is interrupted partway through, fully power-cycle the keyboard before attempting another media upload.
- The renderer keeps the historical filename `tiga_windows_nowplaying_v2.py` internally for compatibility with the existing stack.
- An existing older/manual installation may have its `TIGA Windows Media Display` Scheduled Task replaced by the installer.

Please report installation, Bluetooth, display, startup, or media-detection problems through [GitHub Issues](https://github.com/MichiG21/tiga-zoom75-windows-media-display/issues).

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

---

## BLE uploader

The uploader uses the keyboard's own progress notifications as flow-control.

Instead of sleeping a fixed amount between every write, it sends data quickly inside a 4096-byte block and waits for the keyboard's progress notification before continuing.

This reduced a full upload on the original development unit from roughly **39 seconds** to about **8 seconds**.

The uploader intentionally remains active in **watch mode** after a successful upload so later media changes can be transferred without rebuilding the connection stack from scratch.

---

## Privacy

The active implementation reads the **local Windows media session**.

It does not require:

- a Spotify developer account
- Spotify OAuth
- a cloud service for Now Playing metadata
- uploading your media history anywhere

Runtime logs stay local and are ignored by Git.

---

## Troubleshooting

### TIGA stays on the stock roar animation

Wait for the current upload to finish first.

If the custom frame never appears, use the installed **Status** shortcut and inspect the local logs in the installation directory.

### BLE upload was interrupted

If an upload is stopped halfway through and later uploads time out, fully power-cycle the keyboard before trying again.

### Media is detected but duration is missing

This is usually caused by the application / browser not publishing useful timeline data through Windows GSMTC. The project will still render the available metadata.

### Existing manual installation

The beta installer uses the Scheduled Task name:

```text
TIGA Windows Media Display
```

If an older manual version already uses that task name, the installer may replace that task with the packaged installation.

---

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

From the `windows_ble_probe` folder after building:

```powershell
.\target\release\tiga_stream_watch_flow.exe --stream "..\extracted\session_01_bulk_stream.bin" --intra-block-ms 0 --confirm-flow-watch
```

---

## Repository layout

```text
.
├── .github/workflows/
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
└── reference/
```

### `installer/`

Windows installer definition and packaged-runtime helper scripts.

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

## Contributing

Issues and pull requests are welcome.

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

Useful contribution areas include:

- testing additional TIGA firmware revisions
- Bluetooth adapter / Windows compatibility
- browser / media-session compatibility
- installer robustness
- UI / layout improvements
- uploader robustness and speed
- protocol documentation

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
