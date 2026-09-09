# Zoom75 Tiga RGB565 test patch

This test patch changes **static image encoding only for the Zoom75 Tiga**.

The upstream experimental branch documents the Tiga display as 320x172 RGB565,
but the shared image encoder emits RGB565 plus an extra `0xFF` byte per pixel
(3 bytes/pixel = 165,120 bytes). This patch adds a Tiga-specific true RGB565
encoder (2 bytes/pixel = 110,080 bytes) while leaving Zoom65 V3 / TKL Dyna
static-image encoding unchanged.

Expected diagnostic when uploading a Tiga image:

```text
Zoom75 Tiga detected: using true RGB565 (2 bytes/pixel)
resizing and encoding image as RGB565 (2 Bpp) ... done (110080 bytes)
```

Build:

```powershell
cargo build --release
```

Red image test:

```powershell
.\target\release\zoom-sync.exe --zoom75-tiga set image "$HOME\Downloads\tiga-red.png"
.\target\release\zoom-sync.exe --zoom75-tiga set screen image
```

If the screen is still white, do not keep uploading repeatedly; that would point
to packet framing/commit semantics rather than the pixel stride.
