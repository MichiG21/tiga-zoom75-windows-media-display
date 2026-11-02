### TIGA Windows Media Display v0.3.0-beta.1

Shows what Windows is playing on the LCD of a Meletrix / Wuque Studio **Zoom75 TIGA**:
title, artist, album and artwork, about 8–10 s after each change. Install with the
setup file below; no Python, Rust or Visual Studio needed. **Beta:** it has been
tested on one keyboard and one PC.

**License change:** from this release the project is licensed **GPL-2.0-only**, the
license of [meletricks](https://github.com/athdesk/meletricks), which the planned
on-keyboard work builds on. v0.1.0 and v0.2.0-beta.1 remain under MIT.

#### What's new since v0.2.0-beta.1

**Uploads that fail no longer make things worse**

- If an upload to the keyboard is interrupted, the uploader no longer retries every 8
  seconds against a keyboard stuck mid-transfer. (#4)
- **Most interrupted uploads now recover by themselves.** When an upload simply stops
  at a normal pace, the display software waits 2 minutes and tries again, up to 3 times,
  without you doing anything. In testing this recovered the display without resetting
  the keyboard.
- When an upload slows down before it stops, or the retries don't help, the uploader
  **stops and tells you**: *Status* shows `HALTED`, with the frame and block it stopped
  at and what to do. The Now Playing renderer keeps running.
- If the keyboard just can't be reached (asleep, out of range, Bluetooth off), the
  uploader keeps trying: after 8, 16, 32 and 60 s, then every 5 minutes. It picks the
  display up again by itself; you don't need to restart anything.
- The uploader now finds the keyboard's **display** by the service it offers, not by
  name. A TIGA also paired as a Bluetooth *keyboard* shows up as a second device,
  `Zoom75 Tiga BT1`, which used to be picked by mistake about half the time. It also
  remembers the device that worked last. (#14)
- Once connected, the uploader stops scanning for Bluetooth devices, so the scan no
  longer runs alongside uploads.
- **Stop Display finishes an upload in progress** before it stops (up to 30 s), instead
  of cutting it off halfway. (#20)

**The display**

- Japanese, Chinese, Korean and symbol characters show instead of empty boxes, using
  the fonts already on your PC. Japanese and Chinese titles wrap between characters.
  (#10)
- A title, artist or album written entirely in Morse code gets its decoded text as a
  smaller blue line underneath (`--no-morse-subtitle` turns it off).
- Long titles without spaces wrap inside the text area instead of running off the
  screen; a long album no longer overlaps a two-line artist; new artwork with the same
  file size as the old one is now noticed. (#10)
- If music was already playing when the display started, Now Playing appears within
  about 15 s, even for a player that never reports its position moving. (#5)
- If Windows' media information keeps failing, the display goes to standby after a
  minute instead of showing the last song forever. (#10)

**Display Control**

- Pressing the same button twice now works the second time. (#6)
- Every button checks that the command actually arrived and says so if it didn't, for
  example because antivirus blocked the write. (#15)
- **Standby + Shut Down** waits for the keyboard to confirm the standby screen before
  shutting Windows down, instead of a fixed 12 s. If the keyboard doesn't confirm in
  time, it asks first (default: don't shut down); if the command never reached the
  display software, it cancels and says why. (#7)

**Installer and startup**

- Setup stops the running display software before it replaces any file. Installing over
  a running version used to fail with "DeleteFile failed; code 5. Access is denied".
- When the display software starts, including after an install or update, the keyboard
  no longer gets the red/green test image first; the first upload is the first real
  screen.
- The Start menu's *Start Display*, *Stop Display* and *Status* windows stay open, so you
  can read what they say. Close them when you're done.

**Diagnostics**

- Logs in `logs\` keep their history across restarts and rotate at 5 MB. Every frame
  has an ID that appears in both `renderer.log` and `ble.log`. *Status* shows the last
  frame actually delivered to the keyboard, not just whether processes are running. (#9)
- `ble.log` records how long every block of an upload took, flags any block over 1 s
  as `SLOW`, warns when the first upload after connecting starts slowly, and writes a
  `TIMING` summary after every upload. Please include these lines in a bug report.
- `ble.log` also records anything else the keyboard sends during and between uploads;
  its regular status message is logged once and then counted, so the log stays small.
- The uploader waits up to 2.5 s for each block's confirmation, as in v0.2.0-beta.1. A
  longer wait was tried for a day and helped nothing.

**Documentation**

- Quick start corrected: the keyboard is used **wired over USB-C**, and the display is
  reached over Bluetooth LE **without pairing**.
- A shorter front page; architecture, build-from-source and recovery details moved to
  `docs/`.

#### Verified on the keyboard

On one TIGA, 2026-10-15 to 2026-10-22: more than 100 completed uploads, about 8.5–11 s each;
real interrupted uploads that each halted correctly; the automatic retry recovering two
interrupted uploads without a keyboard reset; Stop Display stopping cleanly six times,
including once in the middle of an upload, which it let finish; the uploader rejecting
`Zoom75 Tiga BT1` and remembering the right device, and stopping its scan once
connected; recovery on its own after the keyboard was briefly unreachable; logs with
history and frame IDs; a 34-character title wrapping; Japanese text and the Morse-code
band displaying; the per-block timing and notification lines. Display Control: Lock
Standby, the same button again and Resume Automatic were each confirmed and reached the
keyboard, and **Standby + Shut Down** waited for the upload and then shut Windows down.

On 2026-10-25 a test build of this installer ran on the same keyboard, installed over
v0.2.0-beta.1: uploads, *Status*, a Japanese title, *Stop Display*, and an automatic retry
that recovered an interrupted upload by itself. The three installer and startup fixes
above come from that test.

On 2026-11-02 a new test build with those fixes was installed over a running
v0.2.0-beta.1 on the same keyboard. Setup stopped v0.2 first and reported no locked file;
the keyboard got no red/green image, and the first upload, the first real screen, took
9.08 s; the *Status* window stayed open and showed `OK`.

**Not yet verified on the keyboard:** the 15-second rule for players that never report
their position moving (no such player is known).

#### Known problems

- **Uploads are occasionally interrupted, cause unknown.** It happens most often on the
  **first upload after the display software starts** (about 1 in 3 in testing, against
  about 1 in 30 for later uploads), whether or not the keyboard was reset. A start
  happens when you sign in, after *Start Display*, and after an update. Most of these now
  recover through the automatic retry within a few minutes; if the first song after a
  start doesn't appear, check *Status*.
- **After a restart the keyboard can be invisible for 20–70 s.** The uploader keeps
  trying and finds it; nothing to do.
- **The only way to reset a stuck keyboard is to disconnect its battery.** The TIGA has
  no power switch, and unplugging USB doesn't reset it (it keeps running on its
  battery). See below.
- Each update takes 8–10 s to reach the keyboard, and there is no live progress bar.
  Both are limits of the keyboard's upload protocol, not bugs.
- **Uninstalling leaves files behind** in
  `%LOCALAPPDATA%\Programs\TIGA Windows Media Display`: the logs in `logs\` and a few
  runtime files in `app\`. Delete the folder by hand afterwards (keep `logs\` first if
  you want to report a problem).
- **Norton CyberCapture may hold or sandbox the programs the first time they run**, for
  a minute or so. "Access is denied" lines during that time are harmless. (An error box
  from Setup is a different matter; see the next point.)
- **Close Display Control before updating.** Setup stops the display software, but
  Display Control is a separate window and can keep its own file locked. If Setup says
  it can't replace `tiga_display_control.exe`, close the window and choose *Retry*.
- **A halt survives reboots.** After an upload that slowed down before it stopped, or
  any other `HALTED`, nothing is uploaded again until you run *Start Display*; restarting
  Windows doesn't clear it. See below.
- **Bluetooth must be on**, even with the keyboard plugged in over USB-C. When it's off,
  the log only says "The device is not ready for use". Turn Bluetooth on; the uploader
  keeps trying and picks the display up within 5 minutes.

#### If the display stops updating and Status says HALTED

An upload was interrupted in a way the software doesn't retry by itself (or three
retries didn't help), and the keyboard may be holding a half-finished transfer.

1. Open the keyboard's case and **disconnect the battery from the PCB for at least
   30 s**, pressing a few keys while it's disconnected. 10–15 s was not enough in
   testing. Reconnect it.
2. Run **Start Display** from the Start menu. That clears the halt and the display
   picks up the current song.

If Status shows HALTED again right away without a new upload, send the files in
`logs\` (especially `ble.log` and `uploader_halted.json`) with an issue.

#### Installing

Download `TIGA-Windows-Media-Display-Setup-v0.3.0-beta.1.exe` and run it. It installs
for your user only (no admin) and starts at sign-in. Plug the keyboard in over USB-C as
usual; the display is reached over Bluetooth LE, with no pairing needed. The installer
is not code-signed, so Windows SmartScreen may warn; choose *More info → Run anyway*.

To update, run the new setup file over the installed version. It stops the running
version first, which can take up to 30 s.

**Updating from v0.2.0-beta.1:** v0.2 can only be stopped abruptly, and stopping it in
the middle of an upload can leave the keyboard needing a battery disconnect (see above).
Run Setup while nothing is being sent: the keyboard shows the current song and it hasn't
changed for 15 s. If a Japanese, Chinese or Korean title is playing, pause or play
something else first and wait a minute: v0.2 keeps restarting on such titles, sending
a new image each time (fixed in this release).
