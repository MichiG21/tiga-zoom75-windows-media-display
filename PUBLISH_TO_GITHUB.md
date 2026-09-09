# Publishing this folder to GitHub

Recommended repository name:

`tiga-windows-media-display`

Recommended description:

`Windows Now Playing display for the Meletrix Zoom75 TIGA using GSMTC and a reverse-engineered BLE media upload protocol.`

Recommended topics:

`zoom75`, `tiga`, `meletrix`, `windows`, `now-playing`, `bluetooth`, `ble`, `rgb565`, `reverse-engineering`

## First push

Create an empty **public** repository on GitHub, then from this folder:

```powershell
git init
git add .
git commit -m "Initial public release"
git branch -M main
git remote add origin https://github.com/YOUR_USERNAME/tiga-windows-media-display.git
git push -u origin main
```

Do not add a second README or license in GitHub's creation form because this
folder already contains both.

## Suggested first release

Tag the first tested public version as `v0.1.0` rather than `v1.0.0`; the core
project works, but testing across more TIGA units/firmware revisions is still
limited.
