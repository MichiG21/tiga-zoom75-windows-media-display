# Contributing

Contributions are welcome, especially around Windows media-session compatibility,
BLE robustness, rendering/layout improvements, packaging, and testing on other
Zoom75 TIGA units.

## Before opening a pull request

1. Keep the project working on Windows 10/11.
2. Do not add proprietary firmware, private packet captures, API tokens, or user logs.
3. Avoid requiring the blocked/vulnerable `meletrixid.sys` driver.
4. Do not change the known-good BLE flow-control behavior without documenting and testing it.
5. If you touch the renderer, test both:
   - a media source with duration/timeline data (for example Spotify), and
   - a source that may omit timeline data (for example YouTube in Firefox).

## Build check

From **Developer PowerShell for VS 2022**:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\BUILD_AND_INSTALL.ps1
```

For code-only validation without installing the scheduled task, you can build the
C++ helpers and Rust uploader manually and run:

```powershell
py -m py_compile .\tiga_windows_nowplaying_v2.py
```

## Bug reports

Please include:

- Windows version
- keyboard model / firmware if known
- media source (Spotify, Firefox/YouTube, etc.)
- `logs/renderer_error.log`
- `logs/ble_error.log`
- whether a full keyboard power-cycle changes the behavior

Do not upload logs containing anything you consider private.
