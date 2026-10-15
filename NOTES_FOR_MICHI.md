# Notes for Michi: TASKS.md run, 2026-10-15

Nothing was sent to the keyboard: no BLE, no USB, no uploads. `tiga_ble_re_tools`
and `tiga-runtime-lab` were not touched. Nothing was merged to `main`, and no pull
request was opened.

## Read this first

**Your local `main` was 4 commits behind GitHub.** It was at `e215796`; `origin/main`
is at `a61af7c` (the audited HEAD in PROJECT_HISTORY §7). The missing commits added
the installer pipeline, so the "packaged supervisor" TASKS.md refers to
(`installer/runtime/`) did not exist locally. I fast-forwarded local `main` to
`origin/main` (no local commits were lost; it was a pure fast-forward) and based
`fixes/beta-2` on it.

## Issues filed (Task 1)

| Letter | # | Title |
|---|---|---|
| A | #4 | BLE upload failure triggers a retry loop the uploader explicitly forbids |
| B | #5 | Display stays on standby if media was already playing when the renderer starts |
| C | #6 | Reissuing the same display command is silently ignored |
| D | #7 | "Standby + Shut Down" shuts down on a fixed timer without confirming the upload finished |
| E | #8 | Development startup fails when the checkout path contains spaces |
| F | #9 | Logs are overwritten on every supervisor restart, destroying failure evidence |
| G | #10 | Rendering and maintenance defects (collected) |

Numbering starts at #4 because #1–#3 are taken by earlier PRs. I created the labels
`priority:high`, `ble`, `renderer`, `display-control`, `tooling`, `diagnostics` and
`polish`. The G checklist is **not** ticked on GitHub, because the fixes are only on
an unmerged branch.

## Tasks and commits (branch `fixes/beta-2`)

