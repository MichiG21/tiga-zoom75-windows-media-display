# Release checklist — v0.3.0-beta.1

**This one is for you, by hand.** Not a Claude Code task file. Every step needs either
your eyes on the keyboard screen or a decision only you can make.

Named `RELEASE_CHECKLIST.md` rather than `TASKS_*` so Claude Code doesn't mistake it for
a job to run.

Work top to bottom. **If a step marked STOP fails, don't continue** — go to Rollback.

---

## Phase 0 — Protect the working setup

- [ ] Zip `C:\Users\Regal\source\tiga_ble_re_tools` to somewhere **outside** `source\`,
      e.g. `C:\Users\Regal\Documents\tiga_ble_re_tools_backup_2026-10-15.zip`

That folder is still the only installation known to work on real hardware, and it has
compiled binaries matching the old code. Everything below can replace its scheduled task.

- [ ] Note which keyboard screen you're on now, so you can tell if something changes

- [ ] Allow PowerShell scripts to run, once, for your user:

```
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
```

Windows blocks `.ps1` files by default — without this every `START_STACK` /
`STOP_STACK` / `STATUS_STACK` call fails with `UnauthorizedAccess`. No admin needed.
The one-off alternative is `powershell -ExecutionPolicy Bypass -File .\SCRIPT.ps1`
in front of every script below.

---

## Phase 1 — Fresh clone and build

- [ ] Open **Developer PowerShell for VS 2022** (not plain PowerShell — the C++ and Rust
      builds need it)

```
cd C:\Users\Regal\source
git clone https://github.com/MichiG21/tiga-zoom75-windows-media-display tiga-fresh-test
cd tiga-fresh-test
```

- [ ] Confirm you're on `main` and it includes tonight's work:

```
git log --oneline -8
```

You should see the bug A commits (`Classify uploader failures…`, `Stop restarting the
uploader…`) and the merge.

- [ ] Run the offline tests before installing anything:

Run these **one at a time**. Note the `cd` lines — several of these only work from the
right folder, and the error you get otherwise ("the argument … does not exist") looks
like a failure but isn't one.

```
cd C:\Users\Regal\source\tiga-fresh-test
py -m unittest discover -s tests
```

```
cd C:\Users\Regal\source\tiga-fresh-test\windows_ble_probe
cargo test --release --locked --bin tiga_stream_watch_flow
```

```
cd C:\Users\Regal\source\tiga-fresh-test
powershell -ExecutionPolicy Bypass -File .\tests\test_supervisor_logging.ps1
```

```
cd C:\Users\Regal\source\tiga-fresh-test
powershell -ExecutionPolicy Bypass -File .\tests\test_supervisor_retry.ps1
```

Expect roughly (`main`, 2026-10-16): 68 Python cases, 40 Rust tests, and the two
PowerShell suites all green.

**STOP if any genuinely fail.** A fresh clone failing tests that passed locally means
something didn't get committed. A "file does not exist" error is a wrong-folder problem,
not a test failure — fix the `cd` and rerun.

- [ ] **Build the uploader first.** `cargo test` does not produce
      `target\release\tiga_stream_watch_flow.exe` — it builds a separate test binary
      under `deps\`. In a fresh clone that `.exe` does not exist yet, and you will get
      "is not recognized as the name of a cmdlet" if you skip this.

```
cd C:\Users\Regal\source\tiga-fresh-test\windows_ble_probe
cargo build --release
```

First run takes a minute or two.

- [ ] **Seed the stream file.** A fresh clone has no
      `extracted\session_01_bulk_stream.bin` — it is gitignored now and seeded from the
      committed template. The supervisors do this automatically at startup; a manual dry
      run does not, and you will get `EXIT code=30 … cannot find the file specified`.

```
cd C:\Users\Regal\source\tiga-fresh-test
Copy-Item .\extracted\session_01_bulk_stream.redgreen-backup.bin .\extracted\session_01_bulk_stream.bin
```

- [ ] Dry run the **new** uploader (this is the one that was never run):

```
cd C:\Users\Regal\source\tiga-fresh-test\windows_ble_probe
.\target\release\tiga_stream_watch_flow.exe --stream ..\extracted\session_01_bulk_stream.bin
```

Expect a timestamp and `run_id=` on the first line — that's how you know it's the new
build. Then `Validated: 220649 bytes, 1239 writes, 53 x 4096-byte progress blocks`.
Compare against `docs/design/RETRY_RECOVERY.md` step 0.3.

**STOP if the first line has no run_id** — you're running an old binary again.

- [ ] Stop the current stack before installing over it:

```
cd C:\Users\Regal\source\tiga_ble_re_tools
.\STOP_STACK.ps1
```

- [ ] Install from the fresh clone:

```
cd C:\Users\Regal\source\tiga-fresh-test
.\BUILD_AND_INSTALL.ps1
```

This re-points the scheduled task at the new folder.

---

## Phase 2 — Does it work at all

- [ ] Start playing something
- [ ] `.\START_STACK.ps1`
- [ ] Within about 10 seconds, Now Playing should appear on the keyboard

**STOP if nothing appears after a minute.** Check `.\STATUS_STACK.ps1` and the logs —
which now actually keep their history.

- [ ] `.\STATUS_STACK.ps1` — should show a rendered frame and a delivered frame, verdict OK
- [ ] Change track → display updates in roughly 8 seconds

---

## Phase 3 — The fixes you can check quickly

**Bug B — already-playing media** (the one most likely to have hit Taylor)

- [ ] Music playing → `.\STOP_STACK.ps1` → wait → `.\START_STACK.ps1`
- [ ] Now Playing should appear within ~15 seconds, not stay on standby

**Bug G — the crash nobody knew about**

- [ ] Play something with a Japanese or Cyrillic title
- [ ] It should render (characters may be boxes — that's a font issue, not a crash)
- [ ] `.\STATUS_STACK.ps1` still OK, renderer still running

This is the `UnicodeEncodeError` that killed the renderer in a restart loop. If the
display freezes here, that fix didn't take.

- [ ] Play something with a very long unbroken title → text should wrap, not run off

**Bug C — repeated commands**

- [ ] Open the display control, press Standby, wait for it, press Standby again
- [ ] Second press should be acted on, not silently ignored

**Bug F — logging**

- [ ] Open `logs\` and confirm files have `===== <time> … run=<id> start =====` markers
      and history from more than one run

---

## Phase 4 — Bug A, the real test

Full steps with expected log lines are in **`docs/design/RETRY_RECOVERY.md`**. The one
that matters:

- [ ] Change track to start an upload
- [ ] About 3 seconds after `UPLOAD START` appears in the log:
      `Stop-Process -Name tiga_stream_watch_flow -Force`
- [ ] Expect within ~2 s: **HALTED**, no restart, renderer still running
- [ ] `.\STATUS_STACK.ps1` shows the halt, with frame and block number
- [ ] `logs\uploader_halted.json` exists with those fields
- [ ] Note what the LCD shows — **don't touch the keyboard yet**
- [ ] Reset the keyboard. **The TIGA has no power switch, and unplugging USB doesn't
      reset it** — it has a battery. Open the case, disconnect the battery from the PCB,
      and wait at least 30 seconds, pressing a few keys while it's disconnected.
      Reconnect. A 10–15 second disconnect was not enough in testing.
- [ ] `.\START_STACK.ps1` → clears the halt → display recovers

**This is the whole point of bug A.** Before tonight it would have retried every 8
seconds against a keyboard mid-transfer.

- [ ] **Unreachable keyboard. A separate test, not halt recovery.** The TIGA has no power
      switch, so "off" means disconnecting the battery from the PCB. Do it only while no
      upload is running: `.\STATUS_STACK.ps1` says OK, and the last line of
      `logs\ble.log` is `UPLOAD COMPLETE`. A disconnect mid-upload halts. Disconnect,
      change track, and keep it disconnected for **1–2 minutes** → expect code-10
      retries at 8/16/32/60 s, then every 5 minutes, and never a halt (since `dc30996`).
      Reconnect → expect an upload within 5 minutes, with no `START_STACK.ps1`. The
      at-least-30-s rule above is for halt recovery only (§7.1).

---

## Phase 5 — Retry policy (decided)

Decided on 2026-10-15 and on `main` since #16 (`dc30996`): an unreachable keyboard
(codes 10 and 11) is retried after 8, 16, 32 and 60 s, then every 5 minutes, and never
halts. Halting is reserved for an interrupted transfer (20, 21, or a session marker) and
for configuration errors (30). Nothing to do here; the Phase 4 "unreachable keyboard" step checks it.

---

## Phase 6 — Optional: the 4-second upload

Only if Phases 2–4 were clean. See `docs/DLX_FORMAT.md`.

- [ ] Generate a single-frame stream (renderer needs `--dlx-frames 1`)
- [ ] Dry run the uploader on it first — expect ~621 writes, 26 progress blocks
- [ ] Live run with `--allow-single-frame`
- [ ] Watch the screen: correct image, no corruption, no tearing

This is the first real test of whether the 452-byte header model is right. If the image
comes out wrong or the LCD misbehaves, that answers the `index` vs `fixed` question from
the notes — write down exactly what you saw.

---

## Phase 7 — Cut the release

Only once the phases above are clean.

- [ ] `CHANGELOG.md` has a `v0.3.0-beta.1` section (done in the release PR); change
      "(not yet tagged)" to the release date
- [ ] Tag and push:

```
git tag v0.3.0-beta.1
git push origin v0.3.0-beta.1
```

- [ ] Wait for the installer workflow to build
- [ ] Paste Part 2 of `docs/RELEASE_NOTES_v0.3.0-beta.1.md` as the release text, after
      deleting its last *(Before publishing)* note, under "Installing", once the Phase 8
      retest has passed (the other one got the 2026-10-25 result)
- [ ] Mark the release as **latest** (not pre-release) if you want people to get it by default
- [ ] Issues: nothing to close; #4–#10, #14 and #15 were all closed by 2026-10-18

---

## Phase 8 — The real end-to-end test

This is what a user actually experiences. Worth doing even though you just tested from
source — the installer path is different code. Do it before Phase 7 with the release PR's
CI artifact (`TIGA-Windows-Media-Display-Setup-vpr-<N>`), and once more afterwards with
the file from the release page.

**What the installer does** (`installer/TIGA-Windows-Media-Display.iss`), which shapes
the test:

- Installs per user to `%LOCALAPPDATA%\Programs\TIGA Windows Media Display` (no admin).
  The AppId is unchanged since v0.2.0-beta.1, so it upgrades an installed v0.2.0-beta.1
  in place.
- **Stops an installed version before copying anything** (since `e0add1b`, 2026-10-25):
  `PrepareToInstall` runs the installed `STOP_STACK.ps1` (v0.2's force-kills; later ones
  stop cleanly) and waits until no uploader, renderer or media helper runs from the install
  folder. After the copy it still runs its own `STOP_STACK.ps1` (`CloseApplications=no`).
  Before the fix, installing over a running v0.2 failed on a locked
  `tiga_stream_watch_flow.exe` (Phase 8 result below).
- Then runs `install_tiga_startup.ps1` and **starts the task. The task name is the same as
  the development install's** ("TIGA Windows Media Display"), so installing takes the
  task over from `tiga-fresh-test`.
- **The packaged STOP_STACK stops any process with these names**, the dev stack's too.
  Its stop request goes to the installed folder, which a dev uploader doesn't watch, so a
  running dev uploader is force-killed after 30 s, possibly mid-upload. **Stop the dev
  stack with its own `STOP_STACK.ps1` first.**
- Logs are in `<install folder>\logs`. Start menu: *Display Control*, *Start Display*,
  *Stop Display*, *Status*.
- **Uninstall stops the stack and unregisters the task.** Afterwards nothing starts at
  sign-in until the dev install re-registers it. The uninstaller leaves runtime files in
  `app\` (the stream's sidecars, `session_01_bulk_stream.windows_nowplaying.bin`, the
  thumbnail and preview) and all of `logs\`. A leftover `.frame` sidecar means a
  reinstall's first upload is the template again (via the mismatched-sidecar path), so
  delete the folder before installing again.

Checks (results of the 2026-10-25 run with build `vpr-32` at `4d06fdf`):

- [ ] Install v0.2.0-beta.1, then this release **over** it while v0.2 is running; note any
      "file in use" message.
      **2026-10-25: failed** with "DeleteFile failed; code 5. Access is denied" on
      `app\_internal\tiga_stream_watch_flow.exe`; worked after v0.2's *Stop Display*.
      Fixed in `e0add1b`. **Retest with a build that has the fix**; steps in
      `docs/NOTES_FOR_MICHI.md`, "Phase 8 on hardware, 2026-10-25".
- [x] An upload completes (`ble.log`: `UPLOAD COMPLETE`); *Status* shows `OK` and a "Last
      delivered" frame. **2026-10-25: OK** (8.6–10.7 s). The first upload was the
      red/green template (`frame=unknown(no-sidecar)`, 64 s) while the renderer took 36 s
      to its first frame; fixed in `319b426`, to be seen in the retest.
- [x] A Japanese title displays as glyphs, not boxes (`renderer.log`: `FONT fallbacks: …`).
      **2026-10-25: OK** (`トーキョーレギー`; no `FONT no font has` line).
- [ ] It works without you touching PowerShell (Start menu shortcuts only).
      **2026-10-25: the *Start Display*, *Stop Display* and *Status* windows closed at
      once**, so their output couldn't be read. Fixed in `7d086bc` (`-NoExit`); check in
      the retest.
- [x] The packaged clean stop: **2026-10-25: OK** (*Stop Display* at 09:41:21, uploader
      exited on request after 0.3 s, `Disconnected cleanly`).
- [x] The packaged automatic retry (not a planned check): **2026-10-25, first time on the
      packaged build:** sudden failure at 09:50:44, retry 1 of 3 at 09:52:44 succeeded.
- [x] Uninstall, then re-register the dev install's task (`install_tiga_startup.ps1`
      in the dev folder) and start it. **2026-10-25: done**: the task points at
      `tiga-fresh-test` again, and its stack (started 09:57) completes uploads. Runtime
      files were left behind (see above).

Also seen: v0.2.0-beta.1 crash-loops on Japanese titles (bug G, fixed in this release):
its renderer exited 16 times in four minutes, and v0.2 completed 14 uploads meanwhile.

Step-by-step for this release: `docs/NOTES_FOR_MICHI.md`, "Phase 8 test steps for
v0.3.0-beta.1"; for the retest, "Phase 8 on hardware, 2026-10-25".

---

## Rollback

If anything goes badly wrong, point the scheduled task back at your known-good copy:

```
cd C:\Users\Regal\source\tiga_ble_re_tools
powershell -File .\install_tiga_startup.ps1
.\START_STACK.ps1
```

If that folder is itself damaged, restore the zip from Phase 0.

If the keyboard is stuck: the TIGA has no power switch, and unplugging USB doesn't
reset it — it has a battery. Open the case, disconnect the battery from the PCB, and
wait at least 30 seconds (30–60 s in older notes), pressing a few keys while it's
disconnected. Reconnect, then run `START_STACK.ps1`. A 10–15 second disconnect was not
enough in testing. See `docs/PROJECT_HISTORY.md` §7.1.

---

## Afterwards

- [ ] Delete `tiga-fresh-test` once you're happy, or keep it as the new working copy
- [ ] If you keep it: decide what happens to `tiga_ble_re_tools`. Having two copies is
      what caused the drift confusion in the first place.
- [ ] Add anything you learned to `docs/PROJECT_HISTORY.md` — especially Phase 6 results
