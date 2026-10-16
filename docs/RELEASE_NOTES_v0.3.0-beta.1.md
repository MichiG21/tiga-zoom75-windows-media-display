# v0.3.0-beta.1 — release draft

**Draft. No tag and no release have been created.** Part 1 is for Michi: what blocks the
release today. Part 2 is the release text for users, ready to paste once Part 1 is done.

Written 2026-10-16 against `main` at `e8f7211` (after #17). The newest release is still
`v0.2.0-beta.1` (2026-09-09).

---

## Part 1 — What blocks v0.3.0-beta.1 today

In the order to do them. **(M)** needs Michi's decision; **(H)** needs the keyboard;
**(C)** Claude Code can do it once told to.

1. **(M) Merge #18 (block timeout back to 2500 ms).** `main` still has the 10000 ms default
   from `224e771`. A release cut from `main` today ships 10000 ms, which §7.1 found rescued
   nothing and only delays the halt by about a minute. The unattended run of 2026-10-16
   opened #18 with green checks but could not merge it (see NOTES_FOR_MICHI). Then
   **#19** (per-block timing, logging only), retargeted to `main`. Both are small; #19 is
   what makes a field report from a beta user useful.
2. **(M) Version name.** `RELEASE_CHECKLIST.md` is still titled `v0.2.0-beta.2`, and its
   Phase 7 tags `v0.2.0-beta.2`. The task and these notes call it `v0.3.0-beta.1`. Pick
   one; then the checklist's title and Phase 7 tag command need that one word changed.
3. **(C) CHANGELOG.** `## Unreleased` lists only the font fallback and the Morse subtitle
   (#19 adds the block-timeout and timing lines). Bugs A–G, device selection, the halt
   instruction and the display-control fixes (#11–#16) are not in it. Part 2 below is a
   complete list to copy from. Not done in this run, to avoid a conflict with #19's
   CHANGELOG lines.
4. **(H) Hardware, from the checklist,** on the build you will release:
   - **Phase 3, bug C via the GUI and #15:** Display Control → Standby, Standby again,
     then Standby + Shut Down. The display-control fix (`dbc902f`) has never run on
     hardware, and #7's ack-then-shutdown path has never completed. This is the one
     user-facing feature with no hardware run at all.
   - **Phase 4, killed uploader:** `Stop-Process -Name tiga_stream_watch_flow -Force` about
     3 s into an upload, by hand. Expect HALTED from the session marker. Never run on
     hardware (every real halt so far was a code 20). **It leaves the keyboard needing a
     battery disconnect**, so do it last in a session.
   - **Phase 3, bug B:** music already playing → STOP_STACK → START_STACK. It has only ever
     released through the old path; that's still a pass for the release.
   - **Phase 8, the installer end to end.** The packaged supervisor, START/STOP/STATUS and
     the PyInstaller renderer have had every change since September (bug A policy, halt
     file, device selection, fonts) but **have never run on this keyboard**; only the dev
     layout has. CI builds the installer on every PR but can't run it. The font fallback
     uses the PC's own `%WINDIR%\Fonts`, so nothing to bundle, but check a Japanese title
     on the packaged build once. Also install it **over** v0.2.0-beta.1 once: the
     installer runs STOP_STACK only after copying files (`CloseApplications=no`), so
     whether running old executables block the copy is untested.
5. **(M) The three "turn the keyboard off" spots** (README ~line 275, RETRY_RECOVERY
   step 4, RELEASE_CHECKLIST Phase 4's last step). The TIGA has no power switch, so "turn
   the keyboard off" needs your answer: the mode toggle (BT / 2.4 GHz / wired), or
   Bluetooth off on the PC? Not a hard blocker, but README line 275 is user-facing.
6. **(M) Issues.** `docs/ISSUE_TRIAGE_2026-10-16.md` (local draft) proposes closing #4, #5,
   #6, #8, #9, #10 and #14 at the release and keeping #7 and #15 open until step 4's GUI
   session. Ready-to-paste comments are in it.
7. **Not for this release:** #20 (clean stop). It changes the stop path and needs its own
   hardware test (`docs/design/CLEAN_STOP.md`). Release without it and say so under
   "Known problems".

Already fine: CI is green on `main`, `Cargo.lock` is committed and every build uses
`--locked`, the installer workflow builds on a `v*` tag push, and the stale text fixed on
2026-10-16 (codes 10/11 never halt; Phase 5 decided; test counts) is in the checklist.

---

## Part 2 — Release text (draft)

> ### TIGA Windows Media Display v0.3.0-beta.1
>
> Shows what Windows is playing on the LCD of a Meletrix / Wuque Studio **Zoom75 TIGA**:
> title, artist, album and artwork, about 8–10 s after each change. Install with the
> setup file below; no Python, Rust or Visual Studio needed. **Beta:** it has been
> tested on one keyboard and one PC.
>
> #### What's new since v0.2.0-beta.1
>
> **Uploads that fail no longer make things worse**
>
> - If an upload to the keyboard is interrupted, the uploader now **stops and tells
>   you**, instead of retrying every 8 seconds against a keyboard stuck mid-transfer.
>   *Status* shows `HALTED`, with the frame and block it stopped at and what to do. The
>   Now Playing renderer keeps running. (#4)
> - If the keyboard just can't be reached (asleep, out of range, Bluetooth off), the
>   uploader keeps trying: after 8, 16, 32 and 60 s, then every 5 minutes. It picks the
>   display up again by itself; you don't need to restart anything.
> - The uploader now finds the keyboard's **display** by the service it offers, not by
>   name. A TIGA also paired as a Bluetooth *keyboard* shows up as a second device,
>   `Zoom75 Tiga BT1`, which used to be picked by mistake about half the time. It also
>   remembers the device that worked last. (#14)
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
> **Diagnostics**
>
> - Logs in `logs\` keep their history across restarts and rotate at 5 MB. Every frame
>   has an ID that appears in both `renderer.log` and `ble.log`. *Status* shows the last
>   frame actually delivered to the keyboard, not just whether processes are running. (#9)
> - *(if #19 is merged)* `ble.log` records how long every block of an upload took, flags
>   any block over 1 s as `SLOW`, warns when the first upload after connecting starts
>   slowly, and writes a `TIMING` summary after every upload.
> - *(if #18 is merged)* The uploader waits up to 2.5 s for each block's confirmation, as
>   in v0.2.0-beta.1. A longer wait was tried for a day and helped nothing.
>
> #### Verified on the keyboard
>
> On 2026-10-15/16, on one TIGA: more than 70 completed uploads, 8.4–10.4 s each; six real interrupted
> uploads that each halted correctly and recovered after a reset; the uploader rejecting
> `Zoom75 Tiga BT1` and remembering the right device; recovery on its own after the
> keyboard was briefly unreachable; logs with history and frame IDs; a 34-character title
> wrapping; Japanese text and the Morse-code band displaying. Repeated commands were
> verified by writing the command file directly.
>
> **Not yet verified on the keyboard:** Display Control's new "did it arrive" check and
> the full Standby + Shut Down sequence; the 15-second rule for players that never report
> their position moving (no such player is known); the installer build itself on this
> release *(update before publishing)*.
>
> #### Known problems
>
> - **Uploads are occasionally interrupted, cause unknown.** It happens most often on the
>   **first upload after the display software restarts** without the keyboard being reset
>   (3 of 9 such restarts in testing, against 0 of 4 after a reset). A restart happens
>   when you sign in, after *Stop Display* / *Start Display*, and after an update. If the
>   first song after a restart doesn't appear, check *Status*.
> - **After a restart the keyboard can be invisible for 20–70 s.** The uploader keeps
>   trying and finds it; nothing to do.
> - **The only way to reset a stuck keyboard is to disconnect its battery.** The TIGA has
>   no power switch, and unplugging USB doesn't reset it (it keeps running on its
>   battery). See below.
> - Each update takes 8–10 s to reach the keyboard, and there is no live progress bar.
>   Both are limits of the keyboard's upload protocol, not bugs.
>
> #### If the display stops updating and Status says HALTED
>
> An upload was interrupted, and the keyboard may be holding a half-finished transfer.
> The software deliberately doesn't retry.
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
> for your user only (no admin) and starts at sign-in. The installer is not code-signed,
> so Windows SmartScreen may warn; choose *More info → Run anyway*.
> *(Before publishing: confirm that installing over v0.2.0-beta.1 works, and add a line
> here.)*
