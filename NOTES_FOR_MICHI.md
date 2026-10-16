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

---

# Device selection run (TASKS_B.md), 2026-10-15 late evening

Branch `fix/device-selection`, pushed. No PR, no merge. Nothing was sent to the keyboard,
nothing started or stopped the stack, and `tiga_ble_re_tools` and `tiga-runtime-lab` were
not touched.

## Read this first

1. **The keyboard needs a power cycle before you restart the stack.** The logs show a
   **second** interrupted transfer that isn't in TASKS_B: at 21:39:48, block 3/53, exit
   20, halted at 21:39:54. `tiga-fresh-test\logs\uploader_halted.json` is still there
   from it. Power-cycle, then `START_STACK.ps1`, as the halt says.
2. **`tiga-fresh-test` was repaired.** An earlier run of this task had worked directly
   in it, on `fix/device-selection`. There was nothing uncommitted to save: the
   previous run had already committed its work as `5223945`, and only the five
   untracked docs were left. They are still there, untouched, because checkout keeps
   untracked files. I ran `git checkout main` there, then
   `git worktree add ..\tiga-dev fix/device-selection`. It is now on `main` at
   `128745d`, same as `origin/main`, with a clean tree apart from those five files.
   Between 21:55 and about 22:00 its source tree held the branch's uploader source;
   the checkout restored `main`'s.
3. **Nothing was built in `tiga-fresh-test` after 21:45.** No `.exe` and nothing under
   `windows_ble_probe\target` is newer than 21:45. The binaries there date from 21:02
   and 21:04 (the display control). What changed after 21:45: the previous run's three
   source edits (now in `5223945`), and renderer output at **21:49**
   (`windows_nowplaying_preview.png`, `windows_media_thumbnail.bin`, the stream files).
   So the renderer was still running at 21:49, kept alive by the halt. By about 22:00
   neither the renderer nor the uploader was running.
4. **Building and running the uploader was blocked again** by this session's safety
   policy, even `cargo build --release` and the dry run. `cargo test` was allowed. So
   `tiga-dev\windows_ble_probe\target\release\tiga_stream_watch_flow.exe` was **not**
   built from this branch. See "What to do on hardware" step 1.

## Issues

| # | What | Filed |
|---|---|---|
| #14 | Uploader picks the wrong BLE device when two advertise a matching name | by the earlier run, 21:51. Identical to `docs/ISSUE_11_DEVICE_SELECTION.md`; not filed twice |
| #15 | Display control reports a command as sent when the write never reached the renderer | this run |

Commented, not closed, on #4, #5, #6, #9 and #10, with the verified / not-verified lines.

## Commits on `fix/device-selection`

| Commit | Task | Change |
|---|---|---|
| `5223945` | 3 | (earlier run) Select by media service, not name; log every candidate; cache last good address in `<stream>.device`; exit 11 only once all were tried |
| `7002196` | 1 | Commit `PROJECT_HISTORY.md` and `RELEASE_CHECKLIST.md`; ignore `TASKS*.md` and the issue drafts |
| `ca51d3a` | 3 | Rescan once if every candidate was rejected; a rejected device's log line lists every service and characteristic it exposes |
| `dc30996` | 4 | Codes 10 and 11: 8/16/32/60 s, then every 5 min, never halt |
| `1be13bc` | 5 | PROJECT_HISTORY §7 (device selection confirmed) and new §7.1 (the session); README Firefox line removed |
| `dbc902f` | 6 | Display control: read back, wait for the renderer's ack, no shutdown prompt without it |
| `f124932` | 6 | `SHUTDOWN_ACK.md` to match |
| `7c93850` | 7 | Per-character font fallback (CJK, Hangul, ♪) |
| `777944b` | 7 | Delete `MANIFEST.json` |
| `e6b8311` | 8 | README install troubleshooting; STATUS labels a past failure as historical |
| (this) | | These notes |

Tests: 28 Rust (`cargo test --release --locked --bin tiga_stream_watch_flow`), 47 Python,
and both PowerShell suites, all passing. The display control compiles at `/W4` and was
not linked or run.

## How device selection behaves now

1. If `extracted\session_01_bulk_stream.bin.device` exists, its address is tried **the
   moment the scan sees it**, without waiting for the 5 s window and with or without
   a name.
2. Otherwise every peripheral whose name contains `zoom75` is a **candidate**. Each is
   logged once as `CANDIDATE name=… address=… rssi=…`. When the 5 s window closes,
   candidates are tried in the order they were seen.
3. Trying one means connecting and discovering services. It **qualifies** only with
   service `1f40eaf8…` and all four characteristics `1f400001`–`1f400004`. Anything
   else is disconnected and logged as `REJECTED <address> for this run: connected, but
   no media service …; exposes N service(s): <uuid> [<char uuids>]; …`. Then the next
   candidate is tried.
4. If every candidate seen was rejected, it logs `RESCAN …` and scans one more 5 s
   window for others. Already-rejected devices are not retried.
5. `SELECTED <address> … after trying N candidate(s)`, then the usual `Found keyboard at …`.
6. After the first completed upload, the address is written to `<stream>.device`.
7. Failure: exit 11 `tried N candidate(s) over M scan(s), none qualified: …` if every
   candidate was reached. Exit 10 if nothing matched, or if any candidate couldn't even
   be connected (that one might have been the keyboard). Both now retry every 5 min.

