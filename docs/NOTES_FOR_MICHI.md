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

---

# The 2026-10-16 18:48–18:55 restart, and the block timeout

Branch `docs/restart-2026-10-16`, off `origin/main` (`e8f7211`, after #17). Docs only,
pushed, no PR. Everything here is read from `tiga-fresh-test\logs\` and `git reflog`; the
40 s battery disconnect is from you. Nothing was run against the keyboard.

## What happened (now in PROJECT_HISTORY §7.1, "Third part")

- **18:48:32** you pulled #17 into `tiga-fresh-test`. The 17:52 stack had just completed 22
  uploads in a row, the last at 18:46:00. The restart at 18:48:44 did not reset the keyboard.
- **18:49:00 code 10.** A 15 s scan saw nothing. The 8 s retry found `Zoom75 Tiga` 3 s into
  its scan, so the keyboard was invisible for about 28 s. That's the fourth case of this
  (19, 38, 67 s last night).
- **18:49:14–18:50:32**, frame `0714ef6a` (the Morse band, first frame with the subtitle):
  **the slowing signature**. Block 5 at 2.42 s (a healthy first upload is 0.71–0.85 s), then
  ~1.2–2.5 s per block for 35 blocks, never recovering, then **no PROGRESS for block 41
  within 10 s**. Code 20 after 77.83 s, halt.
- **40 s battery disconnect, START_STACK 18:54:07.** The **same frame `0714ef6a`
  completed in 9.06 s** at a healthy pace (block 5 at 0.85 s), and Takanaka after it in
  9.52 s. Identical bytes, so the failure was not caused by the image. A 40 s disconnect
  is enough.

Small extra: Takanaka's frame `4bf3daed` is byte-identical to last night's, so the CJK
wrap change didn't touch it, as intended. The renderer logged
`FONT fallbacks: YuGothB.ttc, …; not installed: meiryo.ttc` at both starts. Whether the Morse
subtitle *looked* right on the keyboard isn't in the logs. Tell me if it didn't.

## Block timeout: the verdict

- **27 completed uploads with 10000 ms** (5 last night in `tiga-dev`, 22 today live). After
  block 5, the slowest five-block span averaged 0.21 s per block, so **no block in any of
  them took more than ~1.05 s**. Every one would also have completed with 2500 ms.
- **The one slowing run under 10000 ms didn't recover.** It crawled to block 40, stalled
  completely, and ended exactly like the 2500 ms failures (code 20, battery). It just took
  78 s instead of failing at the first block over 2.5 s, probably somewhere around block 25–35,
  possibly earlier (five-block granularity can't place it).
- So the open question in §7.1 ("with 10 s, would the 22:44 transfer have crawled to
  completion?") has its first answer: **no, it stalled anyway, only later.** That's one
  run, n = 1.

## Proposal: go back to 2500 ms

**Recommend: revert the uploader default to 2500 ms**, keep `--block-timeout-ms` and the
`Block timeout   :` header line. Confidence medium. Reasons:

1. 10000 ms has **rescued nothing** in 27 uploads, and the idea behind it (a block that's
   merely late and then recovers) has not been seen once. Healthy blocks peak around
   1 s, so 2500 ms still leaves well over 2× headroom.
2. In its only test it **delayed the halt by about a minute** with the same outcome. The
   keyboard spends that minute with a media session half-open. Whether that's harmful is
   unknown, but it's no help either.
3. Neither value avoids the battery disconnect; the timeout only decides *when* you're told.

**What argues for keeping 10000 ms:** it costs nothing on healthy uploads, and the 78 s
crawl taught us something (a slowing run doesn't recover). But per-block logging would
capture that, without the wait. That's the better tool: a `SLOW` line for any block over
~1 s, and on a first-on-connection upload a warning if block 5 is past ~1.5 s (healthy
first uploads 0.71–0.85 s, all three slowing runs 2.35–2.97 s). Both are still after
CONTROL2, so they warn rather than prevent.

If you agree, it's a one-line default change plus the RETRY_RECOVERY "Block timeout"
section and the test that asserts 10000. Say the word and I'll do it on a branch. I
haven't changed any code here.

## The more useful finding: failures follow restarts

Counting every uploader start on 2026-10-15/16 by what came before it:

| Before the first upload | Failed | Completed |
|---|---:|---:|
| Restart without a reset, last upload had completed | **3** (21:05, 22:59, 18:49) | 6 |
| Battery disconnect ≥ 30 s | 0 | 4 |
| 10–15 s disconnect, or no reset after a halt | 2 | 0 |
| Later upload on an open connection | 1 (21:39) | 62 |

**The risky moment looks like the first upload after a stack restart that didn't reset
the keyboard.** `STOP_STACK.ps1` ends the uploader with `Stop-Process -Force`, so every
restart drops a live BLE connection without a clean disconnect. That might also be why the
keyboard is invisible for 20–70 s afterwards (a keyboard that thinks it's still connected
doesn't advertise). **Hypothesis only.** Testing it would mean making the uploader
disconnect cleanly on a stop request, then watching whether first uploads after restarts
stop failing. That changes the stop path, so it needs your go-ahead and hardware time. I
haven't touched it.

Practical advice until then: after any `STOP_STACK`/`START_STACK` (a pull, a branch
switch), expect the first upload to be the risky one.

---

# Unattended run E (TASKS_E.md), 2026-10-16, about 19:20–20:30

## Most important: nothing was merged

**The merge of #18 was refused by Claude Code's permission system** (auto mode classifier:
"Merge Without Review"), even though every check had passed and the PR was mergeable. I
did not retry it or work around it. Following the plan's fallback, everything after it is
stacked instead of merged:

- **#18** (Task 1, 2500 ms): open, green, mergeable, **not merged**.
- **#19** (Task 2, diagnostics): base `docs/restart-2026-10-16` so its diff is only
  Task 2. Green. **Not merged** (the plan says: if Task 1 isn't merged, don't merge Task 2).
- **#20** (Task 3, clean stop): base `feature/block-diagnostics`. **Never for merge**
  before hardware.
- `docs/issue-triage` (Tasks 4–5 and these notes): pushed; its PR is opened at the end
  of this run, **not merged**, for the same reason.

**What you need to do first:** merge #18 into `main`. Then change #19's base to `main`
(GitHub: Edit → base) and merge it. After that, #20's base can go to `main` as well, but
leave it open until its hardware test. If you want future unattended runs to be able to
merge, that needs a Claude Code permission rule for `gh pr merge`; I didn't touch any
settings.

One oddity for future runs: right after that refusal, two harmless commands (creating
a local branch, and a `grep`) were refused with the same reason. Repeating them
separately worked. It looks like the classifier stays primed for a moment after a denial.

## Tasks

| Task | Result | Branch | Commits | PR | Merged |
|---|---|---|---|---|---|
| 1. Block timeout 2500 ms | done | `docs/restart-2026-10-16` | `25c8708` | [#18](https://github.com/MichiG21/tiga-zoom75-windows-media-display/pull/18) | **no**, refused by the permission system |
| 2. Per-block diagnostics | done | `feature/block-diagnostics` (from Task 1's branch) | `5daaa13`, `75c312d` | [#19](https://github.com/MichiG21/tiga-zoom75-windows-media-display/pull/19) | no (Task 1 not merged) |
| 3. Clean stop | done, prepared only | `feature/clean-stop` (from `feature/block-diagnostics`) | `c359c73`, `8037670`, `54f0fb8` | [#20](https://github.com/MichiG21/tiga-zoom75-windows-media-display/pull/20) | no, by design |
| 4. Issue triage | written, **not committed** (see below) | — | — | — | — |
| 5. Release draft | done | `docs/issue-triage` (from `origin/main`) | `cce541b`, `d06b808` | [#21](https://github.com/MichiG21/tiga-zoom75-windows-media-display/pull/21) | no (not attempted after #18 was refused) |
| 6. CI fonts, old plans | done | — | — | — | — |

### Task 1

The default is `2_500`, with a new doc comment giving the reason. `--block-timeout-ms` and
the `Block timeout   :` header line are unchanged. The test that asserted 10000 now
asserts 2500. RETRY_RECOVERY "Block timeout" is rewritten: the default, the 10000 ms day,
and why it went back. I also dropped its sentence "a block that is merely slow should
therefore be waited for", since the new text explains that case was never seen.
PROJECT_HISTORY §7.1 gains one sentence in the second part's update, and a
**"Decision, 2026-10-16"** paragraph after the third part's verdict. Tests: 35 Rust
passed; release build; the dry run prints `Block timeout   : 2500 ms`. All three CI
checks are green.

### Task 2

- **Block latency** = previous PROGRESS → this PROGRESS (block 1: from the start of the
  bulk writes), so it includes the block's own writes. That's the quantity §7.1 calls
  "per block ~0.15 s".
- **CONTROL1/CONTROL2 ACK time** = from the write to the second ACK of the pair.
- `SLOW block 7/53 took 1834 ms (elapsed=4.21s)` for a block over 1000 ms (timestamped).
- `WARNING first upload is slow (block 5 at 2.42 s; healthy 0.7–0.85 s)` when the first
  upload of a connection ACKs block 5 later than 1.5 s. "First" means the first *attempt*
  on that uploader process.
- After every `UPLOAD COMPLETE` / `UPLOAD FAILED`:
  `TIMING frame=… blocks_acked=53 median=…ms p90=…ms max=…ms slowest=block N control1_ack=…ms control2_ack=…ms`.
  p90 is nearest-rank. On a failure the figures cover the ACKed blocks, and `-` marks a
  CONTROL stage that wasn't reached.
- `.delivered` gains `block_median_ms`, `block_p90_ms`, `block_max_ms`, `slowest_block`,
  `control1_ack_ms`, `control2_ack_ms`. I checked every reader (STATUS_STACK both
  layouts, the supervisor, display control): all read by key, so extra keys are safe.
- **No behaviour change:** only `Instant::now()` calls and log lines are added; writes,
  sleeps, timeouts and exit codes are identical. One tiny difference: `UPLOAD FAILED …
  after X s` is now measured a few microseconds earlier (before the summary is computed),
  not after.
- 5 new tests (40 Rust). The thresholds are tested against the recorded values: 2.35,
  2.42 and 2.97 s warn; 0.71, 0.85 and 1.5 s don't; a later upload at 2.04 s doesn't.
- Docs: RETRY_RECOVERY "Per-block timing", CHANGELOG (also notes the 2500 ms return),
  §7.1 question 4 now says the logging part is implemented.

### Task 3 (details in `docs/design/CLEAN_STOP.md`)

- **Mechanism: `<stream>.stop`**, a sidecar like `.session` and `.exit`. The supervisor
  can see it too, which is what stops it restarting the uploader. Windows has no SIGTERM,
  and Ctrl+Break would need the hidden `cmd /c` console.
- **Uploader:** checks the file only at the top of its watch loop, i.e. between uploads,
  so an upload is never abandoned for a stop. Then the existing `disconnect()` and exit 0.
  If the file exists at start, it doesn't connect. It does *not* interrupt a scan or a
  connect in progress.
- **STOP_STACK (both):** writes the request, waits **30 s** (a healthy upload is ~10 s,
  a failing one ends within ~15 s at 2500 ms, and a scan can take 15 s), then
  `Stop-ScheduledTask` and the old force-kill, then removes the request. It logs one of
  `exited on request after X s` / `did not exit within 30 s; force-killing it (pid N)` /
  `no BLE uploader was running`, to the console and `startup_supervisor.log`. The logic is
  `Invoke-StackStop` in `tiga_supervisor_common.ps1`, with the task stop and the kill
  passed in as script blocks so the tests can use stand-ins.
- **Supervisor:** an exit while the request stands is `stopped-on-request`. It isn't
  restarted and isn't counted as a failure, but a marker or code 20/21 still halts. No
  uploader starts while the request exists. A stale request is removed at supervisor
  start (the logon task doesn't go through START_STACK), and START_STACK removes one too.
  If the request vanishes while the supervisor still runs, the supervisor resumes and
  logs it, like a cleared halt.
- **Honest caveat, also in the doc:** in btleplug 0.12 on Windows, `disconnect()` just
  drops the device object, `Close()`-ing its GATT services and the `BluetoothLEDevice`.
  A killed process releases the same objects without those calls. The radio may see no
  difference. The certain gain is that a stop never cuts an upload in half; whether the
  restart pattern changes is what the hardware plan has to show. No GATT unsubscribe was
  added: that would be a write the keyboard has never had from this uploader.
- Tests: Rust `stop_request_is_a_sidecar` (41 total). In `test_supervisor_retry.ps1`, the
  decision table, three loop scenarios with dummy uploaders, `Invoke-StackStop` with
  dummies (cooperating, ignoring, none), and text checks on both STOP/START_STACKs. All
  pass, and so does `test_supervisor_logging.ps1`. I read both suites first: they kill only
  their own dummy PIDs, and they read STOP/START_STACK as text without running them.
  **Not tested:** driving the uploader's `run()` offline. With a bug, that would open
  Bluetooth.

### Task 4: triage written, but the commit was refused

`docs/ISSUE_TRIAGE_2026-10-16.md` exists in `tiga-dev\docs\`, but it is **not committed**.
`.gitignore` has `docs/ISSUE*_*.md` ("Local task files and issue drafts (GitHub is the
record once filed)"), so a plain `git add` ignores it. My `git add -f` was refused by
the permission system ("Out-of-Place Publication"). I didn't rename the file to dodge
the ignore rule. Your call: `git add -f` it yourself, rename it (e.g.
`docs/TRIAGE_2026-10-16.md`), or keep it local, as the ignore rule intends. It is ignored,
so the tree is clean either way.

Proposals in it: **close** #4 (at the release, as your comment said), #5 (reopen if
seen), #6, #8, #9, #10 (all seven boxes have commits), #14. **Keep open** #7 and #15
until one Display Control session on hardware. Each has a ready-to-paste comment, plus a
3-step GUI session that would settle #7 and #15 together. #11–#13 are PRs, not issues.

### Task 5

- `RELEASE_CHECKLIST.md`, stale text fixed: Phase 4's last step now says codes 10/11
  retry every 5 min and never halt, and that no START_STACK is needed. Phase 5 is marked
  decided (`dc30996`). The expected test counts are 68 Python / 35 Rust (they were
  39 / 16). The "Keyboard off" / "Turn it on" words are left as they were.
- `docs/RELEASE_NOTES_v0.3.0-beta.1.md`: Part 1 lists the blockers, Part 2 is the
  user-facing text. It notes where the text depends on #18/#19 being merged, and has a
  placeholder for "installing over v0.2.0-beta.1".

### Task 6

- **CI fonts: they ran.** In the latest `main` run (38934771243, the #17 merge) all 13
  `FontFallbackCoverageTests` and the 7 `FontFallbackTests`, including `test_japanese`,
  `test_chinese` and `test_korean`, report `ok`. There are 0 `skipped` lines, 68 tests,
  `OK`. So `windows-latest` has Yu Gothic, YaHei and Malgun Gothic, and the skip paths
  are just insurance. Nothing changed in CI.
- **Old plans:** `TASKS.md` and `TASKS_A.md` copied to
  `C:\Users\Regal\source\old-loose-files\`. Nothing was there under those names, so
  nothing was overwritten. The originals and the stale folder are untouched.

## Decisions I made that you would normally make

1. **Stacked PRs** for #19 and #20 (bases `docs/restart-2026-10-16` and
   `feature/block-diagnostics`), not PRs into `main`. Into `main` they'd have shown Task
   1's diff (and Task 2's) as well. Retarget them once the lower ones merge.
2. **Block latency includes the block's writes** (PROGRESS to PROGRESS), not "last write
   to PROGRESS". That matches §7.1's per-block figures, and every past log is comparable.
3. **SLOW and WARNING lines carry a timestamp** (`log!`), unlike the every-fifth-block
   lines, so they can be grepped and lined up with other logs.
4. **Clean stop: a request file, 30 s, exit 0, no unsubscribe**, and a withdrawn request
   restarts the uploader. Reasons above.
5. **The triage file was left uncommitted** rather than renamed past `.gitignore`.
6. **CHANGELOG not completed** for #11–#16 on `docs/issue-triage`, to avoid a conflict
   with #19's CHANGELOG lines. It's blocker 3 in the release draft.
7. **The checklist title `v0.2.0-beta.2` was left alone.** The version name is yours
   (release draft, blocker 2).

## Contradictions with PROJECT_HISTORY.md

None found. Two nuances:

- CLEAN_STOP.md adds that on Windows a `disconnect()` and a kill may well look the same
  to the keyboard. That weakens, but doesn't contradict, §7.1's "kill, not a clean
  disconnect" hypothesis.
- `main`'s §7.1 still says the longer wait is the uploader's default. That's true on
  `main` until #18 merges; #18 updates it.

**Merge conflict to expect:** `docs/issue-triage` and `docs/restart-2026-10-16` both
append to the end of `NOTES_FOR_MICHI.md`. Whichever merges second will conflict there.
Keep both sections, restart section first.

## What needs you or the keyboard, in order

1. **Merge #18** (no keyboard needed). Retarget #19 to `main`, check it's green, merge.
2. **Deploy** (below), with a 30–40 s battery disconnect. Check the header and the
   `TIMING` lines.
3. **Decide** on the triage file (commit / rename / local), and post the issue comments
   you agree with.
4. **One Display Control session** (triage draft, last section) → settles #7 and #15.
5. **Answer the "turn the keyboard off" question** (README ~275, RETRY_RECOVERY step 4,
   RELEASE_CHECKLIST Phase 4's last step): mode toggle, or Bluetooth off on the PC? Then
   I can fix all three.
6. **The clean-stop hardware test** (#20, `docs/design/CLEAN_STOP.md`). At least 10
   restarts without a reset, counting first-upload failures and the invisible period.
   Do it after the TIMING lines are live, so every first upload has its block-5 time.
7. **The release** (`docs/RELEASE_NOTES_v0.3.0-beta.1.md`, Part 1): version name,
   CHANGELOG, the killed-uploader test, and the installer end to end, including over
   v0.2.0-beta.1.

## Deploy steps

**Right now `main` has nothing new to deploy:** #18 and #19 aren't merged, so a pull
would change nothing. Once you've merged **#18, and #19 retargeted to `main`**:

```
C:\Users\Regal\source\tiga-fresh-test\STOP_STACK.ps1
cd C:\Users\Regal\source\tiga-fresh-test
git pull
cd windows_ble_probe
cargo build --release --locked
```

Then disconnect the battery for 30–40 s (pressing a few keys), reconnect, and:

```
C:\Users\Regal\source\tiga-fresh-test\START_STACK.ps1
```

Check the newest `tiga-fresh-test\logs\ble.log`:

- the header says `Block timeout   : 2500 ms`
- after the first `UPLOAD COMPLETE`, a line
  `TIMING frame=… blocks_acked=53 median=…ms p90=…ms max=…ms slowest=block N control1_ack=…ms control2_ack=…ms`.
  Expect a median of about 150 ms. On this first-on-connection upload, no
  `WARNING first upload is slow` line should appear
- `extracted\session_01_bulk_stream.bin.delivered` has the new `block_*` /
  `control*_ack_ms` keys

If only #18 is merged, there are no TIMING lines; the 2500 ms header is the only check.
**Task 3 (#20) is not part of this pull**, and STOP_STACK still force-kills.

`tiga-dev` is left on `docs/issue-triage` with a clean tree (the triage file is ignored).
Its uploader binary was last built from `feature/clean-stop`, so it has Task 3. Rebuild
it before using `tiga-dev` for anything else.

---

# Follow-up to run E, 2026-10-16 evening: `docs/issue-triage` brought up to date

**#21 is ready for you to merge; I left it open.** Nothing was posted on any issue.

## Checked first

After `git fetch`, `origin/main` (`56c3825`) contains #18 (`019da37`, with `25c8708`) and
both timing commits, `5daaa13` and `75c312d`. They came in through #19 (merged into
`docs/restart-2026-10-16`) and then #22. So the plan went ahead.

## Done, on `docs/issue-triage`

| Commit | What |
|---|---|
| `4e192f9` | Merged `origin/main`. The `NOTES_FOR_MICHI.md` conflict is resolved by keeping both sections: the 2026-10-16 restart section first, then run E. Nothing else conflicted. |
| `393a75c` | `docs/TRIAGE_2026-10-16.md`, the triage draft renamed from `docs/ISSUE_TRIAGE_2026-10-16.md` so `.gitignore` no longer catches it. One added line notes that #18 and #22 merged since and belong to no open issue, so no proposal changes. |
| `03c4f80` | Release draft: blocker 1 (merge #18/#19) is now done. What's left of it is hardware: pull #22 into the live install and look for a `TIMING` line. The keyboard-off blocker is marked done, the CHANGELOG blocker is narrowed to #11–#16, the triage path is updated, and the "(if #18/#19 is merged)" conditionals are gone from the release text. "The new per-block timing lines" is added to "not yet verified on the keyboard". |
| `5a7543e` | The three "turn the keyboard off" spots, worded as you said (below). Also the checklist's Phase 5 cross-reference to that step, and its expected test count (40 Rust now). |
| (with these notes) | Release draft: CI on `main` is green after #22. |

### The keyboard-off wording

All three now say that the TIGA has no power switch, so "off" means the battery
disconnected from the PCB. Do it only while no upload is running: **Status** says OK, or
`ble.log` ends in `UPLOAD COMPLETE`, because a disconnect mid-upload halts.

- **README** (the paragraph after the halt recovery steps): the uploader retries and
  picks the keyboard up within about 5 minutes with no Start Display. A new paragraph
  explains "off", the no-upload-running check, and that this is not the halt recovery
  (that needs ≥ 30 s and Start Display).
- **RETRY_RECOVERY §4** ("Retryable: keyboard unreachable"): opens with "not a halt test,
  not the halt recovery of 2.6". Step 1 is the no-upload check, and then the battery
  disconnect. Step 2 is: change track, keep it disconnected 1–2 minutes for a few code-10
  retries. Step 4 is: reconnect, expect an upload within 5 minutes with no START_STACK. It
  also says the 30 s rule (§7.1) is for halt recovery only. I removed the old "switch the
  keyboard's Bluetooth off" alternative, because you defined "off" as the battery.
- **RELEASE_CHECKLIST Phase 4:** the last step is rewritten the same way, labelled "a
  separate test, not halt recovery".

**Not changed, a fourth spot with the same wording:** `docs/design/SHUTDOWN_ACK.md:108`,
"Keyboard switched off: expect the delivery-timeout dialog at about 90 s". You asked for
three, so I left it. The same definition would apply, with one difference: that test
*wants* the standby upload to fail, so the disconnect has to happen before pressing
Standby + Shut Down. Say if you want it changed.

## Tests (all on this branch after the merge)

68 Python (`py -m unittest discover -s tests`), 40 Rust (`cargo test --release --locked
--bin tiga_stream_watch_flow`), `test_supervisor_logging.ps1` and
`test_supervisor_retry.ps1`: all passed. I read both PowerShell suites first: they use
dummy children killed by PID, and they read START/STATUS_STACK as text or from a temp
copy. Release build plus dry run: `Block timeout   : 2500 ms`. `tiga-dev`'s uploader
binary is now built from this branch (timing code in, no clean-stop code).

## Still open for you

- **Merge #21.**
- **Pull #22 into the live install and rebuild its uploader.** It's at `019da37` (#18
  only): `ble.log` already shows `Block timeout   : 2500 ms`, but no `TIMING` lines yet.
  The steps are the deploy steps in the run E section; the battery disconnect is optional
  this time, since the stack runs healthily.
- #20 (clean stop): its base is still `feature/block-diagnostics`. Retarget it to `main`.
  Leave it open until its hardware test.
- The rest of the list in run E's "What needs you or the keyboard" still holds, except
  steps 1, 3 (the triage file is now committed; posting comments is still yours) and 5.

---

# The 2026-10-16 20:45–20:56 session in PROJECT_HISTORY (§7.1, fourth part)

Branch `docs/session-2026-10-16-evening`, off `origin/main` (`5ca54b3`, after #21). Docs
only. The PR into `main` is open and **not merged**. Everything is read from
`tiga-fresh-test\logs\`, the `.ack` and `.delivered` files and `git reflog`; nothing was
run against the keyboard and no issue was touched.

## What's in it

- **#22 deploy:** pulled 20:49:06, uploader rebuilt 20:49:26, started 20:51:27.
  `Block timeout   : 2500 ms`. The first upload took 9.17 s: `median=158ms p90=180ms
  max=210ms control1_ack=18ms control2_ack=76ms`, block 5 at 0.90 s.
- **Code 11 after the battery reconnect:** the first one from the real keyboard (all four
  earlier ones, 2026-10-15, were BT1). The media service `1f40eaf8…` was present with no
  characteristics. The retry 8 s later qualified it. **Detail you didn't mention:** the
  scan saw the cached address with an **empty name** (`CANDIDATE name=""`). Only the
  address cache made it a candidate at all.
- **Display Control:** Lock Standby, Lock Standby again and Resume Automatic. Each was
  acted on by the renderer within a second and uploaded (11.38 / 10.68 / 9.97 s). The
  surviving ack matches the third command, written 0.9 s after the command file. That is
  recorded as verifying #15 and #6's GUI path. Standby + Shut Down (#7) wasn't pressed.
- **TIMING finding**, with a table of all seven uploads in §7.1. The median stays at
  150–158 ms. Later uploads have one slow block among blocks 1–3, and §7.1 now says this
  answers its own question about *where* later uploads lose ~0.7 s.

## Where I wrote it differently from your summary

1. **control1_ack doesn't grow steadily.** It was 18 ms on the first upload, then 166,
   577, **653**, then back down to 178, 170 and 357 ms. "18 → ~650 ms" is the range, not a
   trend, and §7.1 says so.
2. **"One or two slow early blocks" can't be read from the data.** TIMING gives only the
   slowest block. §7.1 says "at least one", and notes that a p90 of 172–195 ms allows at
   most five blocks over ~0.2 s.
3. **0.7–1.1 s holds for five of the six later uploads.** The sixth (20:55:28) had its
   slowest block at 0.42 s (block 3).

## Also found in the same logs

- **20:46:17, before the deploy (#18 only, no TIMING yet): a 20.01 s upload that
  completed.** It's the slowest completed upload on record. Block 5 was at 3.90 s, then
  every five-block span took 0.9–2.3 s, slow throughout with no recovery, but no block
  reached 2.5 s. The same image (`525691c8`) took 9.17 s after the deploy. It's in §7.1
  as "uniformly slow but finished".
- **20:54:26 reached block 5 at 2.39 s and completed fine** (one slow block 2 at 1079 ms,
  then a normal pace). 2.39 s is inside the slowing failures' 2.35–2.97 s range, so it
  shows why the early warning is limited to first-on-connection uploads.
- **A freshly built uploader starts ~25 s late on its first run** (20:20:54 → first line
  20:21:20; 20:51:27 → 20:51:52). The code-11 retry with the same binary started at once.
  That fits antivirus inspecting a new exe (#15's theory). Harmless.
- **The #18 start at 20:20:54** logged another code 10 after a restart (the fifth such
  case). Its retry found the keyboard immediately.

## Triage

`docs/TRIAGE_2026-10-16.md`: **#15 is now proposed for closing**, with a new hardware
paragraph and a new ready-to-paste comment. #6's hardware line and comment now include
the GUI test. "What would close the rest" is down to #7 alone (one Standby + Shut Down).
Nothing was posted.

## Not changed, but now stale

`docs/RELEASE_NOTES_v0.3.0-beta.1.md` still lists Display Control's "did it arrive" check
under "not yet verified on the keyboard", and its Part 1 blocker 4 still asks for the
GUI session. Both are now verified except for Standby + Shut Down. It's a small edit if
you want it.

---

# Later on 2026-10-16: issues closed, the 20:58 failure, and two recoveries

Same branch, `docs/session-2026-10-16-evening`, PR #23 (open, **not merged**). Docs only.
Read from `tiga-fresh-test\logs\` plus your account; nothing was run against the keyboard
and nothing was posted on issues.

## Most important: it was the disconnect length, not STOP_STACK

The first recovery at 21:09 was a **25 s** battery disconnect **without key presses**,
and it failed. The second at 21:14 was **about a minute with key presses**, and it worked.
Both have been in the record since 2026-10-15: 10–15 s failed, and every disconnect of
30 s or more has worked (30, 30, 40 s and now about a minute). **25 s without key
presses is the second too-short disconnect.** That explains 21:09 on its own.

STOP_STACK before the disconnect has no mechanism here. The uploader had exited when it
halted at 20:58:42, so no process held a Bluetooth link to the keyboard. The supervisor
and renderer never talk to it. **The recovery advice stays as it is:** at least 30 s,
pressing a few keys, then START_STACK. Running STOP_STACK first is harmless and gives a
fresh supervisor, but nothing shows it's needed.

**Your call:** 25 s failing and 30 s working leaves very little margin. If you want
the instructions to say "about a minute, pressing a few keys" instead of "at least 30 s",
that's a text change in the halt instruction (`tiga_supervisor_common.ps1`, the
uploader's exit message), README, BUILD_AND_RECOVERY, RETRY_RECOVERY and
RELEASE_CHECKLIST, plus the test that asserts it. I haven't changed it.

## Commits on this branch since the last notes section

| Commit | What |
|---|---|
| `08af884` | `docs/TRIAGE_2026-10-16.md` records #4, #5, #6, #8, #9, #10, #14 and #15 as closed on 2026-10-16 (about 21:00, confirmed on GitHub: 19:00Z). #7 is still open, pending one Standby + Shut Down on hardware. The draft gets a Status column and per-issue status lines. `docs/RELEASE_NOTES_v0.3.0-beta.1.md` marks the Display Control check verified on 2026-10-16 except Standby + Shut Down. Three other lines there were also out of date, and I updated them: the issues blocker is done, the timing lines are live on hardware since 20:51, and the timing lines moved from "not verified" to verified. |
| `4c6c8b7` | PROJECT_HISTORY §7.1, **fifth part**: the 20:58 failure and both recoveries. |
| (with these notes) | The fifth part corrected with your durations (25 s / no key presses; about a minute / key presses; my first version said "not recorded" and "40–60 s"). The battery list in §7.1's second part now includes the 25 s failure. |

## The fifth part, in short

- **20:58:37, Spotify auto-advance, on the open 20:52 connection:** code 20 after 4.86 s.
  Block 5 came at a normal 1.58 s, block 3 had taken 802 ms, then there was no PROGRESS
  for block 6. It's the sudden signature, 2.5 minutes after a healthy upload. This is the
  **second failure ever of a later upload on an open connection** (the first was 21:39
  on 10-15). The 802 ms block is within the 0.42–1.08 s slow first block every later
  upload has (fourth part), so it's no warning sign.
- **21:09, after 25 s without key presses, START_STACK with the supervisor still
  running:** code 20 after 3.74 s at block 5. CONTROL1/CONTROL2 ACKs were 20 / 77 ms,
  like any first upload on a new connection (18 / 76 at 20:52, 32 / 73 at 21:14). No log
  shows what those ACKs look like on a keyboard that was *not* reset, so they can't tell
  "reset" from "not reset". RSSI was −66, against −58/−59 on the healthy starts.
- **21:14, after STOP_STACK, about a minute with key presses, START_STACK:** RSSI −58,
  three uploads in 8.95 / 9.93 / 10.06 s, medians 156–157 ms.
- **2.4 GHz interference (the iPad streaming nearby):** recorded as a hypothesis only,
  with a simple A/B test if you want one. Nothing in the logs measures it, and the short
  disconnect already explains 21:09 without it.

## Updated counts

- Later uploads on an open connection: **2 failed** (21:39 on 10-15, 20:58 on 10-16).
  Everything else completed.
- Battery disconnects as recovery, where the length is known: ≥ 30 s **4 of 4 worked**
  (22:49 and 23:06 on 10-15; 18:54 and 21:14 on 10-16). Shorter: **0 of 2** (10–15 s at
  22:44 on 10-15; 25 s without key presses at 21:09 on 10-16). 21:15 on 10-15 also
  worked, but its length isn't known.

---

# Run F (TASKS_F.md), 2026-10-18, about 14:00–14:30: why do uploads fail?

Branch `investigate/upload-failures`, off `origin/main` (`0c57230`). Analysis only:
nothing was sent to the keyboard, the live stack wasn't touched, and nothing was posted
on issues. PR into `main` is open, **not merged**. Read
`docs/investigations/UPLOAD_FAILURES_2026-10-18.md` first.

## Questions only you can answer

1. **13:41: how long between reconnecting (battery + USB-C) and START_STACK?** On
   2026-10-16 20:51 the keyboard advertised before its GATT table was complete (code 11).
   If 13:41 came within seconds of power-on, "not settled yet" is a candidate cause.
   That would explain why a power-off was followed by a failure.
2. **When Windows is shut down, does the USB-C port keep powering the keyboard?** If the
   keyboard ran on battery for the 38 h before 13:20, its state at the first upload
   differs from a normal restart.
3. **At each failure today (13:20, 13:41, 14:03, 14:09), what did the LCD show?** The
   roar or upload animation, the previous image, or something broken? And at 13:42 and
   14:11, before the upload: was it still showing the roar?
4. **The halt policy (decision, not a fact):** would you accept a bounded automatic retry
   after a *sudden* code 20 (wait 60–120 s, at most two retries, then halt as now;
   slowing failures halt at once)? Not implemented; see "What it means for bug A" below.
   I'd suggest collecting 6–8 more halts by hand first (experiment 1).
5. Minor: anything heavy on 2.4 GHz (the iPad streaming) during today's failures?

## Findings in plain words

- **The table:** `docs/investigations/upload_attempts.csv`, one row per upload, built by
  `tools/upload_attempts.py` (stdlib only, with a test). The context it can't get from
  logs (how each stack start came about, battery disconnects) is in
  `stack_events_context.csv`, which you can correct or extend. As read at 14:15:
  **115 uploads, 12 failed, all code 20.** All at pace 0 ms.
- **The first upload on a connection is the risky one:** 9 of 24 failed, against 3 of
  91 later uploads.
- **Your question: independent failures or a stuck keyboard? The data favour
  independence.** Uploads that followed a failure failed **5 of 12**; uploads whose
  previous upload had completed failed **4 of 12**. That's the same rate. Retries with
  no power-off: 2 of 4 worked (13:42, 14:11); retries after a full power-off: 5 of 6.
  Both fit "each first upload is a ~1-in-3 coin flip" (the 5/6 vs 2/4 difference is
  within chance). "Stuck until a power-off" is ruled out by 13:42 and 14:11, and the
  13:42 connection then did five more uploads normally. **The one exception:** the two
  slowing failures at block 11 back to back on 2026-10-15 (22:44, 22:46). That looks
  like a real stuck state, but only for the slowing kind.
- **What it means for bug A:** the halt assumes a retry into a partial session fails or
  breaks the LCD. For sudden failures, 6 logged retries without a proper reset gave 2
  completions and 4 clean failures, with no recorded damage. If independence holds, one
  delayed retry would clear about 6 halts in 10, and two retries about 85 %, with no
  power-off. That's a safety-rule change, so it's your call; it's written up as a
  proposal only.
- **The wait doesn't matter.** Failures after 116 s to 65 min; successes after 49 s and
  ~2.5 min.
- **A power-off isn't a guarantee.** "Every reset of 30 s or more worked" is corrected
  in §7.1. The old "30 s works, shorter doesn't" record (4 of 4, 0 of 2) could have
  happened by chance at a 1-in-3 failure rate.
- **Nothing else separates failures:** not RSSI (−55 failed, −79 worked), time of day,
  image (two CRCs both failed and completed), trigger, renderer activity, CONTROL ACK
  times (so no early warning before CONTROL2 is possible), or the time into the transfer.
  Failure blocks: 3, 5, 5, 6, 7, 11, 11, 27, 37, 41, 41, 52. Only one is at the
  frame-1/frame-2 boundary (block 27, 13:20). 5 of 12 fall in blocks 3–7, which is worth
  watching, but I picked that window after looking.
- **Pace 0 ms has never been varied.** The archive (PDF pp. 297–299) planned 2 → 1 → 0 ms,
  "each separately". The only recorded result is "0 ms klappt ohne Probleme", 7.68 s,
  so 0 ms rests on one upload. A lost write fits the sudden signature exactly; untested.
- **Two blind spots in the uploader:** notifications other than PROGRESS/COMPLETE are
  discarded without logging, and the BLE scan keeps running during uploads.
- **Recorded in §7.1 (sixth part):** resets are full power-offs including a USB-C
  unplug; typing goes over USB-C; #7 verified (2026-10-16 23:07: upload completed 23:07:13,
  Windows shut down 23:07:27) and closed; today's timeline 13:20–14:11. Corrections where
  today contradicts earlier text. `RETRY_RECOVERY.md`'s "LCD broken after recovery" row
  now also names "a new failure". **README and the halt instruction are unchanged:** the
  evidence doesn't yet clearly support new advice.

## Proposed experiments, in order

1. **Retry-first recovery on the halts that happen anyway.** Sudden code 20: wait ~2 min,
   START_STACK without a power-off, once more if needed, power off only after two failed
   retries. Slowing: power off as now. 6–8 halts. Costs nothing extra, and decides
   question 4.
2. **Notification logging** (code proposal A, then normal use): see what, if anything,
   the keyboard says when it stalls. No active time.
3. **Pace 2 ms vs 0 ms on first uploads:** 10–12 deliberate STOP/START restarts at 2 ms.
   0 failures would be strong evidence (chance ~1 %). Costs about 2.5 s per upload and a
   power-off per failure.
4. **First upload after a cold boot:** just note it each time; n = 2 so far (1 failed).

## Not done / blocked

- The fallback's logs (`tiga_ble_re_tools\logs`) can't be tabulated: no timestamps, and
  overwritten per run. Summarised in `upload_attempts.md` only.

---

# Three changes from run F, 2026-10-18 afternoon

Three branches off `origin/main` (`212bda3`, after #24), each with its own PR into
`main`, **none merged**. Nothing was sent to the keyboard, the live stack wasn't touched,
and no issue was commented on. Only a dry run of the uploader (no Bluetooth) was run.
This notes section is on the third branch only, so the three PRs don't conflict over
this file.

## Your answers to run F, recorded

In PROJECT_HISTORY §7.1 sixth part (on `feature/sudden-retry`):

- The delay between reconnecting and START_STACK at 13:41 is not known. So "not settled
  after power-on" stays open; noting that delay at future power-offs would settle it.
- The LCD showed the roar at every failure. So at each stall the keyboard was still in
  the transfer it had accepted, which fits both the uploader's view and the successful
  plain retries at 13:42 and 14:11.

## PR #25: `feature/log-notifications` (logging only)

- Every notification that isn't PROGRESS or COMPLETE gets a `NOTIFY` line in `ble.log`:
  characteristic, hex value, what the uploader was waiting for (including `between
  uploads`), and for how long.
- A normal upload adds four lines (the CONTROL1/CONTROL2 ACK pairs, named as such).
- Between uploads the uploader now reads the notification stream instead of sleeping.
  The timing is the same; before, those notifications were thrown away unseen by the
  next upload's drain.
- **What to look for at the next stall:** any `NOTIFY … while=waiting for PROGRESS of
  block N/53` just before the `UPLOAD FAILED`. None means the keyboard went silent.
- Tests: cargo 44, all others pass.

## PR #26: `feature/stop-scan-after-connect` (behaviour change)

- The BLE scan stops as soon as a device qualifies (connected, media service present),
  with a `Scan stopped` line after `SELECTED`.
- Until then it runs as before, including the rescan after rejected candidates.
- Nothing after selection ever needs a scan: every failure ends the process and the
  next run scans afresh. If stopping fails, it's logged and the upload runs with the
  scan on, as before.
- **Hardware check:** `Scan stopped` after `SELECTED`, then normal uploads. If the very
  first block never gets PROGRESS after this change, revert it.
- Tests: cargo 42, all others pass.

## PR #28: `feature/sudden-retry` (the bug A policy change you approved)

- **Uploader:** on a code 20 it logs `SIGNATURE sudden|slowing|unclassified` and writes
  `signature=` into the exit record.
  - Sudden: a block timeout with a normal pace (median ≤ 500 ms, at most one block over
    1 s). Today's 14:09, with one 1860 ms block, counts as sudden.
  - Slowing: median over 500 ms or two or more slow blocks.
  - Unclassified: anything else (link lost, no block ACKed, an earlier run's marker).
- **Supervisor:** on a sudden code 20 it waits 120 s, removes the session marker, and
  starts the uploader again, up to 3 times per incident.
  - Each step goes into `startup_supervisor.log`: scheduled (with the time), started,
    and SUCCEEDED or FAILED.
  - A delivered frame ends the incident and resets the count.
  - A code 10/11 during a retry uses the normal reconnect backoff and doesn't use up a
    retry.
  - After 3 failed retries it halts exactly as now (same class and instruction); the
    halt file adds `auto_retries` and `signature`.
  - Slowing, unclassified, code 21 and a killed uploader still halt at once.
  - A retry pending when the supervisor stops isn't resumed.
- **STATUS_STACK** (dev and packaged): `RETRYING - sudden upload failure; automatic
  retry 1 of 3 at HH:MM:SS` while one is waiting, plus an `Auto-retry` line with the
  last events from `logs\uploader_retry.json`. A halt after retries shows the signature
  and the retries used.
- **Docs:**
  - RETRY_RECOVERY: the rule's exception, with PR #24's numbers; the exit-code table;
    the signature definitions; the policy and STATUS.
  - README: the known-issues bullet and the "BLE upload was interrupted" section.
  - PROJECT_HISTORY: a note under bug A.
- **Tests:** PowerShell retry test, with new decision-table rows and 8 new loop scenarios
  (three retries then halt, a delivering retry, slowing/unclassified/21 halts, a
  delivery resetting the count, code 10 during a retry, a leftover waiting retry) and
  STATUS checks. Cargo 41 with the signature cases from the record; logging test and
  Python pass.

**Not changed, on purpose:**

- The uploader's own exit message for code 20 still says to disconnect the battery
  before retrying. It's printed for every sudden failure even when the supervisor will
  retry. The halt instruction is unchanged as you asked, and the message is asserted by
  a test.
- 120 s and 3 are constants at the top of the policy block in
  `tiga_supervisor_common.ps1` (`$BleSuddenRetryDelay`, `$BleSuddenRetryMax`).

## Merge order and conflicts

- #25, #26 and #28 all touch `tiga_stream_watch_flow.rs` in different places. Whichever
  merges second or third may need a trivial rebase. #25 and #28 also both add to
  `RETRY_RECOVERY.md`, in different sections.
- #27 (relicensing, `docs/gpl-and-wired-setup`, opened during this run and not mine)
  also edits `README.md`. If it merges first, #28's README hunks may need a rebase.
- My suggested order: #25 (logging), then #28 (retry), then #26 on its own afterwards.
  #26 changes the radio's behaviour, so it's best judged in a period without other
  changes.

## Deploying (yours, not done)

The live install only gets any of this after merge, `git pull` in `tiga-fresh-test`, a
rebuild of the uploader, and a stack restart. Expect the ~25 s first-start delay after a
rebuild (antivirus).

---

# Shorter front page, 2026-10-18 evening

Branch `docs/shorter-front-page`, off `origin/main` (`5718dde`, after #25–#28). Docs and
file moves only, no behaviour change. PR into `main` is open, **not merged**.

## What moved where

- **README** (498 → 136 lines) keeps download, previews, features, quick start, Display
  Control, known limitations, two troubleshooting cases (stuck on the roar; interrupted
  upload), a "For developers" list of links, contributing (one line), disclaimer and
  license.
  - The previews are now a three-column table, and the download and quick-start lists
    are written inline. The wording is unchanged.
- **docs/ARCHITECTURE.md** now holds How it works, media-session behaviour (including the
  stale-session gate), BLE uploader, privacy, repository layout and reverse-engineering.
  - The README's diagram items (renderer fields, completion acknowledgement) were merged
    into the existing diagram.
  - The layout now also lists `tests/` and `tools/`.
- **docs/BUILD_AND_RECOVERY.md** now holds build from source (requirements, clone, build,
  manual BLE test, installing-from-source troubleshooting) and a Troubleshooting section:
  logs, keyboard unreachable, turning it off on purpose, missing duration, existing
  manual installation.
- **.github/CONTRIBUTING.md** got the README's "useful contribution areas" list.
- **Checked by script:** every line of the old README is still somewhere in the new
  README or these three files. The only differences are the compacted lists, the merged
  diagram, and links rewritten for their new location.

**One content fix, not just a move:** BUILD_AND_RECOVERY's "BLE recovery" paragraph
still said every interrupted upload halts. Since #28 that isn't true, so it now
describes the automatic retry after a sudden failure. It also lists
`uploader_retry.json` among the runtime files.

## Files moved

| From | To |
|---|---|
| `Now-Playing.jpeg`, `Standby.jpeg`, `Intermission-Screen.jpeg` | `docs/images/` |
| `NOTES_FOR_MICHI.md`, `RELEASE_CHECKLIST.md` | `docs/` |
| `CONTRIBUTING.md`, `SECURITY.md` | `.github/` (GitHub still finds them there) |

- **`PUBLISH_TO_GITHUB.md` deleted as obsolete.** It was a one-time guide for the first
  push (2026-09-09, "Initial public release").
  - Everything it describes is done: the repository exists, v0.1.0 and v0.2.0-beta.1 are
    released, and the description is set word for word.
  - It recommends the name `tiga-windows-media-display` and a `YOUR_USERNAME` remote,
    neither of which matches the real repository (`tiga-zoom75-windows-media-display`).
  - Its topic list is a subset of the topics now set on GitHub, except `rgb565` and
    `reverse-engineering`. Add those by hand if you want them.
  - It's still in git history.
- **No script, `.py`, `.cpp`, `.ps1` or anything the task, installer, tests or supervisor
  use was moved.** Nothing in the scripts, workflows, installer, tests or `tools/`
  referenced the moved files; the only CI copy is `LICENSE`, which stays.
- **Historical mentions** of the old file names inside earlier notes sections were left
  as they were: they describe what was true at the time.

## Outside the repository

- `C:\Users\Regal\source\CLAUDE.md`: a new paragraph under "Read this first" pointing
  to `tiga-dev/docs/NOTES_FOR_MICHI.md` (plus where the checklist, CONTRIBUTING and
  SECURITY went).
- `TASKS_C.md` to `TASKS_F.md`: every `tiga-dev\NOTES_FOR_MICHI.md` is now
  `tiga-dev\docs\NOTES_FOR_MICHI.md`.
- The Claude memory note for the old `tiga-windows-media-display` folder named the same
  path; updated too.

## Note for the live install

`tiga-fresh-test` still has the files at the root until you pull. Pulling moves them;
nothing that runs depends on them.

# Hardware test of PR #20 (clean stop), 2026-10-22, 20:30–20:49

Run by Michi with Claude guiding. The task was pointed at `tiga-dev` (`feature/clean-stop`
at `10f1e69`, origin/main unmoved since the rebase), then back at `tiga-fresh-test`
(`main`, `5c1237a`). Claude ran no START/STOP_STACK or install script.

## Stop path: works as designed in all three cases

| Stop | Situation | STOP_STACK line | ble.log |
|---|---|---|---|
| 1 | idle | `exited on request after 0,3 s` | summary, `STOP requested`, `Disconnected cleanly.`, `EXIT code=0` |
| 2 | ~3 s into an upload (f0003, after a skip) | `exited on request after 6,9 s` | `UPLOAD COMPLETE` 9.78 s, then the same three lines |
| 3 | during the 120 s retry wait | `no BLE uploader was running.` + `automatic retry 1 of 3 was waiting; cancelled.` | nothing (no uploader); `uploader_retry.json` → `cancelled` |
| 4 | idle | `exited on request after 0,3 s` | as stop 1; supervisor also logged `stopped on request … not restarting` |

No request file left after any stop. The session marker stayed after stop 3, as designed.
In stop 1 the supervisor logged nothing of its own: STOP_STACK stopped the task within
0.3 s, probably before the supervisor's next poll. Stop 4 shows the classification works.

## First uploads: the hypothesis is not supported (yet)

| Start | Before it | Visible after | First upload |
|---|---|---:|---|
| A | battery reset | 1 s | completed 8.76 s |
| B | clean idle stop (1) | 2 s | completed 8.55 s |
| C | clean stop during upload (2) | 2 s | **failed** block 5, 3.84 s, code 20 sudden |
| D | battery reset (~30–40 s) | 0 s | **failed** block 7 (block 6 `SLOW` 2385 ms), 6.31 s, code 20 sudden; automatic retry 1 completed 8.73 s, no power-off |
| E | clean idle stop (4), `main` stack | 2 s | completed 8.55 s |

- No invisible periods and no code-10 exits at all (baseline: 4 of 9, 19–67 s).
- Start D failed **after a battery reset**, which the baseline never saw (0 of 4). So
  a failed first upload is not only a stale-link effect; C should not be blamed on the
  clean stop with confidence either.
- #28 worked on hardware: a sudden failure was recovered by retry 1 without a power-off.
- Too few starts (2 clean-stop restarts, both via `tiga-dev`) to compare with the
  baseline; the plan asks for ≥ 10.

## Open for Michi

- Merge PR #20 on the stop-path result alone? The design says the change is worth keeping
  for never cutting an upload in half, whatever the first-upload numbers show.
- The ≥ 10-restart comparison is still to do, and D (failure after a reset) is new data
  for PROJECT_HISTORY §7.1.

# 2026-10-22 evening: runtime lab sessions 1–3, and the live-install log (PR #31)

Claude ran nothing against the keyboard. Every read session was run by Michi with
`capture.py --live`; Claude only analysed the dump files. No START/STOP_STACK, no install
script, no scheduled-task change. Details of the lab work are in
`tiga-runtime-lab/docs/LAB_NOTES.md` (private repo `MichiG21/tiga-runtime-lab`, branch
`experiment/tiga-runtime-smoke`).

## Runtime lab: what the read sessions established

| Session | Time | Read | Result |
|---|---|---|---|
| 1 | 20:58:13–20:58:24 | tick window `0x110014C0`, GUI veneers | **F2 is not an instruction** (middle of a `push.w`). **F4 is `ldr r6, [pc, #0x3f8]`**, a PC-relative literal load. Upstream TweakLoader would refuse to hook there |
| 2, 2b | 21:14, 21:18 | — | refused before connecting: "Target name mismatch" (WinRT's first advertisement has no name). Nothing sent. Fixed in `capture.py` (identity by name **or** the media service UUID, 15 s) |
| 2c | 21:22:55–21:23:15 | literal pool, rest of the function, r6 struct | **The F4 literal is `0x110038D4`**, as §6 says. The hooked function is a message dispatcher: a pending flag at `0x110038D5` (0 = idle), commands from the main MCU at `0x11005D2C` |
| 3 | 22:23:56–22:24:08 | function tail, vector table, veneer table | The function ends at `0x110019DA`; **no branch into the hook bytes** anywhere in it. No veneer points at it. The UART handler is at `0x1100146E` in RAM (not yet read) |

All dumps were identical across three passes, with no memory writes. You saw no LCD
reaction in sessions 1 and 3.

**What an F4 hook needs:** the trampoline must set r6 = `0x110038D4` itself, replay
`ldrb r0, [r6, #1]` and return to `0x110014F8`. In PSRAM (next to the payload at
`0x220E0000`) it would need an 8- or 10-byte patch instead of 4, because `b.w` reaches only
±16 MB.

**Session 3 is only partly analysed.** Claude's responses on it were stopped twice by an
automated safety check on Anthropic's side, although the work is read-only analysis of your
own keyboard's firmware. At your request only the established facts were recorded. The
trampoline comparison and recommendation were **not** done.

## Live install (PR #31, not merged)

PR #31 adds §7.1's seventh part from the live logs: clean stops at 21:11 and 22:23; the
21:23 sudden failure, recovered by the automatic retry at 21:25 without a power-off; the
22:24 slowing failure, which halted, and the plain START_STACK at 22:27 that completed
in 8.53 s. That is the first slowing failure recovered without a reset. RETRY_RECOVERY.md now
counts four slowing failures.

**Correction made after opening the PR:** it first called 21:11 and 21:25 the *first* clean
stops and the *first* automatic retry. The PR #20 test at 20:30–20:49 (above) came first, on
`tiga-dev`. The PR now says "on the live install", points to that test, and the retry tally
counts start D: retries without a power-off **5 of 7**; after a sudden failure 4 of 5.

**Observation only:** both evening failures were first uploads with the stack started 7–8 s
after a RAM read session ended (after session 1: ~29 s, first upload ~68 s, no failure).
Two of three, no control, at the normal 1-in-3 first-upload failure rate.

## Open for Michi

- **Read sessions:** wait a minute or more before START_STACK and note the gap. That tests
  the observation above cheaply.
- **Next lab read, if wanted:** the UART handler, `0x11001460–0x110014BF`, 18 requests over
  three passes. It needs a new `capture.py` range.
- **Trampoline placement:** SRAM at `0x11008A60` (needs a free-RAM argument), PSRAM next to
  the payload (needs an 8/10-byte patch that can't be written atomically), or a small SRAM
  stub that jumps on to PSRAM. Undecided; see LAB_NOTES 2026-10-22 "night, final".
- **PR #31:** merged (2026-10-22 20:41 UTC). This section reached `main` later, via PR #32.
- **v0.3.0-beta.1 release task:** done later the same evening; see the next section.

# v0.3.0-beta.1 release preparation, 2026-10-22 late evening (PR #32)

Branch `release/v0.3.0-beta.1` off `origin/main` (`8b2011a`, after #31). Docs only. Claude
ran no install, START_STACK, STOP_STACK or scheduled-task command.

## What the PR does

- **First commit, cherry-picked:** `5ec7430` → `e2da0a7`. It was pushed to
  `docs/hardware-log-2026-10-22` after #31 was merged, so it never reached `main`. It holds
  the §7.1 "first retry" corrections and the previous section of these notes. It applied
  without conflicts.
- **`docs/RELEASE_NOTES_v0.3.0-beta.1.md`:** finished blockers removed (version name,
  CHANGELOG, #7, issues, #20); #25–#31 added to the release text; #7 under "verified"; the
  failure-rate text uses the 2026-10-18 figures.
- **`CHANGELOG.md`:** a full `v0.3.0-beta.1 (not yet tagged)` section, including the
  license change MIT → GPL-2.0-only.
- **`docs/RELEASE_CHECKLIST.md`:** title and Phase 7 tag `v0.3.0-beta.1`; Phase 7's
  "close issues" items replaced (all closed already); Phase 8 rewritten against the
  installer script.

**Dates for #7, to avoid confusion:** verified on hardware 2026-10-16 23:07, closed on
GitHub 2026-10-18 11:31 UTC.

## Installer facts found while checking Phase 8

1. **The installer takes over your scheduled task.** It registers "TIGA Windows Media
   Display", the same name the dev install uses, pointing at
   `%LOCALAPPDATA%\Programs\TIGA Windows Media Display`, and starts it.
2. **Its STOP_STACK kills the dev stack too, uncleanly.** The packaged `STOP_STACK.ps1`
   matches processes by name. Its stop request goes to the installed folder, which the
   `tiga-fresh-test` uploader doesn't watch, so a running dev uploader is force-killed after
   30 s, possibly mid-upload. **Stop the dev stack with its own STOP_STACK first.**
3. **Files are copied before anything is stopped** (`CloseApplications=no`). Installing over
   a running v0.2.0-beta.1 may hit locked executables. That's what the test is for.
4. **Uninstall unregisters the task** and leaves `logs\` behind.
5. **v0.2.0-beta.1 has no bug A fix.** If one of its uploads fails, it retries every 8 s
   against a keyboard stuck mid-transfer, the pattern that broke the LCD before. Keep its
   part of the test short.

## Phase 8 test steps for v0.3.0-beta.1

All commands are for PowerShell. Each install starts the stack and uploads to the keyboard.

**0. Before you start**

- The keyboard is on, wired over USB-C, and showing an image; no halt is pending
  (`C:\Users\Regal\source\tiga-fresh-test\STATUS_STACK.ps1` shows no `HALTED`).
- Pick a track with a **Japanese title** for step 6 (e.g. `トーキョーレギー`, used on
  2026-10-15).

**1. Stop the live stack cleanly**

```
powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\Regal\source\tiga-fresh-test\STOP_STACK.ps1
```

Expect `exited on request after … s` or `no BLE uploader was running`. If it reports a
force-kill, check the keyboard before you start the test.

**2. Download both installers**

```
$d = "$env:USERPROFILE\Downloads\tiga-phase8"; New-Item -ItemType Directory -Force $d | Out-Null
gh release download v0.2.0-beta.1 -R MichiG21/tiga-zoom75-windows-media-display -p "TIGA-Windows-Media-Display-Setup-v0.2.0-beta.1.exe" -D $d
$run = gh run list -R MichiG21/tiga-zoom75-windows-media-display --branch release/v0.3.0-beta.1 --workflow installer.yml --status success --limit 1 --json databaseId -q ".[0].databaseId"
gh run download $run -R MichiG21/tiga-zoom75-windows-media-display -n TIGA-Windows-Media-Display-Setup-vpr-32 -D $d
Get-ChildItem $d
```

Expect `TIGA-Windows-Media-Display-Setup-v0.2.0-beta.1.exe` (13,210,661 bytes) and
`TIGA-Windows-Media-Display-Setup-vpr-32.exe`. `$run` is the newest successful installer
build of the PR; every push builds a new one. Without `gh`: PR #32 → Checks → *Build
Windows installer* → Artifacts.

Check that no packaged version is installed yet:
`Test-Path "$env:LOCALAPPDATA\Programs\TIGA Windows Media Display"` → `False`. If `True`,
note it: that is an earlier install, and step 3 becomes an upgrade too.

**3. Install v0.2.0-beta.1**

1. Run `TIGA-Windows-Media-Display-Setup-v0.2.0-beta.1.exe`. SmartScreen: *More info → Run
   anyway*. Leave "Open TIGA Display Control" unticked; *Finish*.
2. Check the task now points at the installed copy:
   `(Get-ScheduledTask -TaskName "TIGA Windows Media Display").Actions.Arguments` contains
   `AppData\Local\Programs\TIGA Windows Media Display`.
3. Wait for the LCD to show the current track (about 20–30 s; play something if nothing is
   playing). Then **don't change tracks** and go straight to step 4.
4. **If v0.2's upload doesn't complete** (LCD stuck on the roar, or nothing after a minute):
   Start menu → *Stop Display* at once, battery disconnect ≥ 30 s, reconnect, then continue
   with step 4. Note it; it's a v0.2 behaviour, not this release's.

**4. Install the new build over it, while v0.2 is running**

1. Run `TIGA-Windows-Media-Display-Setup-vpr-32.exe`, same SmartScreen step.
2. **Watch for a "file in use" / "unable to write" / "DeleteFile failed" message.** If one
   appears, write down the file name and take a screenshot. Click *Retry* once; if it
   comes back, run v0.2's Start menu → *Stop Display*, then *Retry*. This is the untested
   point the checklist asks about.
3. The installer's status text shows "Stopping any previous TIGA display session...",
   "Installing automatic startup task...", "Starting TIGA Windows Media Display...".
   The stop may take up to 30 s.
4. Keep the setup log:
   `Get-ChildItem $env:TEMP -Filter "Setup Log*.txt" | Sort-Object LastWriteTime | Select-Object -Last 1`
   and copy it to `$d`.
5. Settings → Apps → *TIGA Windows Media Display* shows version `pr-32`. The task arguments
   (step 3.2) still point at the installed folder.

**5. An upload works, and Status**

1. Change track. Within about 20 s the LCD shows it.
2. `$l = "$env:LOCALAPPDATA\Programs\TIGA Windows Media Display\logs"`, then
   `Get-Content "$l\ble.log" -Tail 40`. Expect `Scan stopped: connected to the keyboard`,
   `UPLOAD COMPLETE frame=… in ~8–10s` and a `TIMING` line.
   - If the first upload fails with `SIGNATURE sudden`, that's the known first-upload
     failure. The packaged build should retry after 120 s by itself, and *Status* shows
     `RETRYING`. Wait for it, and note it: it's the first packaged run of #28.
3. Start menu → *Status*. Expect `OK - the keyboard shows the newest rendered frame` and
   `Last delivered: <frame id> <time> in <s>s`, and no `HALTED`.

**6. A Japanese title**

1. Play the Japanese-titled track. The LCD shows the characters, not empty boxes.
2. `Select-String -Path "$l\renderer.log" -Pattern "FONT" | Select-Object -Last 5`.
   Expect `FONT fallbacks: …` listing Windows fonts, and **no** `FONT no font has U+…` for
   the title's characters.

**7. Optional: the packaged clean stop**

Start menu → *Stop Display*, while no upload is running. Expect it to finish quickly; then
*Start Display*, and the current track comes back. That's #20 on the packaged layout.

**8. Uninstall**

1. Copy the logs first: `Copy-Item -Recurse $l "$d\logs-packaged"`.
2. Settings → Apps → *TIGA Windows Media Display* → Uninstall.
3. Check the task is gone: `Get-ScheduledTask -TaskName "TIGA Windows Media Display"` gives
   an error ("No MSFT_ScheduledTask objects found"). The install folder may remain with
   `logs\` in it; delete it by hand once the logs are saved.

**9. Point the task back at `tiga-fresh-test`**

```
powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\Regal\source\tiga-fresh-test\install_tiga_startup.ps1
(Get-ScheduledTask -TaskName "TIGA Windows Media Display").Actions.Arguments
```

The arguments must contain `C:\Users\Regal\source\tiga-fresh-test`. The script registers
the task but doesn't start it. Start the stack:

```
powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\Regal\source\tiga-fresh-test\START_STACK.ps1
```

and check `C:\Users\Regal\source\tiga-fresh-test\logs\ble.log` for `UPLOAD COMPLETE`.

**10. Record the result**

For the release notes: whether step 4 hit a locked file, and whether the packaged upload,
Status and Japanese title worked, plus anything odd. Then the two *(update before
publishing)* notes in Part 2 can be replaced, and Phase 7 can go ahead.

## Open for Michi

- Review and merge PR #32 after Phase 8 (or before; it's docs only, and the notes'
  placeholders say what's still pending).
- Whether the Phase 4 killed-uploader test must pass before the release (Part 1, item 2).

# Phase 8 on hardware, 2026-10-25: three fixes (PR #32)

Michi ran Phase 8 in the morning with build `vpr-32` (installer CI run 39853101947, at
`4d06fdf`); logs in `C:\Users\Regal\Downloads\tiga-phase8\`. Claude read them, made three
fixes on `release/v0.3.0-beta.1`, one commit each, and recorded the results. Claude
installed nothing, sent nothing to the keyboard, and ran no START/STOP_STACK or
scheduled-task command. The only uploader run was a dry run (no Bluetooth) on a copy of
the template in the scratchpad.

## Commits

| Commit | What |
|---|---|
| `e0add1b` | Installer: stop the installed version before copying files (the blocker) |
| `319b426` | Uploader: hold a stream with no `.frame` sidecar until the renderer writes one |
| `7d086bc` | Installer: keep the Start/Stop/Status shortcut windows open (`-NoExit`) |
| docs commit | Phase 8 results in the release notes and the checklist; this section |

Tests after the three code commits: Python 73 OK, Rust 54 passed (51 + 3 new), PowerShell logging 19 and
retry 138 checks passed. The installer workflow on `7d086bc` compiled the new `[Code]`
section with Inno Setup 6.7.1. Its only warning, about RunOnceId in `[UninstallRun]`, was
there before. **The Pascal code has never run**, because CI only compiles it. The retest
below is its first run.

## What the logs show, in order (local time)

- **09:26** v0.2.0-beta.1 installed (`%TEMP%\Setup Log 2026-10-25 #001.txt`, not in the
  Downloads folder). **09:27:29–09:31:00:** its renderer exited 16 times, each time
  restarted after 8 s; v0.2 completed 14 uploads (8.3–9.3 s) in those minutes. Michi:
  the crash loop came from a Japanese title (bug G, fixed in v0.3).
- **The "DeleteFile failed; code 5" error is not in any of the three setup logs.** #002
  (09:30:20–09:30:56) ends with "Exit Setup? Yes" before "Starting the installation
  process", and #003 installed all 78 entries at 09:33:00, after v0.2 had been stopped
  (the `[Run]` STOP_STACK at 09:33:02 found "no BLE uploader was running"). The cause
  doesn't depend on the log: `[Files]` always ran before the `[Run]` stop.
- **09:33:06** v0.3 stack started. The renderer's first frame came at **09:33:42 (36 s)**.
  The uploader had connected at 09:33:23 and sent the template meanwhile:
  `UPLOAD START frame=unknown(no-sidecar) crc32=57e8e0cf` (the tracked template's CRC,
  checked), **64.08 s**, every block `SLOW` (median 1140 ms, max 2407 ms), but complete.
- 09:34:28 the standby frame failed with code 10 (`control ACK pair timed out`); restarted
  after 8 s, it completed at 09:34:49 in 8.83 s. Then `OH! TENGO SUERTE` (10.72 s) and
  `トーキョーレギー` (9.62 s, glyphs; `FONT fallbacks: YuGothB.ttc, msyhbd.ttc,
  msjhbd.ttc, malgunbd.ttf, seguisym.ttf; not installed: meiryo.ttc`).
- **09:41:21** *Stop Display*: `exited on request after 0,3 s`, `Disconnected cleanly`.
- **09:44:34** *Start Display*. First upload: slowing failure at block 9 → **HALTED**
  (correct: slowing is never retried).
- **09:50:36** halt cleared (Start Display). First upload: **sudden** failure at block 5 →
  retry 1 of 3 at **09:52:44** → **succeeded**, delivered 09:52:55 (8.64 s). The first
  automatic retry on the packaged build.
- Uninstall, then the task re-pointed at `tiga-fresh-test`; that stack started at 09:57
  and is completing uploads (checked in its `ble.log`).

## Fix 1: the upgrade lock (`e0add1b`)

`PrepareToInstall` runs before `[Files]`. If `{app}\STOP_STACK.ps1` exists, it runs it
hidden and waits (a progress page says "Stopping TIGA Windows Media Display"). That is the
**installed** version's script, so v0.2 is stopped with v0.2's force-kill, and later
versions with their clean stop. Then it waits up to 15 s until no
`tiga_stream_watch_flow.exe`, `tiga_windows_nowplaying.exe` or
`tiga_windows_media_snapshot.exe` runs **from the install folder** (WMI,
`ExecutablePath`). `Stop-Process` returns before a killed process is gone, and v0.2's
script doesn't wait. If one is still running after 15 s, Setup stops on the Preparing page
("…is still running… Nothing has been changed… Stop Display, then run Setup again")
instead of failing halfway. The `[Run]` steps are unchanged. Everything is in the setup
log, before "Starting the installation process".

Not covered, by design or for later:

- **Display Control** isn't part of the stack and STOP_STACK leaves it running. If its
  window is open during an upgrade, `tiga_display_control.exe` is still locked and Inno's
  own Retry dialog comes up: close the window, then click Retry.
- **v0.2 is still stopped by force.** If it's mid-upload when Setup runs, the keyboard may
  need a battery disconnect. That was true of the old `[Run]` step too. The release text
  now says to run Setup while nothing is being sent, and not while a Japanese title plays
  on v0.2.
- If Setup is cancelled after the stop, the old version stays stopped until *Start
  Display* or the next sign-in.

## Fix 2: no template upload at startup (`319b426`)

In watch mode the uploader now holds a stream that has **no** `<stream>.frame` at all.
It logs one `UPLOAD HELD frame=unknown(no-sidecar) crc32=…: no ….frame yet, …` line,
then looks again every 250 ms, and the stop request is still checked on every look. When
the sidecar appears it logs `RENDERER sidecar appeared after N s; uploading its frame`
and takes one more look before uploading, because the renderer writes the sidecar just
before it replaces the stream. A sidecar that exists but doesn't match is uploaded exactly
as before (`unknown(sidecar-is-…)`). `--allow-no-sidecar` uploads the template anyway
(manual tests; `docs/BUILD_AND_RECOVERY.md` says so), and a dry run prints a note when a
live run would hold. The first upload of the connection is now the renderer's frame, so
the first-upload warning applies to it.

**Not covered (your call): reinstall or upgrade over leftover files.** The installer
copies the template over `session_01_bulk_stream.bin` on every install but leaves the old
`.frame`. The CRCs then don't match, the template goes out once through the
mismatched-sidecar path you asked to keep, and the hold doesn't apply. That will happen on
every upgrade from v0.3.0-beta.1 onward, and on a reinstall into a folder that still has
leftovers. Options:

- (a) an `[InstallDelete]` entry for `{app}\app\_internal\extracted\session_01_bulk_stream.bin.frame`,
  so the hold covers it (my suggestion: a few lines, and it reuses this fix);
- (b) install the stream with `onlyifdoesntexist`, so the last frame and its sidecar
  survive an upgrade (the uploader then resends the last frame);
- (c) leave it.

## Fix 3: shortcut windows (`7d086bc`)

*Start Display*, *Stop Display* and *Status* run `powershell.exe -NoExit …`. I checked
that `-NoExit -File` runs a script and then stays at the prompt in Windows PowerShell 5.1.
None of the three scripts calls `exit`, and no process match in STATUS_STACK or STOP_STACK
matches the shortcut's own command line. An install creates the shortcuts again, so an
upgrade gets the new ones.

## Retest for fix 1: which Phase 8 steps to repeat

From "Phase 8 test steps for v0.3.0-beta.1" above: **0 (prep), 1, 2, 3, 4, 5.1–5.3, 8,
9**. Skip 6 and 7: they passed, and the fixes don't touch them. Fixes 2 and 3 are checked
along the way at no extra cost. Changes against the original steps:

- **Before step 1: delete the leftover install folder.** Your logs are already in
  `Downloads\tiga-phase8\logs-packaged`. Otherwise its old `.frame` makes the first upload
  the template again (see fix 2, "Not covered") and step 3 isn't a clean v0.2 install:
  `Remove-Item -Recurse -Force "$env:LOCALAPPDATA\Programs\TIGA Windows Media Display"`
- **Step 2: the new build only, into a new folder.** The artifact still has the old name
  (`…-vpr-32.exe`), so keep the two apart. The v0.2 setup is already in
  `Downloads\tiga-phase8`. Any successful installer run at `7d086bc` or later has all three
  fixes:
  `$d = "$env:USERPROFILE\Downloads\tiga-phase8-retest"` and then step 2's
  `New-Item`, `$run = …` and `gh run download …` lines with this `$d`.
- **Step 3: play a Latin-titled track, not a Japanese one** (v0.2 crash-loops and uploads
  every few seconds). Before step 4, wait until the LCD shows it and nothing has changed
  for 15 s, so v0.2's force-kill doesn't land mid-upload.
- **Step 4 is the check.** Expect **no** "DeleteFile failed" or "file in use" message. A
  page "Stopping TIGA Windows Media Display" may show briefly. Then, in the newest setup log:
  `$log = (Get-ChildItem $env:TEMP -Filter "Setup Log*.txt" | Sort-Object LastWriteTime | Select-Object -Last 1).FullName`
  and
  `Select-String -Path $log -Pattern "Stopping the installed version|STOP_STACK.ps1 exit code|TIGA stack process|Still running|Could not|Starting the installation process"`.
  Expect `Stopping the installed version before copying files: …`,
  `STOP_STACK.ps1 exit code: 0` and `No TIGA stack process runs from … (waited N ms)`,
  **all before** `Starting the installation process`. If Setup instead stops with "…is
  still running…", screenshot it and keep the log: that is the new 15 s wait giving up.
- **Step 5 adds two checks.** In `ble.log`, the first `UPLOAD START` is **not**
  `frame=unknown(no-sidecar)`. You may see `UPLOAD HELD … crc32=57e8e0cf` and
  `RENDERER sidecar appeared after …` (only if the renderer is slower than the connect;
  36 s last time). The *Status* window stays open (fix 3).
- **Steps 8 and 9 as before.** Copy the logs to `$d\logs-packaged` first. Afterwards
  delete the install folder by hand.

## Also in the logs (observations, nothing changed)

- **All three starts had a failed upload early:** 09:33, code 10 on the upload right after
  the slow template; 09:44, slowing, halted; 09:50, sudden, retried OK. Plus a 64 s
  template upload with every block slow that still **completed**: the slowing pattern
  without a timeout. Candidates for an eighth part of PROJECT_HISTORY §7.1, which I didn't
  edit.
- **Between the 09:44 halt and the 09:50:36 Start Display**, the logs can't show whether
  the battery was disconnected. Worth noting for the retry tally.

## Open for Michi

- **The fix 1 retest** above; then delete the last *(Before publishing)* note in Part 2 of
  the release notes ("Installing") and go on to Phase 7.
- **Fix 2's leftover-sidecar case:** (a), (b) or (c) above.
- PR #32 is no longer docs only: it carries the installer and uploader fixes. Not merged.
