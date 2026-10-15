# Release checklist — v0.2.0-beta.2

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

Expect roughly: 39 Python cases, 16 Rust tests, and the two PowerShell suites all green.

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
- [ ] Power-cycle the keyboard (full battery disconnect if it's stuck)
- [ ] `.\START_STACK.ps1` → clears the halt → display recovers

**This is the whole point of bug A.** Before tonight it would have retried every 8
seconds against a keyboard mid-transfer.

- [ ] Keyboard off, change track → expect retries at 8/16/32/60 s, then a halt with no
      power cycle needed. Turn it on, `START_STACK.ps1`, recovers.

---

## Phase 5 — Decide on the retry policy

After Phase 4 you'll have felt it. The open question from the notes:

**Should an unreachable keyboard halt after ~2.5 minutes, or keep retrying slowly?**

Halting means: keyboard off for the evening → display halted next morning until you run
Start Display manually.

- [ ] Decide. If you want it changed, it's a one-line change in `Get-UploaderDecision`
      — hand that to Claude Code.

My suggestion: keep retrying slowly for "can't reach the keyboard", and reserve halting
for an actually interrupted transfer. A keyboard that's merely off isn't in a bad state.

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

- [ ] Update `CHANGELOG.md` — bug A, B, C, E, F, G, and the Unicode crash
- [ ] Tag and push:

```
git tag v0.2.0-beta.2
git push origin v0.2.0-beta.2
```

- [ ] Wait for the installer workflow to build
- [ ] Mark the release as **latest** (not pre-release) if you want people to get it by default
- [ ] Close issues #4, #5, #6, #8, #9, #10 — and tick the #10 checklist
- [ ] Leave #7 open (shutdown ack) unless that got merged too

---

## Phase 8 — The real end-to-end test

- [ ] Download the installer from your own release page
- [ ] Run it
- [ ] Confirm it works without you touching PowerShell

This is what a user actually experiences. Worth doing even though you just tested from
source — the installer path is different code.

---

## Rollback

If anything goes badly wrong, point the scheduled task back at your known-good copy:

```
cd C:\Users\Regal\source\tiga_ble_re_tools
powershell -File .\install_tiga_startup.ps1
.\START_STACK.ps1
```

If that folder is itself damaged, restore the zip from Phase 0.

If the keyboard is stuck: full battery power-cycle — open the case, disconnect the
battery, 30–60 seconds, press a few keys while it's dead, reconnect. See
`docs/PROJECT_HISTORY.md` §2.4.

---

## Afterwards

- [ ] Delete `tiga-fresh-test` once you're happy, or keep it as the new working copy
- [ ] If you keep it: decide what happens to `tiga_ble_re_tools`. Having two copies is
      what caused the drift confusion in the first place.
- [ ] Add anything you learned to `docs/PROJECT_HISTORY.md` — especially Phase 6 results
