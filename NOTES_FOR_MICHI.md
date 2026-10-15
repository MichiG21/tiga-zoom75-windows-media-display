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
