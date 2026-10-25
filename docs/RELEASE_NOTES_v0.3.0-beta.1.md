# v0.3.0-beta.1 — release draft

**Draft. No tag and no release have been created.** Part 1 is for Michi: what blocks the
release today. Part 2 is the release text for users, ready to paste once Part 1 is done.

Written 2026-10-16 against `main` at `e8f7211` (after #17); updated the same evening for
`main` at `56c3825` (after #18 and #22); brought up to date on 2026-10-22 for `main` at
`8b2011a` (after #31); Phase 8 results and the three fixes they led to added on
2026-10-25. The version name is Michi's choice. The newest release is still
`v0.2.0-beta.1` (2026-09-09).

---

## Part 1 — What blocks v0.3.0-beta.1 today

**(H)** needs the keyboard; **(M)** needs Michi's decision.

1. **(H) Phase 8, the installer end to end: run on 2026-10-25; one retest left.** Build
   `vpr-32` (installer CI run 39853101947, `4d06fdf`), installed over a running
   v0.2.0-beta.1 on this keyboard. Results:
   - **Upgrade lock found, and fixed but not yet retested.** Installing over the running
     v0.2 failed with "DeleteFile failed; code 5. Access is denied" on
     `app\_internal\tiga_stream_watch_flow.exe`: `[Files]` copied before the `[Run]` step
     that stops the stack. After v0.2's *Stop Display*, Setup ran through (09:33). Fixed in
     `e0add1b`: Setup now runs the installed version's `STOP_STACK.ps1` before copying
     anything and waits until its executables have exited. **The retest of this fix on
     hardware is the one Phase 8 item left**; steps in `docs/NOTES_FOR_MICHI.md`,
     "Phase 8 on hardware, 2026-10-25".
   - **Worked on the packaged build:** uploads (`UPLOAD COMPLETE`, 8.6–10.7 s), *Status*,
     a Japanese title (`トーキョーレギー`, glyphs; `FONT fallbacks: YuGothB.ttc, …`, no
     missing characters), and a clean *Stop Display* (09:41:21, uploader exited on request
     after 0.3 s).
   - **First packaged automatic retry:** a sudden failure at 09:50:44 (block 5, last ACKed
     4) was retried at 09:52:44 and succeeded; the frame was delivered at 09:52:55. The
     start before it (09:44) had halted on a slowing failure, which by design is not
     retried.
   - **Two more problems, fixed:** the first upload after install was the red/green
     template (`frame=unknown(no-sidecar)`, crc32 `57e8e0cf`, 64 s), because the packaged
     renderer took 36 s to its first frame (fixed in `319b426`; for upgrades and
     reinstalls too since Setup deletes a leftover `.frame`); and the Start menu
     shortcuts *Start Display*, *Stop Display* and *Status* closed their window at once
     (fixed in `7d086bc`). Neither is retested on hardware yet; the same retest covers
     both.
   - **v0.2.0-beta.1 crash-loops on Japanese titles:** its renderer exited 16 times between
     09:27:29 and 09:31:00, each time restarted after 8 s, and v0.2 completed 14 uploads in
     those minutes. That is bug G, fixed in this release; it matters for the upgrade advice
     in Part 2.
   - **The uninstaller leaves runtime files** in `app\` (the stream's sidecars,
     `session_01_bulk_stream.windows_nowplaying.bin`, the thumbnail and preview) and all
     of `logs\`. Listed under "Known problems" in Part 2.
2. **(H, optional) Phase 4, killed uploader:** `Stop-Process -Name tiga_stream_watch_flow
   -Force` about 3 s into an upload. Expect HALTED from the session marker. Never run on
   hardware. **It leaves the keyboard needing a battery disconnect**, so do it last in a
   session. **(M)** whether it must pass before release; the marker path is covered by
   offline tests.
3. **(M) After the Phase 8 retest:** delete the one remaining *(before publishing)* note in
   Part 2, under "Installing". The other one, under "Verified on the keyboard", now has
   the 2026-10-25 result.

**Done since the first draft:** the version name (v0.3.0-beta.1; the checklist's title and
Phase 7 tag now say so); `CHANGELOG.md` is complete; **#7 verified on hardware on
2026-10-16 at 23:07** (Standby + Shut Down waited for the upload, then shut Windows down;
PROJECT_HISTORY §7.1) and **closed on 2026-10-18**; every issue is closed (none open);
**#20 clean stop merged and verified live on 2026-10-22** (PROJECT_HISTORY §7.1, seventh
part), so it is now in this release instead of under "Known problems". Earlier: the
2500 ms block timeout and per-block timing (#18, #19/#22), and the "turn the keyboard off"
wording.

Already fine: CI is green on `main`, `Cargo.lock` is committed and every build uses
`--locked`, the installer workflow builds on a `v*` tag push and uploads to the release.

---

## Part 2 — Release text (draft)

> ### TIGA Windows Media Display v0.3.0-beta.1
>
> Shows what Windows is playing on the LCD of a Meletrix / Wuque Studio **Zoom75 TIGA**:
> title, artist, album and artwork, about 8–10 s after each change. Install with the
> setup file below; no Python, Rust or Visual Studio needed. **Beta:** it has been
> tested on one keyboard and one PC.
>
> **License change:** from this release the project is licensed **GPL-2.0-only**, the
> license of [meletricks](https://github.com/athdesk/meletricks), which the planned
> on-keyboard work builds on. v0.1.0 and v0.2.0-beta.1 remain under MIT.
>
> #### What's new since v0.2.0-beta.1
>
> **Uploads that fail no longer make things worse**
>
> - If an upload to the keyboard is interrupted, the uploader no longer retries every 8
>   seconds against a keyboard stuck mid-transfer. (#4)
> - **Most interrupted uploads now recover by themselves.** When an upload simply stops
>   at a normal pace, the display software waits 2 minutes and tries again, up to 3 times,
>   without you doing anything. In testing this recovered the display without resetting
>   the keyboard.
> - When an upload slows down before it stops, or the retries don't help, the uploader
>   **stops and tells you**: *Status* shows `HALTED`, with the frame and block it stopped
>   at and what to do. The Now Playing renderer keeps running.
> - If the keyboard just can't be reached (asleep, out of range, Bluetooth off), the
>   uploader keeps trying: after 8, 16, 32 and 60 s, then every 5 minutes. It picks the
>   display up again by itself; you don't need to restart anything.
> - The uploader now finds the keyboard's **display** by the service it offers, not by
>   name. A TIGA also paired as a Bluetooth *keyboard* shows up as a second device,
>   `Zoom75 Tiga BT1`, which used to be picked by mistake about half the time. It also
>   remembers the device that worked last. (#14)
> - Once connected, the uploader stops scanning for Bluetooth devices, so the scan no
>   longer runs alongside uploads.
> - **Stop Display finishes an upload in progress** before it stops (up to 30 s), instead
>   of cutting it off halfway. (#20)
>
> **The display**
>
> - Japanese, Chinese, Korean and symbol characters show instead of empty boxes, using
>   the fonts already on your PC. Japanese and Chinese titles wrap between characters.
>   (#10)
> - A title, artist or album written entirely in Morse code gets its decoded text as a
>   smaller blue line underneath (`--no-morse-subtitle` turns it off).
> - Long titles without spaces wrap inside the text area instead of running off the
>   screen; a long album no longer overlaps a two-line artist; new artwork with the same
>   file size as the old one is now noticed. (#10)
> - If music was already playing when the display started, Now Playing appears within
>   about 15 s, even for a player that never reports its position moving. (#5)
> - If Windows' media information keeps failing, the display goes to standby after a
>   minute instead of showing the last song forever. (#10)
>
> **Display Control**
>
> - Pressing the same button twice now works the second time. (#6)
> - Every button checks that the command actually arrived and says so if it didn't, for
>   example because antivirus blocked the write. (#15)
> - **Standby + Shut Down** waits for the keyboard to confirm the standby screen before
>   shutting Windows down, instead of a fixed 12 s. If the keyboard doesn't confirm in
>   time, it asks first (default: don't shut down); if the command never reached the
>   display software, it cancels and says why. (#7)
>
> **Installer and startup**
>
> - Setup stops the running display software before it replaces any file. Installing over
>   a running version used to fail with "DeleteFile failed; code 5. Access is denied".
> - When the display software starts, including after an install or update, the keyboard
>   no longer gets the red/green test image first; the first upload is the first real
>   screen.
> - The Start menu's *Start Display*, *Stop Display* and *Status* windows stay open, so you
>   can read what they say. Close them when you're done.
>
> **Diagnostics**
>
> - Logs in `logs\` keep their history across restarts and rotate at 5 MB. Every frame
>   has an ID that appears in both `renderer.log` and `ble.log`. *Status* shows the last
>   frame actually delivered to the keyboard, not just whether processes are running. (#9)
> - `ble.log` records how long every block of an upload took, flags any block over 1 s
>   as `SLOW`, warns when the first upload after connecting starts slowly, and writes a
>   `TIMING` summary after every upload. Please include these lines in a bug report.
> - `ble.log` also records anything else the keyboard sends during and between uploads;
>   its regular status message is logged once and then counted, so the log stays small.
> - The uploader waits up to 2.5 s for each block's confirmation, as in v0.2.0-beta.1. A
>   longer wait was tried for a day and helped nothing.
>
> **Documentation**
>
> - Quick start corrected: the keyboard is used **wired over USB-C**, and the display is
>   reached over Bluetooth LE **without pairing**.
> - A shorter front page; architecture, build-from-source and recovery details moved to
>   `docs/`.
>
> #### Verified on the keyboard
>
> On one TIGA, 2026-10-15 to 2026-10-22: more than 100 completed uploads, about 8.5–11 s each;
> real interrupted uploads that each halted correctly; the automatic retry recovering two
> interrupted uploads without a keyboard reset; Stop Display stopping cleanly six times,
> including once in the middle of an upload, which it let finish; the uploader rejecting
> `Zoom75 Tiga BT1` and remembering the right device, and stopping its scan once
> connected; recovery on its own after the keyboard was briefly unreachable; logs with
> history and frame IDs; a 34-character title wrapping; Japanese text and the Morse-code
> band displaying; the per-block timing and notification lines. Display Control: Lock
> Standby, the same button again and Resume Automatic were each confirmed and reached the
> keyboard, and **Standby + Shut Down** waited for the upload and then shut Windows down.
>
> On 2026-10-25 a test build of this installer ran on the same keyboard, installed over
> v0.2.0-beta.1: uploads, *Status*, a Japanese title, *Stop Display*, and an automatic retry
> that recovered an interrupted upload by itself. The three installer and startup fixes
> above come from that test.
>
> **Not yet verified on the keyboard:** the 15-second rule for players that never report
> their position moving (no such player is known).
>
> #### Known problems
>
> - **Uploads are occasionally interrupted, cause unknown.** It happens most often on the
>   **first upload after the display software starts** (about 1 in 3 in testing, against
>   about 1 in 30 for later uploads), whether or not the keyboard was reset. A start
>   happens when you sign in, after *Start Display*, and after an update. Most of these now
>   recover through the automatic retry within a few minutes; if the first song after a
>   start doesn't appear, check *Status*.
> - **After a restart the keyboard can be invisible for 20–70 s.** The uploader keeps
>   trying and finds it; nothing to do.
> - **The only way to reset a stuck keyboard is to disconnect its battery.** The TIGA has
>   no power switch, and unplugging USB doesn't reset it (it keeps running on its
>   battery). See below.
> - Each update takes 8–10 s to reach the keyboard, and there is no live progress bar.
>   Both are limits of the keyboard's upload protocol, not bugs.
> - **Uninstalling leaves files behind** in
>   `%LOCALAPPDATA%\Programs\TIGA Windows Media Display`: the logs in `logs\` and a few
>   runtime files in `app\`. Delete the folder by hand afterwards (keep `logs\` first if
>   you want to report a problem).
>
> #### If the display stops updating and Status says HALTED
>
> An upload was interrupted in a way the software doesn't retry by itself (or three
> retries didn't help), and the keyboard may be holding a half-finished transfer.
>
> 1. Open the keyboard's case and **disconnect the battery from the PCB for at least
>    30 s**, pressing a few keys while it's disconnected. 10–15 s was not enough in
>    testing. Reconnect it.
> 2. Run **Start Display** from the Start menu. That clears the halt and the display
>    picks up the current song.
>
> If Status shows HALTED again right away without a new upload, send the files in
> `logs\` (especially `ble.log` and `uploader_halted.json`) with an issue.
>
> #### Installing
>
> Download `TIGA-Windows-Media-Display-Setup-v0.3.0-beta.1.exe` and run it. It installs
> for your user only (no admin) and starts at sign-in. Plug the keyboard in over USB-C as
> usual; the display is reached over Bluetooth LE, with no pairing needed. The installer
> is not code-signed, so Windows SmartScreen may warn; choose *More info → Run anyway*.
>
> To update, run the new setup file over the installed version. It stops the running
> version first, which can take up to 30 s. *(Before publishing: delete this note once the
> Phase 8 retest has shown it working over a running v0.2.0-beta.1.)*
>
> **Updating from v0.2.0-beta.1:** v0.2 can only be stopped abruptly, and stopping it in
> the middle of an upload can leave the keyboard needing a battery disconnect (see above).
> Run Setup while nothing is being sent: the keyboard shows the current song and it hasn't
> changed for 15 s. If a Japanese, Chinese or Korean title is playing, pause or play
> something else first and wait a minute: v0.2 keeps restarting on such titles, sending
> a new image each time (fixed in this release).