## What to do on hardware

1. **Build and dry-run** (in `tiga-dev\windows_ble_probe`):
   `cargo build --release`, copy `..\extracted\session_01_bulk_stream.redgreen-backup.bin`
   to `..\extracted\session_01_bulk_stream.bin`, then
   `.\target\release\tiga_stream_watch_flow.exe --stream ..\extracted\session_01_bulk_stream.bin`.
   Expect a timestamped first line with `run_id=`, `Validated: 220649 bytes, 1239 writes,
   53 x 4096-byte progress blocks`, `DRY RUN ONLY`.
2. **Getting it onto the scheduled task:** the task runs `tiga-fresh-test`, which is on
   `main`, so none of this runs until it is merged and pulled there. Running
   `BUILD_AND_INSTALL.ps1` from `tiga-dev` would re-point the task at `tiga-dev`. That
   works for a test, but then the task runs whatever `tiga-dev` has checked out.
3. **First live start, with no `.device` file:** in `ble.log` look for **both**
   `CANDIDATE` lines. If the scan lists `1B:A3:A9:10:13:5D` first, expect its
   `REJECTED … exposes …` line before `SELECTED 07:01:88:B3:66:1B`. **Please keep that
   `exposes` line.** It's the first clue to what that device is. If `07:01…` comes
   first, the other device is never probed and you won't get that line this time.
4. **After that upload:** `extracted\session_01_bulk_stream.bin.device` holds
   `address=07:01:88:B3:66:1B`.
5. **Next start:** `Scanning … (trying 07:01:88:B3:66:1B first: it completed the last
   upload)`, then `SELECTED` within a second or two of the scan starting, with no
   `REJECTED` line.
6. **Keyboard off, change track:** code 10 at 8/16/32/60 s, then
   `Restarting BLE uploader in 300s … (no halt)`. Switch it on and do nothing else. It
   should upload within 5 minutes, with no `START_STACK`.
7. Probing the wrong device took about 6 s in tonight's log (21:08:53 found → 21:08:59
   exit). So a cold start without the cache can take about 5 s scan + 6 s + connect
   before `SELECTED`.

## Task 6: the two questions

- **Path mismatch? No.** The GUI builds the path from its own exe location
  (`GetModuleFileNameW`), not the working directory, so an Explorer double-click, a
  shortcut and another CWD all give the same path. The renderer uses its script
  folder, and its log printed `Control file: C:\Users\Regal\source\tiga-fresh-test\tiga_display_command.txt`.
  Only one build contains the messages you saw: `tiga-fresh-test\tiga_display_control.exe`,
  21:04:57. There is no other copy of it anywhere under `source`, Program Files,
  AppData\Local\Programs, Desktop or Downloads.
  - The write **reported success** to the GUI; otherwise it would have said "Could not
    request standby". Yet the renderer logged no command before your hand-written
    `standby test1` at 21:30:31.
  - **The same exe worked later.** At 21:34:27, process 32884 (its 3rd command) wrote
    `auto 134365664677263042-32884-3` into `tiga-fresh-test`. The renderer acted on it
    at 21:34:28 and acked it; both files are still there. The black at 21:33:04 and
    the standby at 21:33:18 were probably commands 1 and 2 from that process, but the
    file was overwritten, so I can't prove it.
  - That fits CyberCapture holding only the first launch. **Still unconfirmed**; the
    code rules out the path, not the antivirus.
- **Can Lock Standby reach "Shut down anyway?"? No.** Lock Standby writes the file and
  sets the status, nothing else. The text "Standby requested. Waiting for the
  renderer…" and the "did not confirm standby" dialog exist **only** in the Standby +
  Shut Down worker. So the message you saw means Standby + Shut Down was pressed.