| Task | Status | Commit(s) |
|---|---|---|
| 1. File issues | done | (GitHub only) |
| 2. Release flag | **no change needed**: see below | none |
| 3. F: logging (#9) | done | `c9e9048` |
| 4. B: standby gate (#5) | done | `13c218c` |
| 5. C: command IDs (#6) | done | `45af4b7` |
| 5. E: paths with spaces (#8) | done (fix in `c9e9048`, test here) | `d184416` |
| 6. G: polish (#10) | done, all 7 items | `713e30c` titles · `48e22d8` album overlap · `ad3dc42` artwork hash · `7057f46` query failures · `320d792` stream file · `a795e9e` diagnostic paths · `64bec34` Cargo.lock |
| 7. D: shutdown ack (#7) | **prepared, not merged** | `c1f0c59` on branch `review/shutdown-ack` |
| 8. Duplicate frame | investigated; flag-gated code + offline tests | `f016590` |

Tests: 39 Python unittest cases (`py -m unittest discover -s tests`), 10 Rust unit
tests (`cargo test --release --locked --bin tiga_stream_watch_flow`), and a PowerShell
supervisor test (`tests\test_supervisor_logging.ps1`). All pass locally, and all three
now run in CI.

### Task 2: the release was already right

When I checked, `v0.2.0-beta.1` already had `isPrerelease: false`, and
`/releases/latest` already returned `v0.2.0-beta.1`. TASKS.md's description
(v0.1.0 holding "Latest") no longer matched GitHub, so I ran nothing. Either you
fixed it already, or the description was written from an older state.

### Task 3: what changed (F, #9)

- Both supervisors launch children through `cmd.exe /c "..." >> log 2>> err` instead of
  `Start-Process -RedirectStandardOutput`, which truncated the file on every start.
- Every child start writes `===== <time> <name> run=<run-id> start =====` into both of
  its logs and into the supervisor log, and passes the run ID as `TIGA_RUN_ID`.
- Python runs with `-u` and `PYTHONUNBUFFERED=1`; the renderer also reconfigures stdout
  to UTF-8 and line buffering (that matters for the packaged PyInstaller exe, see the
  findings below).
- Logs rotate at 5 MB, keeping `.1`–`.3`.
- Frame IDs: the renderer logs `FRAME <id> rendered/installed crc32=…` and writes
  `<stream>.frame`. The uploader logs `UPLOAD START/COMPLETE/FAILED frame=<id> crc32=…`
  and writes `<stream>.delivered` on completion. The CRC alone is enough to join the
  two logs if the sidecar is ever missing.
- `STATUS_STACK.ps1` (dev and packaged) shows last rendered vs last delivered frame,
  `OK / PENDING / STALLED`, and the last `UPLOAD FAILED` line.
- The uploader also logs the adapter it picked and the keyboard address (part of the
  §7 repair-order diagnostics).

### Task 4: B (#5)

Standby is now released when the same media item, with a title, reports Playing
continuously for **15 s**, even if the position never moves. Paused/Stopped remembered
sessions, and Playing sessions without a title, still stay on standby. The trade-off:
a stale session that falsely reports *Playing* is released after 15 s. The only way to
tell it apart would be GSMTC's timeline `LastUpdatedTime`, which the snapshot helper
doesn't expose. That's a possible follow-up in C++. A `main()`-level test reproduces
the original bug against the old code and passes on the new.

### Task 6: the stream file decision

**Untracked and ignored, seeded from the committed template.** Reasoning:

- The tracked `extracted/session_01_bulk_stream.bin` was **byte-identical** to
  `extracted/session_01_bulk_stream.redgreen-backup.bin` (SHA-256 `a4be0a5d…`), and so
  is `session_02_bulk_stream.bin`.
- The renderer never reads the active file; it always builds from the redgreen
  template. So the tracked copy carried no information and only made the tree dirty.
- Both supervisors now copy the template into place if the active file is missing.
  The installer workflow stages the template under the active name. README and
  BUILD_AND_RECOVERY explain where the file comes from.
- One cost: a fresh clone has no active stream until the renderer runs (or you copy
  the template) when doing a manual uploader test. The README says so.

### Task 7: shutdown ack (D, #7): ready for review

Branch `review/shutdown-ack` (one commit on top of `fixes/beta-2`, pushed, **no PR**).
The design is in `docs/design/SHUTDOWN_ACK.md` on that branch. In short:

1. The renderer acks the exact command text with the frame ID and CRC it installed.
2. The GUI waits until `.delivered` names that frame ID.
3. If either stage times out (15 s / 90 s), it asks before shutting down, defaulting to No.

It matches on the frame ID, not just the CRC, because two standby frames in the same
minute are byte-identical. A CRC-only match could accept an old delivery while another
frame is mid-upload. The uploader therefore records "already displayed" when a new
frame is identical to its last completed upload. The design doc has four open
questions for you and a five-step hardware test plan. The GUI compiles cleanly at
`/W4` but was never run.

### Task 8: duplicate frame and the 452-byte header

Full write-up: `docs/DLX_FORMAT.md`. Headlines:

- The header is mostly **file-relative offsets**. The first u16 (`60 01` = 352) points
  to a section table at file offset 352:
  `[368 records, 452 frame index, 460 pixel data, 220620 pixel end]`. Next come
  a record count (3) and a probable frame count (2).
- Three 28-byte records, each 320×172 (BE). Record 2 carries count 2 and `0x0a`,
  plausibly a 100 ms frame delay. The counts 1+2+1 don't add up to 2 frames, so
  records aren't per-frame.
- The 8 bytes at 452 hold `[0, 110080]`. That reads equally well as a per-frame
  offset table (`index`) or as an `[offset, size]` descriptor (`fixed`). One capture
  cannot distinguish them; the first hardware test should.
- `tiga_dlx.py` rebuilds the captured 220,649-byte stream **byte-for-byte** from
  this model.
- A one-frame container is 110,565 bytes (`index`) or 110,569 (`fixed`). That's
  621 BLE writes and 26 PROGRESS blocks, against 1,239 and 53. On transfer volume
  alone that's roughly 8 s → 4 s, before handshake overhead.
- Gated twice: the renderer needs `--dlx-frames 1`, **and** the uploader needs
  `--allow-single-frame`. Each refuses the other's stream otherwise. Defaults are
  untouched: the default renderer output is tested equal to the model, and CONTROL1
  plus the chunking for the captured stream are tested unchanged.
- The suggested hardware order is in `docs/DLX_FORMAT.md`. Run the uploader's dry
  run (no `--confirm-flow-watch`) on a generated stream first.

## Blocked or not verified

- **Executing the uploader binary was blocked** by this session's safety policy, even
  in dry-run mode (which opens no Bluetooth). So the uploader is verified only by
  `cargo check`, which is warning-free, and its unit tests. Please run the dry run
  once yourself:
  `.\target\release\tiga_stream_watch_flow.exe --stream ..\extracted\session_01_bulk_stream.bin`
  It should print `Validated: 220649 bytes, 1239 writes, 53 x 4096-byte progress blocks`.
- The supervisors' main loops were not run, because they would start the uploader
  live. Their helper functions (launch, append, run ID, rotation, tree-kill, quoting,
  seeding) were extracted and tested on a dummy Python child.
- `tiga_display_control.exe` was only compile-checked, never launched.
- The installer (PyInstaller + Inno Setup) was not built locally. `installer.yml` runs
  on PRs and tags, not on branch pushes, so it hasn't run against these changes either.
  Worth a `workflow_dispatch` run before any release.
- The "packaged supervisor" part of F: done, but only verified in the same offline
  way as the dev one.

## Findings that contradict or extend PROJECT_HISTORY.md

1. **§10 "a captured header that is otherwise not understood"**: not quite.
   `reference/TRANSCRIPT.md` (local, gitignored) already recorded six header fields
   (452, 460, 220620, 320, 172, 110080). It listed them by header-relative position,
   which hid that they are file offsets. The section-table structure above is new.
2. **New crash cause not in §7: non-Latin titles.** With stdout redirected to a file,
   Python on Windows encodes `print()` output in the ANSI code page (cp1252 here). A
   title the code page can't encode, such as Japanese, makes `print(f"Title: …")`
   raise `UnicodeEncodeError`, so the renderer exits and the supervisor restarts it
   into the same crash. I hit exactly this error printing a Japanese test title from
   this shell. It's fixed in `c9e9048` (UTF-8 stdout with `errors="replace"`). It's a
   strong candidate for "it stopped working on some songs" reports.
3. **The supervisors never logged exit codes.** `Start-Process -PassThru` objects
   report `ExitCode` as empty unless the handle was touched while the process ran, so
   every "exited with code …" line had a blank code. Fixed in `c9e9048`. This matters
   for A (#4), which needs to distinguish failure classes.
4. **§7 G "720-pixel line"**: reproduced as 780 px for a 60-character unbroken title
   (depends on the string). Same bug, fixed.
5. **TASKS.md Task 2's premise** didn't match GitHub (see above).
6. Minor and **not fixed**: the no-artwork fallback tile's "♪" renders as a missing-glyph
   box in Segoe UI Bold. Visible in any render without a thumbnail.
7. `MANIFEST.json` (the 2026-09-09 snapshot) is now stale: hashes of changed files,
   and the untracked stream file. I left it alone; regenerate or drop it.

## Still open

- **A (#4) is untouched.** It wasn't in TASKS.md, and it is the highest-severity bug.
  F makes it diagnosable now: a failed transfer logs `UPLOAD FAILED … (last ACKed block
  N)` with a frame ID, and that evidence now survives the restart that follows.
- `TASKS.md`, `docs/ISSUES_TO_FILE.md` and `docs/PROJECT_HISTORY.md` are still
  untracked. I didn't know whether you want them public.

---

## Verified after the run (2026-10-15 19:46, by Michi)

The dry run listed as blocked above was executed manually and **passed**:

```
TIGA flow-controlled stream uploader
Stream          : ..\extracted\session_01_bulk_stream.bin
Intra-block pace: 2 ms/write
Flow control    : WAIT for one PROGRESS after every 4096-byte DLX block
Mode            : watch + coalesce
Validated: 220649 bytes, 1239 writes, 53 x 4096-byte progress blocks

DRY RUN ONLY - no Bluetooth connection was opened.
Live mode requires --confirm-flow-watch
```

Those three figures match the original PocketWuque capture documented in
`docs/PROJECT_HISTORY.md` §3.2-3.3, so the rebuilt uploader still agrees with the
known-good reference.

The "Executing the uploader binary was blocked" item under **Blocked or not verified**
is closed. Everything else in that section still stands: the supervisor main loops,
`tiga_display_control.exe`, and the installer build remain unverified.

---

# Bug A run (TASKS_A.md), 2026-10-15 evening

Branch `fix/a-retry-recovery`, based on `fixes/beta-2` and pushed. No PR, no merge.
Nothing was sent to the keyboard.

## Correction to the "Verified after the run" note above

**That dry run tested the old uploader, not the rebuilt one.** The binary at
`windows_ble_probe\target\release\tiga_stream_watch_flow.exe` was built at **19:09**,
before any of the previous run's uploader changes (the first, `c9e9048`, was committed
at 19:15). My `cargo build` of the new code was blocked in that run, so the old binary
stayed in place. Two ways to tell:

- the pasted first line is `TIGA flow-controlled stream uploader`; every build since
  `c9e9048` prints `<timestamp> TIGA flow-controlled stream uploader starting (run_id=…)`
- the old binary does not contain the strings `UPLOAD START`, `run_id` or `allow-single-frame`

The three figures (220649 / 1239 / 53) are still right for the new code: the unit
tests check exactly that. But the item "uploader binary not run" is **not** closed.

This run, building was allowed: `target\release\tiga_stream_watch_flow.exe` is now
**this branch** (built 20:07). **Running** it, even the dry run TASKS_A explicitly
permits, was blocked again by this session's safety policy, so please repeat the dry
run against the new binary. The expected output is in `docs/design/RETRY_RECOVERY.md`,
step 0.3.

## What changed

| Commit | Change |
|---|---|
| `a21fcd1` | Uploader: failure classes and exit codes 0/10/11/20/21/30, CONTROL2 boundary, session marker, `EXIT code=N (meaning)` line, `<stream>.exit` record |
| `e45e0b9` | No behaviour change: shared supervisor helpers moved into `tiga_supervisor_common.ps1` (dev and packaged both dot-source it; the installer stages it) |
| `5336c78` | Supervisor policy: backoff 8/16/32/60 s then give up; halt on 20/21/30 or a leftover marker; `logs\uploader_halted.json`; renderer untouched; live un-halt |
| `71ca4f4` | `START_STACK.ps1` (dev and packaged) clears the halt; README and BUILD_AND_RECOVERY updated |
| `8559eb3` | `STATUS_STACK.ps1` (dev and packaged): `HALTED` verdict with reason, frame, block and instruction |
| `367e21f` | `docs/design/RETRY_RECOVERY.md`: policy and hardware test plan |

Tests: 15 Rust unit tests (6 new), and the new `tests\test_supervisor_retry.ps1`
(40 checks, all passing, and in CI). The latter drives the **real** supervisor loop
with dummy children for every exit class: backoff growth and give-up, no restart and
renderer untouched on a halt, the halt file's fields, a killed uploader with a marker,
a stale exit record being ignored, a counter reset by a delivery, halted at startup,
live un-halt, `Clear-UploaderHalt`, STATUS output, and START_STACK ordering. The
39 Python tests and the logging test still pass.

## Was the CONTROL2 boundary detectable as assumed?

**Not as the code stood. Yes after a small restructure.** Every error inside
`upload_stream` came back through `?` as a plain message, with nothing recording how
far the upload had got. The upload is strictly linear and CONTROL2 is written in exactly
one place, so I added an `UploadProgress` tracker that records whether CONTROL2 was
attempted and the last ACKed block. Every error after that point is classified from it.

Two judgment calls, both on the safe side:

- **Boundary = CONTROL2 *attempted*,** not acknowledged. A WriteWithoutResponse that
  returns an error may still have reached the keyboard.
- **CONTROL1 counts as before the boundary** (retryable), as TASKS_A specified.
  CONTROL1 announces the file length. Whether the keyboard treats that alone as an open
  session is unknown.

Also, contrary to TASKS_A's summary: before this branch, `(last ACKed block N)` appeared
**only on PROGRESS timeouts**. A write error or a dropped notification stream
mid-transfer logged no block. Every post-CONTROL2 failure reports it now.

### One addition beyond TASKS_A: the session marker

Exit codes can't classify the case that matters most. A **killed** uploader, which is
exactly how the hardware plan interrupts a transfer, or a crashed one, or one that dies
with Windows shutting down, never reports its class. The uploader therefore keeps
`extracted\session_01_bulk_stream.bin.session` from just before CONTROL2 until
COMPLETE. If it survives an exit, the supervisor halts whatever the code. If it exists
at uploader start, the uploader refuses to run (exit 20). This also catches "PC shut
down mid-upload, keyboard never power-cycled" at the next logon.

## Needs a decision

1. **Giving up on an unreachable keyboard.** As TASKS_A suggested, code 10 halts after
   about 2.5 minutes of retries. Consequence: turn the keyboard off for the evening, and
   the uploader is halted when you come back, until you run Start Display. Before this
   branch it retried every 8 s forever, which is harmless for this class because no
   session is opened. An alternative is to keep retrying code 10 slowly (say every 5
   min) instead of halting, and reserve halting for 20/21/30. That's a one-line change in
   `Get-UploaderDecision`.
2. **STOP_STACK or Windows shutdown during an upload now counts as an interruption.** The
   marker survives, so the next logon halts. This is correct by the rule, but it makes
   #7 (shutdown timing) more visible: today "Standby + Shut Down" can easily shut down
   mid-upload. `review/shutdown-ack` would fix that.
3. **START_STACK trusts you.** It assumes the keyboard was power-cycled; nothing can
   verify that. It prints a reminder.
4. **Merge order.** `review/shutdown-ack` (#7) and this branch both edit the uploader's
   upload loop. Whichever merges second needs a rebase; it should be mechanical.
5. **GUI.** It doesn't show the halt (optional in TASKS_A). I skipped it to avoid a
   second branch touching `tiga_display_control.cpp`.

## Exact hardware steps to verify

The full plan, with expected log lines and a "what if it goes wrong" table, is in
`docs/design/RETRY_RECOVERY.md`. In short:

0. **The scheduled task on this PC runs `tiga_ble_re_tools`**, not this repo, so none
   of `fixes/beta-2` or this branch runs here yet. Check out `fix/a-retry-recovery`, run
   `BUILD_AND_INSTALL.ps1` from Developer PowerShell (this re-points the task at the
   repo), do the dry run, then run the three offline test scripts.
1. Baseline upload; status OK; marker gone afterwards.
2. **Kill the uploader about 3 s after `UPLOAD START`**
   (`Stop-Process -Name tiga_stream_watch_flow -Force`). Expect: HALTED (session-open)
   within about 2 s, **no restart**, renderer still running, halt file with frame and block,
   red STATUS. Note what the LCD shows without touching it. Power-cycle (full battery if
   needed), run `START_STACK.ps1`, and expect recovery.
3. `STOP_STACK.ps1` mid-upload, then `Start-ScheduledTask` (not START_STACK). Expect the
   uploader to refuse with exit 20 before connecting, then a halt.
4. Keyboard off, change track. Expect code 10 retries at 8/16/32/60 s, then a gave-up
   halt with no power cycle needed. Turn it on, run START_STACK, and it recovers.
5. Restore the task to the live copy:
   `powershell -File C:\Users\Regal\source\tiga_ble_re_tools\install_tiga_startup.ps1`.