- **Fix (#15):** read-back after every write, with a plain error naming the path and
  antivirus. Every button then waits up to 10 s for the renderer's ack before saying
  "confirmed". **A read-back alone can't catch a sandbox**, because a sandboxed process
  reads back its own redirected copy; the ack is the real check. Standby + Shut Down no
  longer offers to shut down when the renderer didn't ack, since from there a stopped
  stack and a command that never arrived look the same. The "keyboard has not
  confirmed the upload" prompt is unchanged.

## Other decisions

- **Retry policy:** an unexplained exit with **no** session marker (code 1, -1, …) still
  gives up after five attempts. TASKS_B covered only 10 and 11. Say if you want those on
  the slow retry too.
- **Font fallback:** Segoe UI, then Yu Gothic → Meiryo → Microsoft YaHei → Malgun
  Gothic → Segoe UI Symbol, per character. A character counts as missing when it draws
  identically to the font's missing-glyph box. Latin-only text renders pixel-identical
  to before. Cosmetic: Malgun's Hangul sits a little taller, so a Hangul album line
  comes close to the divider. Not tuned.
- **`MANIFEST.json`: deleted, not regenerated.** Nothing reads it. It hashed the
  September bundle, most of those files have changed, and it contained a hash of
  itself, which can't be right. Git already records the content; a regenerated copy
  would be stale again at the next commit.
- **README limitations:** removed the Firefox line. The others are grounded: the roar
  (§4.1), power-cycle after an interrupted transfer (bug A policy), the historical
  filename, and the task-name replacement. I nearly "corrected" the Standby + Shut Down
  description to "fixed 12 s" before seeing that #12 (shutdown ack) was merged, so it's
  right as written.

## Tests skipped under rule 3

None. Before running them I checked both PowerShell suites. They kill only their own
dummy children, by PID, through `Stop-LoggedProcess`. They read `START_STACK.ps1` as text
rather than running it. The only real-system call is `STATUS_STACK.ps1`'s read-only
`Get-ScheduledTask` / process listing, run from a copy in a temp folder.

## Contradicts or extends PROJECT_HISTORY.md

Recorded in §7.1:

1. **A second interrupted transfer** (21:39, block 3/53), 3.7 minutes after a clean
   upload on the same connection. "The keyboard was in a bad state from stopping the
   previous stack" does not obviously explain this one.
2. **Upload times:** 19 uploads took 8.42–10.01 s. §3.3 says ~7.7–8.5 s. The two
   first-on-a-connection uploads were the fastest (8.71, 8.42); the later ones took
   9.49–10.01. Two samples; recorded, not explained.
3. `CLAUDE.md` called `tiga_ble_re_tools` the live copy. Fixed (below).

## Task 9: the stale folder and CLAUDE.md

**`tiga-windows-media-display` is safe to delete.** It has no commits outside `origin`,
no stash, and every local branch tracks a remote branch. Its untracked files:

- `docs/PROJECT_HISTORY.md`, `docs/ISSUES_TO_FILE.md`: identical to the ones now in `tiga-dev`
- `RELEASE_CHECKLIST.md`: an older subset of the committed one (0 unique lines)
- `extracted/session_01_bulk_stream.bin`: byte-identical to the template
- **`TASKS.md` and `TASKS_A.md`: unique.** The prompts for the first two runs. Copy them
  out first if you want them kept.

I ran `git fetch` there to compare branches, which updated its remote-tracking refs and
nothing else.

**`C:\Users\Regal\source\CLAUDE.md` is updated** (outside the repo, so not committed).
The new folder map: `tiga-fresh-test` is live, `tiga-dev` is the development worktree,
`tiga_ble_re_tools` is the fallback, and `tiga-windows-media-display` is stale and
awaiting deletion. The ground rules now include TASKS_B's: work only in `tiga-dev`, no
START/STOP_STACK, check PowerShell tests before running them, dry runs allowed.

## Follow-up: two missed items

**1. Unplugging USB is not a power cycle** (`583faf9`). Every place the halt instruction
appears now says: use the keyboard's **power switch**; unplugging USB does not
power-cycle the TIGA because it has a battery; if the display stays stuck, **disconnect
the battery**. That covers:

- the supervisor's halt instruction (`tiga_supervisor_common.ps1`), which is what
  `STATUS_STACK.ps1` shows and `uploader_halted.json` stores
- the "halted at startup" supervisor log line
- the uploader's exit meanings for codes 20 and 21, and its session-marker refusal
- `START_STACK.ps1`, dev and packaged
- README, `BUILD_AND_RECOVERY.md`, `RETRY_RECOVERY.md` and `RELEASE_CHECKLIST.md` (Phase 4)

PROJECT_HISTORY §7.1 now records the two attempts. The first was a ~5 s USB unplug,
which did not reset the keyboard (the roar stayed). The second was a battery disconnect,
which did. It also says what is still open: whether the power switch alone is enough
after an interrupted transfer isn't shown by tonight's record. The same section said
"after a power cycle"; that was wrong, because the first attempt was not a power cycle.

Tests after the change: 28 Rust, both PowerShell suites, all passing. The uploader binary
still isn't rebuilt (blocked, as before), so the new exit-meaning text appears only after
your `cargo build --release`.

**2. #7 commented, not closed.** TASKS_B Task 5 has no #7 line, so the comment uses
Task 6's observation. Standby + Shut Down was pressed on the ack-wait build. It did
**not** shut down on a timer: it waited for the renderer's ack, got none, and asked. The
command never reached the renderer (#15), so the delivery-ack stage and the actual
shutdown were never exercised. Verdict: partly exercised, happy path unverified. Also
added to §7.1's "not verified" list (`(this)` commit). Correct me if you had a
different #7 result in mind.


---

# Hardware results run (TASKS_C.md), 2026-10-15, about 23:20–23:35

## Step 0: what was live

**`tiga-fresh-test` was live, so the run went ahead.** All three checks agreed:

- the task action runs `tiga-fresh-test\tiga_startup_supervisor.ps1`
- the running `tiga_stream_watch_flow.exe` and the renderer's Python are both from `tiga-fresh-test`
- the newest supervisor launch is `tiga-fresh-test`'s, at 23:15:15; `tiga-dev`'s last one is 23:09:12

`tiga-fresh-test` is on `main`, level with `origin/main`, with no tracked changes. It does
have five untracked files from earlier runs (`RELEASE_CHECKLIST.md`, `TASKS_B.md`,
`docs/ISSUES_TO_FILE.md`, `docs/ISSUE_11_DEVICE_SELECTION.md`, `docs/PROJECT_HISTORY.md`). I left
them alone.

## Commits on `fix/device-selection` (pushed)

| Commit | What |
|---|---|
| `cc07788` | Task 1: try `Zoom75 Tiga` first, `… BT<n>` last |
| `4e5b2ac` | Task 2: a scan that has seen nothing keeps going up to 15 s |
| `224e771` | Task 3: block timeout 10000 ms as the uploader's default; hand edit dropped |
| `60e94c1` | Task 4: no power switch; disconnect the battery for at least 30 s |
| `e1bd5d2` | Task 5: PROJECT_HISTORY §7.1, second part |
| `2baca15` | Task 6: every upload of the day in one table, with answers |
| (this) | these notes |

Tests after the last code change: 35 Rust (was 28 before TASKS_B's follow-up; 7 new tonight),
47 Python, and both PowerShell suites, all passing. **None skipped under rule 3:** I
re-checked both suites. They run `STATUS_STACK.ps1` from a temp copy (read-only
`Get-ScheduledTask`), read `START_STACK.ps1` as text, and kill only their own children by
PID.

**The uploader is rebuilt this time** (`tiga-dev\windows_ble_probe\target\release`,
23:23+). The dry run prints `Block timeout   : 10000 ms`. This matters because of Task 3,
below.

## Task 1: prefer the exact display name

Ranking is now: cached address → exactly `Zoom75 Tiga` → any other name containing
"zoom75" → names ending in ` BT<digits>` (any case). Within each group, discovery order.
Still nothing is tried before the 5 s window closes, except the cached device, so every
candidate still gets its `CANDIDATE` line.

New tests: BT1 seen first, `Zoom75 Tiga` is still tried first (and BT1 second if the
display doesn't qualify); a third, other zoom75 name lands between them; only BT1
present still gets probed; a cached address wins even over the exact name; the BT-suffix
matcher itself.

## Task 2: invisible after a restart → implemented

Simple and testable offline, so I implemented it rather than proposing it. If the scan
has seen **no candidate at all** when the 5 s window closes, it keeps scanning up to 15 s
and tries the first candidate that appears. It logs one line when it does:
`Nothing seen in 5 s; scanning up to 15 s …`. Candidates that were seen and rejected
still get exactly one 5 s rescan, as before. The code-10 message now says "15 s scan".

**Honest arithmetic: 15 s would not have prevented the code-10s, only halved them.**
The live log shows the same thing after two more restarts, so there are three cases:
21:35 (keyboard first seen ~19 s after start), 23:09 (~38 s), 23:15 (~67 s). With 15 s,
23:09 and 23:15 would most likely each have logged one code 10 instead of two. Absorbing
them completely would need a 30–60 s first scan. I stayed with your 15 s because a
longer scan also makes every "keyboard really is off" attempt that much longer. Say if
you want 30 s.

Side effect on the "keyboard off" test (step 6 of the TASKS_B hardware list): each code-10
attempt now takes ~15 s instead of ~5 s before the 8/16/32/60 s backoff. Nothing else
changes.

One counter-observation is recorded in §7.1: the 22:59 start found the keyboard within
2 s, a few minutes after the live stack's last upload. When that stack was stopped isn't
logged, so it doesn't refute the "still holding the old connection" idea. It doesn't
support it either.

## Task 3: block timeout, decided

I took the preferred option. **10000 ms is now the uploader's own default**, so the
packaged supervisor gets it with no change, and the hand-added `--block-timeout-ms 10000`
is dropped from `tiga-dev\tiga_startup_supervisor.ps1` (it would be redundant). Rule 6
said don't discard that edit; dropping it is what Task 3's option 1 asks for, and the
behaviour it added is kept. `ble.log` now prints `Block timeout   : N ms` in its header,
so future logs say which value was in effect. RETRY_RECOVERY.md has a new "Block
timeout" section. §7.1 records it as an **unverified mitigation**.

**Consequence: the binary must match.** A `tiga-dev` run with an *old* binary would now
use 2500 ms, because the supervisor no longer passes the argument. I rebuilt it, so
`tiga-dev` is consistent. **The live install keeps 2500 ms** until this branch is merged,
pulled into `tiga-fresh-test` *and* the uploader there is rebuilt.

## Task 4: power-cycle wording

Short form in: the supervisor instruction (→ `uploader_halted.json` and `STATUS_STACK.ps1`),
the "halted at startup" log line, `START_STACK.ps1` (dev and packaged), and the
uploader's code-20/21 exit meanings and session-marker refusal. Long form in README (the
limitations bullet and the recovery steps), `BUILD_AND_RECOVERY.md`, `RETRY_RECOVERY.md`
(step 6 and both "if it goes wrong" rows) and `RELEASE_CHECKLIST.md` (Phase 4 and the
rollback note, whose §2.4 reference I changed to §7.1). Tests now assert on the new text,
including that "power switch (" no longer appears.

Left alone on purpose: the generic "power-cycle" mentions (the exit-code table's "halt,
power-cycle", code comments, legacy probes in `reference/`, the September bundle notes),
and `main`.

**Three places now read oddly, and I didn't change them, because I don't know the
answer:** README line ~274 ("If the uploader cannot reach the keyboard (**switched
off**, …) … turning the keyboard on is enough"), RETRY_RECOVERY step 4 ("Switch the
keyboard's Bluetooth off, **or turn the keyboard off**"), and RELEASE_CHECKLIST ~line 204
("Keyboard off …"). With no power switch, how do you turn it off for that test: the
mode toggle (BT / 2.4 GHz / wired), or Bluetooth off on the PC? Tell me and I'll fix
all three. Separately, RELEASE_CHECKLIST line ~204 still says code 10 ends in "a halt
with no power cycle needed". That's stale since the slow retry (it never halts now).

## Task 5: §7.1 and #14

§7.1 has a new second part covering: BT1 identified; the rejection path, the address
cache and the code-10 retry on hardware; the three restart-invisibility cases; no power
switch and the 10–15 s / 30 s result (one observation each, so 30 s is a working
minimum); the slowing and sudden signatures; the block-timeout experiment. I also
corrected §7's "what 1B:A3… is remains unknown" paragraph and the §7.1 paragraph that
assumed a power switch, and fixed a stray backspace character that had turned
`logs\ble.log` into `logs<BS>le.log`.

Two things the logs added beyond the task text:

- **The slowing signature, made precise.** The log prints every fifth block, so "each
  ACK slower than the last" can't be read directly. But blocks 6–10 each had to arrive
  within 2.5 s and together took ~9.6 s, so **at least four of the five took 1.8–2.5 s**,
  against ~0.15 s healthy. Sustained, not one hiccup.
- **Five dev uploads completed, not four.** The fifth (23:13:15, 9.62 s) came after
  TASKS_C was written. The live install then completed eight more at 2500 ms after
  23:15, which is part of why the timeout evidence stays inconclusive.

**#14 commented** with the hardware results and the two follow-up commits (not closed):
https://github.com/MichiG21/tiga-zoom75-windows-media-display/issues/14#issuecomment-6202678563

## Task 6: every upload, side by side

The table and answers are in §7.1 ("Every upload of 2026-10-15"): 42 uploads, read at 23:29.
Short version:

1. **21:05 was sudden** *(medium-high)*: block 4 arrived at about the healthy pace, then
   nothing. **21:39 was probably sudden** *(low-medium)*. These are placed using a
   derived ~0.42 s pre-block overhead, which the three failures with block data agree
   on to ±0.01 s.
2. **No drift before failures** *(high, at 5-block granularity)*. The upload just before
   21:39 was the fastest of the evening.
3. **22:44/22:46 as the uncleared 21:39 state: can't tell** *(low)*. The identical
   block-11 stall with different images suggests a stable keyboard-side state, but the
   signature changed from sudden to slowing, so the 10–15 s disconnect did *something*.
   It could have been a partial clear or a brown-out.
4. **Earlier detection** (proposal only): log per-block and CONTROL1/2 ACK latencies
   first. Look for a signal *before* CONTROL2, the only point where stopping is still
   safe. Inside the transfer, warn on a block over ~1 s rather than aborting sooner,
   because aborting is itself what creates the partial session.

Two patterns that came out of the table:

- **First-on-connection uploads are ~0.7 s faster, 6 of 6.** All of the difference is
  already there by block 5. Later uploads lose ~0.7 s in their first five blocks, then
  run at the same pace. That's no longer "two samples, maybe coincidence".
- **Failures cluster at connection start**: 4 of 5 were first uploads and 1 a second
  upload; 0 of the other 31 later uploads failed. This is confounded with
  "restarts follow resets and failures", so don't read it as cause.

Minor, not changed: `extracted\…bin.device` records `last_completed_at` only for the first
completion in each uploader process (by design: `device_recorded`), so the name of that
field slightly overstates it.

## Ready to merge?

**Verified on hardware** (`fix/device-selection`, 22:59–23:13, plus earlier `main` work):

- #14: service check; BT1 rejected and the display selected (23:06); address cache used
  (23:09); 6 uploads on the branch, 5 completed, the failure unrelated to selection
- the code-10 retry recovering from a temporarily invisible keyboard without a halt
- #4 (bug A): real code-20 halts at 21:05, 21:39, 22:44, 22:46 and 23:00, each held
  until START_STACK after a battery disconnect
- #6, #9, #10 and the Unicode crash fix (from the first part of §7.1)

**Not verified on hardware:**

- **Tonight's own changes**: the name ranking (cc07788), the 15 s scan (4e5b2ac), the
  10000 ms default as compiled in (224e771; the 10000 ms *behaviour* ran via the hand
  edit, but never near the limit), and the new instruction text (60e94c1). All are
  covered by offline tests only. The binary they're in has had a dry run only.
- the block-timeout change as a fix for anything: inconclusive by design until an upload
  comes near 2.5 s and survives, or a slowing run is seen with 10 s
- #15 (display-control false success) and #7's delivery-ack and shutdown stages
- #5 (bug B) in the field; #8 (bug E, paths with spaces)
- the killed-process / session-marker path of #4
- what BT1 exposes (its connect failed, so no `exposes …` line yet)

**My read:** mergeable once one short hardware session on the rebuilt `tiga-dev` binary
shows: a cold start with no `.device` file selecting `Zoom75 Tiga` without probing BT1
first; `Block timeout   : 10000 ms` in the header; and a restart logging the
`Nothing seen in 5 s; scanning up to 15 s` line, if the keyboard is invisible again.
Everything else on the branch has either run on hardware or only changes text. After
merging, pull into `tiga-fresh-test` **and rebuild its uploader**, or the live install
keeps 2500 ms.

---

# Font fallback run (TASKS_D.md), 2026-10-16, about 18:20–18:45

## Step 0

- `origin/main` (`5543de1`) contains all of `fix/device-selection` (merge PR #16).
- **`tiga-fresh-test` is live, `tiga-dev` is not:** the task action runs
  `tiga-fresh-test\tiga_startup_supervisor.ps1`. The running `tiga_stream_watch_flow.exe`,
  the renderer's Python and the supervisor's PowerShell are all from `tiga-fresh-test`.
  Newest supervisor lines: `tiga-fresh-test` 2026-10-16 17:52:11, `tiga-dev` 2026-10-15 23:09:49.
- `tiga-fresh-test` is on `main` at `5543de1`, level with `origin/main`, with nothing to
  report in `git status`. The five untracked files mentioned last run are gone.
- `tiga-dev` was clean. Branch `feature/font-fallback` created from `origin/main`.

## Commits on `feature/font-fallback` (pushed, no PR)

| Commit | What |
|---|---|
| `ff2f39d` | Task 3: CJK line breaks, font/missing-glyph log lines, JhengHei |
| `957590a` | Task 4: tests, preview script, 9 preview PNGs |
| `7274fd5` | Task 5: Morse subtitle (+ 2 previews, 6 tests) |
| `53af926` | Task 6: README, CHANGELOG, PROJECT_HISTORY §7.2 |
| (this) | these notes |

Python tests: **66, all passing** (47 before, plus 19 new). No PowerShell or Rust
code changed, so I didn't run those suites. Nothing was sent to the keyboard, and
nothing in any `extracted\` folder was written. `tiga-dev\extracted\session_01_bulk_stream.bin`
still has its 2026-10-15 23:13 timestamp.

## Task 1: the exact characters (from `tiga-fresh-test\logs\renderer.log`)

Confirmed. The renderer already logs `Title` / `Artist` / `Album` for every new media item,
so no new log line was needed for this.

| Field | Text | Decodes |
|---|---|---|
| Title | `･－･･ ･ － ･･ － －－･ －－－` | LETITGO |
| Artist | `･ ･－･ ･－ ･･･ ･ －･･` | ERASED |
| Album | same as title | LETITGO |

- U+FF65 HALFWIDTH KATAKANA MIDDLE DOT (the dot)
- U+FF0D FULLWIDTH HYPHEN-MINUS (the dash)
- U+0020 SPACE, **always single**. There is no wider gap and no `/`, so the word breaks
  in "LET IT GO" can't be recovered from the data.

Takanaka `トーキョーレギー`: U+30C8 KATAKANA LETTER TO, U+30FC KATAKANA-HIRAGANA PROLONGED
SOUND MARK, U+30AD KATAKANA LETTER KI, U+30E7 KATAKANA LETTER SMALL YO, U+30EC KATAKANA
LETTER RE, U+30AE KATAKANA LETTER GI.

The only other non-ASCII code points in all the live logs are U+00E4 ä, U+00FC ü
(`Alte Sünder`) and U+00D2 Ò (`SALÒ`). Segoe UI has all three.

What each font covers: Segoe UI has none of the Morse or katakana characters above.
Yu Gothic has all of them. Of the TASKS_D test set, Segoe UI already has U+00B7, U+2022,
U+2219, U+2212, U+2013, U+2014, U+25AC and U+2E3A. The preview `morse-like-symbols.png`
shows them drawn as real glyphs, not boxes.

## Task 2: how text is drawn (all in `tiga_windows_nowplaying_v2.py`)

- **Fonts:** `find_font()` loads `segoeui(b).ttf` (Arial as backup) from `%WINDIR%\Fonts`.
  `find_text_font()` wraps it in a `FontChain` with the fallbacks. It's cached per
  (size, bold). Only metadata text (title, artist, album) and the no-artwork `♪` use the
  chain. The clock, labels, duration and source name use the plain font (always ASCII).
- **Drawn:** `draw_text()`, run by run, with every run on the primary font's baseline.
  A run that is all primary font is drawn exactly as `draw.text()` did before.
- **Measured:** `text_width()`, with the same runs.
- **Wrapped and truncated:** `fit_text()`, which measures through `text_width()`. The
  ellipsis is added there too. `layout_text_block()` places the lines.
- **The Unicode crash fix** touched only `configure_output()` (stdout/stderr to UTF-8 with
  `errors="replace"`) and the snapshot helper's `encoding="utf-8", errors="replace"`. It
  never touched drawing. That's why it stopped the crash but left boxes.

Measuring, wrapping and drawing already went through the same chain.

## Task 3: what 7c93850 already did, and what I changed

**Already right, not rebuilt:**

- Primary font unchanged. Ordered fallbacks from `%WINDIR%\Fonts`, missing ones skipped
  without crashing.
- `.ttc` index: Pillow's default index 0 is the right face in every collection used
  (Yu Gothic, not Yu Gothic UI; YaHei, not YaHei UI; Meiryo, not Meiryo Italic). Now
  noted in a comment.
- Per-character choice, runs, shared baseline, glyph cache (built once per font and
  character, not per frame).
- The Morse code points. U+FF65 and U+FF0D resolve to Yu Gothic, which is why the band
  displayed on hardware at 23:52.
- Emoji don't crash. They come out monochrome from Segoe UI Symbol where it has them.

**Changed (`ff2f39d`):**

1. **CJK wrapping.** Before, a line broke only at spaces, and a run was broken between
   characters only when the *whole* run was wider than the box. So `ABC ラブ・ストーリーは…`
   put `ABC` alone on line 1 and wasted it. Now a line can break between any two kana or
   Han characters. A minimal kinsoku rule applies: no line starts with `ー ・ 、 。` or a
   small kana. Hangul still breaks at spaces, since Korean uses them. U+FF65 is
   deliberately not a break-anywhere character, so a Morse letter is never split. Latin
   words wrap exactly as before.
2. **Log lines.** At the first render:
   `FONT fallbacks: YuGothB.ttc, msyhbd.ttc, msjhbd.ttc, malgunbd.ttf, seguisym.ttf; not installed: meiryo.ttc`.
   (Meiryo isn't installed on this PC. That's normal on Windows 11, and Yu Gothic covers
   it.) A character no font has is logged once:
   `FONT no font has U+13000 EGYPTIAN HIEROGLYPH A001; drawn as a box`.
3. **Microsoft JhengHei** added after YaHei for Traditional Chinese.

**Not done, on purpose:**

- **No fontTools.** TASKS_D suggests it as the reliable glyph check. The existing check
  draws the character and compares it with the font's own missing-glyph box. It's
  indirect, but it's correct for every character tested and on hardware. It also needs
  nothing new, so **`requirements.txt` (Pillow only) and the installer build are
  unchanged**. The installer build is `.github/workflows/installer.yml`:
  `pip install -r requirements.txt pyinstaller`, then PyInstaller. `BUILD_AND_INSTALL.ps1`
  also installs from `requirements.txt`. The fonts come from the user's own
  `%WINDIR%\Fonts` at runtime, so nothing has to be bundled. If you want fontTools
  anyway, it goes in `requirements.txt` and both of those pick it up.
- **Fallback order kept as 7c93850 had it** (Japanese → Chinese → Korean → symbols), not
  TASKS_D's symbols-first. Segoe UI already has the common symbols. The one visible
  difference is that `♪` comes from Yu Gothic, not Segoe UI Symbol.
- **Han unification:** Chinese titles use Yu Gothic's (Japanese) glyph shapes for shared
  characters. They're readable (see `chinese.png`), but not the native Chinese forms.
  Fixing that would need a per-string language guess, so I left it.

## Task 4: tests and previews

New tests in `tests\test_renderer.py` (`FontFallbackCoverageTests`, 13 tests), covering
every bullet in Task 4: the logged Morse code points and the 8 Morse-like symbols;
`渚・モデラート`, `月亮代表我的心`, `밤양갱`; `高中正義 — Blue Lagoon` (both fonts used); a
long Japanese title with no spaces wraps inside 174 px, loses nothing, and no line starts
with `ー`/`・`; CJK fills the line after a Latin word; Morse letters are never split; a
non-existent fallback font is skipped with one log line; a missing code point is logged
once and still drawn; emoji don't crash.

**Latin pixel identity:** full 320×172 frames of four Latin titles, compared with the
renderer using the plain font and a verbatim copy of the pre-fallback `fit_text`. I also
checked once by hand against `128745d` (old `main`) itself: identical, once the
no-artwork tile is the same in both. 7c93850 deliberately changed that tile from a box
to `♪`.

Tests needing a CJK/Korean font skip cleanly if it isn't installed (e.g. on a CI image).

**Previews** in `previews\font-fallback\` (committed), written by
`py tests\render_font_previews.py` (PNGs only): `morse-band-logged`, `takanaka-logged`,
`morse-like-symbols`, `japanese`, `chinese`, `korean`, `mixed-latin-japanese`,
`long-japanese-no-spaces`, `latin-only`, plus `morse-subtitle-logged` and
`morse-subtitle-word-gaps`. I checked them by eye: no boxes, and CJK sits on the Latin
baseline. The long Japanese title fills three lines and ends in `…`. Its third line sits
close to the artist line, which is the existing #10 layout, not new.

**Render time** (`render_media`, mean of 50, no artwork): old `main` 2.1 ms (Latin) /
1.8 ms (Japanese, as boxes); new 1.4 ms / 1.5 ms. That's noise. The first frame with new
characters pays a few ms once for the glyph cache. Upload time is ~8–10 s, so this is
irrelevant either way.

## Task 5: Morse subtitle (done)

`decode_morse()` accepts dots `･ . · • ∙` and dashes `－ - _ − – —`, single spaces between
letters, and `/` or 2+ spaces between words. It returns nothing unless **every** letter
decodes and there are at least two letters. So `...`, `-`, `—`, `Hello - World` and
`.-.. . ........` all show nothing. A Morse field gives up one of its lines to the
decoded text: 14 px bold, light blue.

- **Setting:** the renderer's settings are its command-line flags (`--dlx-frames`,
  `--dlx-layout`), so it's `--no-morse-subtitle`. It's **on** by default, and no
  supervisor change was needed.
- **Layout, real band (`morse-subtitle-logged.png`):** title (2 Morse lines) + `LETITGO`,
  artist + `ERASED`, then the album has room for **one** line only. That line is the
  Morse album, truncated with `…`, and no decoded line. Nothing overlaps, and the title
  and artist look fine. The album line is the weak spot: a truncated Morse line says
  little. Two possible changes, your call: (a) when the album has room for one line and it
  decodes, show the decoded text instead of the Morse; (b) leave it.
- **`LETITGO` without spaces** is the data, not a bug (single spaces only, see Task 1).
  `morse-subtitle-word-gaps.png` shows `LET IT GO` when the gaps exist.

## Task 6: docs

README feature bullet. CHANGELOG got a new `## Unreleased` section (there was none; the
file only had the 2026-09-09 bundle). PROJECT_HISTORY has a new **§7.2** (crash fix vs.
display fix, the Morse code points, the decisions above), and §7.1's "boxes" line now
points to it.

## Something odd in the tooling, for future runs

Escape sequences typed into shell heredocs came out as literal characters in the files.
So `MORSE_LOGGED` in the tests holds the literal `･－`, not escapes. That's harmless. The
renderer's `MORSE_DOTS`/`MORSE_DASHES` were rewritten to real escapes, so the look-alike
characters can be told apart.

## What to do on hardware

The change is renderer-only (Python). The uploader and supervisor on this branch are the
same as `main`. `tiga-dev`'s uploader binary was rebuilt on 2026-10-15 from the code now
on `main`, so it matches.

1. **Stop the live stack:** `C:\Users\Regal\source\tiga-fresh-test\STOP_STACK.ps1`.
2. **Point the task at `tiga-dev`:** run
   `C:\Users\Regal\source\tiga-dev\install_tiga_startup.ps1`. It registers the task at
   its own folder. Check:
   `(Get-ScheduledTask "TIGA Windows Media Display").Actions.Arguments` shows `tiga-dev`.
   `tiga-dev` must stay on `feature/font-fallback` meanwhile.
3. **Start:** `C:\Users\Regal\source\tiga-dev\START_STACK.ps1`.
4. **Check `tiga-dev\logs\renderer.log`:** after the first media frame, a line
   `FONT fallbacks: YuGothB.ttc, msyhbd.ttc, msjhbd.ttc, malgunbd.ttf, seguisym.ttf; not installed: meiryo.ttc`.
5. **Play the Morse band** (the `･－･･ ･ － ･･ …` track). On the keyboard: Morse title on
   two lines, then `LETITGO` in blue. Morse artist, then `ERASED` in blue. One truncated
   Morse album line. Compare with `previews\font-fallback\morse-subtitle-logged.png`.
6. **Play Takanaka `トーキョーレギー`**: katakana, no boxes, on the same baseline as Latin.
   If you have one, play a long Japanese title too. It should wrap within the right-hand
   column and never run off the edge.
7. **Play an ordinary Latin track.** It should look exactly as it does now.
8. If any character shows as a box, `renderer.log` has a `FONT no font has U+…` line.
   Send me that.
9. **Switch back:** `tiga-dev\STOP_STACK.ps1`, then
   `C:\Users\Regal\source\tiga-fresh-test\install_tiga_startup.ps1`, check the task
   arguments show `tiga-fresh-test`, then `tiga-fresh-test\START_STACK.ps1`.

If a transfer gets interrupted during the test, that's the known BLE issue (§7.1), not
this branch. The same battery-disconnect procedure applies.

## Ready to merge?

**Yes, in my view, after one look on hardware (steps 5–7).** The basic fallback was
already verified on hardware last night. This branch adds wrapping, logging, one font and
the Morse line. Latin output is pixel-identical by test, all 66 Python tests pass, and
nothing in the dependencies or the installer build changes. The one open choice is the
Morse album line (Task 5, (a) or (b)), and it can follow after a merge. Not verified
anywhere: how a CI `windows-latest` image's font set affects the CJK tests. They're
written to skip, not fail, if the fonts are missing.

## Follow-up: Morse album line, option (a) (2026-10-16)

Done in `61e4ffc`, as you chose. A field that has room for **one** line and decodes now
shows only the decoded text. Under the band's title and artist, the album is now
`LETITGO` in blue instead of a truncated Morse line. An album that doesn't decode, or
the subtitle turned off (`--no-morse-subtitle`), behaves as before. Two new tests (68
Python tests, all passing). Only `morse-subtitle-logged.png` changed; every other preview
is byte-identical. PROJECT_HISTORY §7.2 has one sentence on it.

**Corrects hardware step 5 above:** expect Morse title + `LETITGO`, Morse artist +
`ERASED`, then **`LETITGO` again** (the album) instead of a truncated Morse line.

One thing to judge on the keyboard: `ERASED` and the album's `LETITGO` sit directly under
each other in the same blue, so they read a little like one block. If that's confusing,
the album's decoded line could use the album grey instead.

PR into `main`, not merged: see the link in the run summary.
