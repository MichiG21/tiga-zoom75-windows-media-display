# Uploader failure classes and recovery (issue #4)

**Status:** implemented on `fix/a-retry-recovery` and merged. The interrupted-transfer
halt (step 2's outcome, from a real PROGRESS timeout rather than a kill) was seen on
hardware on 2026-10-15; the rest of the plan below has not been run. The slow retry for
10 and 11 (below) was added on `fix/device-selection` and is tested offline only. The
**automatic retry after a sudden code 20** (below) was added on `feature/sudden-retry`,
2026-10-18, with Michi's approval; tested offline only.

## The rule

A failure is safe to retry only if no media session was opened on the keyboard. The
boundary is **CONTROL2**: it puts the stock firmware into its upload state (the roar
animation, PROJECT_HISTORY §4.1). Before CONTROL2 the keyboard is untouched. From the
moment CONTROL2 is *attempted*, a failure may leave a partial session, and another
upload on top of it is what led to the broken-LCD states recorded in the history. A
CONTROL2 write that reports an error still counts, because a WriteWithoutResponse may
have gone out anyway.

**Exception since 2026-10-18: the sudden failure.** A code 20 whose transfer ran at a
normal pace and then simply got no PROGRESS (the *sudden* signature, PROJECT_HISTORY
§7.1) is retried automatically: 120 s later, up to 3 times per incident, with the
session marker removed first. If all three fail, the uploader halts exactly as before.
The evidence (`docs/investigations/UPLOAD_FAILURES_2026-10-18.md`, PR #24):

- Uploads that followed a failure failed **5 of 12**; uploads whose previous upload had
  completed failed **4 of 12**. That's the same rate: a first upload on a connection
  fails about 1 time in 3 whatever came before.
- Retries with no power-off worked **2 of 4** (2026-10-18 13:42 and 14:11), and the
  13:42 connection then completed five more uploads. A keyboard that needed a power-off
  could not have done that.
- In the 6 logged retries without a proper reset, none damaged the LCD; each failure
  was an ordinary code 20.
- Under that rate, one retry clears about 6 halts in 10, three about 95 %.

The rule still holds for everything else. **Slowing** failures (sustained slow blocks
before the stall; the back-to-back pair at block 11 on 2026-10-15 is the one hint of a
state that persists), failures the uploader can't classify, **21**, and a session marker
without this run's exit record (a killed or crashed uploader) halt at once.

## Uploader exit codes (`tiga_stream_watch_flow`)

| Code | Class | Supervisor |
|---:|---|---|
| 0 | success (only with `--once`; watch mode normally never exits) | restart |
| 10 | keyboard unreachable: no adapter, not found, connect failed, link lost before CONTROL2 | retry with backoff, then every 5 min; **never halts** |
| 11 | connected, but the media service was missing or unusable: every candidate was reached and none qualified (issue #14), or subscribing to its notifications failed | retry with backoff, then every 5 min; **never halts** |
| 20 | transfer interrupted after CONTROL2 (also: refused to start because a session marker exists) | signature `sudden`: **automatic retry after 120 s, up to 3 per incident**, then halt. Otherwise **halt, power-cycle** |
| 21 | every block ACKed, COMPLETE never arrived | **halt, power-cycle** |
| 30 | bad arguments, missing or invalid stream, internal error | halt, no power cycle |
| other | crashed or killed | decided by the session marker (below) |

Each exit is logged as one line in `ble.log` and `ble_error.log`:
`EXIT code=20 (transfer interrupted after CONTROL2; …): timed out waiting for PROGRESS after 4K block 18/53 (last ACKed block 17/53) frame=…`.
The same information goes to `extracted\session_01_bulk_stream.bin.exit`.

For code 20 the uploader also logs `SIGNATURE sudden|slowing|unclassified frame=…` and
writes `signature=` to the exit record:

- **sudden:** the keyboard stopped sending PROGRESS (a block timeout), and the pace was
  normal: block median ≤ 500 ms and at most one block over 1000 ms. One slow block just
  before the stall still counts (2026-10-18 14:09: block 6 took 1860 ms, then nothing;
  a plain retry 2.5 minutes later worked).
- **slowing:** median over 500 ms, or two or more blocks over 1000 ms (2026-10-15 22:44
  and 22:46, 2026-10-16 18:49).
- **unclassified:** not a block timeout (link lost, write error, CONTROL2 ACK missing),
  no block ACKed at all, or the marker of an earlier run.

### Block timeout

After each 4096-byte block the uploader waits for the keyboard's PROGRESS. If it does
not come within the **block timeout**, the transfer is abandoned with code 20, and an
abandoned transfer is what leaves the keyboard needing its battery disconnected.

The default is **2500 ms** (`--block-timeout-ms` overrides it; `ble.log` prints the
value in effect as `Block timeout`). Healthy blocks peak at about 1 s.

From 2026-10-15 to 2026-10-16 the default was 10000 ms, on the theory that a block that
is merely slow, and then recovers, should be waited for. That case was never seen. In
27 completed uploads at 10000 ms no block came near 2.5 s, so all of them would also have
completed at 2500 ms. The one slowing transfer at 10000 ms did not recover: it crawled to
block 40 at 1.2–2.5 s per block, stalled completely and ended with code 20 and a battery
disconnect, like the 2500 ms failures, only about a minute later (PROJECT_HISTORY §7.1,
third part). So the default went back to 2500 ms on 2026-10-16. Neither value avoids
the battery disconnect; the timeout only decides when the failure is reported.

### Per-block timing (logging only)

Each block's latency is the time from the previous PROGRESS (for block 1, the start of
the bulk writes) to its own, so it includes that block's writes. Healthy blocks take
about 0.15 s. Nothing below changes a write, a timeout or an exit code.

- `SLOW block 7/53 took 1834 ms (elapsed=4.21s)` for any block over 1000 ms.
- `WARNING first upload is slow (block 5 at 2.42 s; healthy 0.7–0.85 s)` when the first
  upload on a connection reaches block 5 after more than 1.5 s. All three slowing
  failures so far were at 2.35–2.97 s. It comes after CONTROL2, so it warns; it cannot
  prevent anything.
- After every `UPLOAD COMPLETE` and `UPLOAD FAILED`:
  `TIMING frame=… blocks_acked=53 median=150ms p90=160ms max=420ms slowest=block 1 control1_ack=40ms control2_ack=55ms`
  (p90 is nearest-rank; on a failure it covers the blocks that were ACKed). The
  CONTROL ACK times run from the write to the second ACK of the pair; `-` means that
  stage was not reached.
- `<stream>.delivered` gets the same figures as `block_median_ms`, `block_p90_ms`,
  `block_max_ms`, `slowest_block`, `control1_ack_ms` and `control2_ack_ms`.

### The session marker

`extracted\session_01_bulk_stream.bin.session` is written just before CONTROL2,
rewritten after every ACKed block (frame ID, last ACKed block, total blocks), and
deleted when the keyboard sends COMPLETE.

It exists because exit codes cannot cover the important case. An uploader that is
killed, crashes, or dies with Windows shutting down never reports a class
(`taskkill` gives 1). The marker survives all of these. **If it exists when the uploader
exits, the supervisor halts, whatever the exit code.** If it exists when the uploader
*starts*, the uploader refuses to run (exit 20). That covers manual runs and a PC that
shut down mid-transfer.

## Supervisor policy (`tiga_supervisor_common.ps1`, used by both supervisors)

- **Slow retry** (10, 11): restart after 8, 16, 32, 60 s, then every 300 s for as long
  as it keeps failing. Neither code opens a media session, so the keyboard is not in a
  bad state; it is off, asleep or out of range. Halting here used to mean a dead display
  the next morning until someone ran `START_STACK.ps1`. Code 11 used to give up after
  three attempts, which is what stranded the display on 2026-10-15 when the uploader
  kept picking the wrong device (#14). The supervisor log says
  `retrying every 300s until it is (no halt)` once the slow interval starts.
- **Retryable, then give up** (an unexplained exit with no marker): restart after 8, 16,
  32, 60 s; the 5th consecutive failure halts as `gave-up`, no power cycle needed.
- A frame delivered during a run resets the count.
- **Sudden-failure retry** (20 with `signature=sudden` in this run's exit record): the
  supervisor logs `Sudden failure (code 20, signature sudden, last ACKed block N):
  automatic retry 1 of 3 at HH:MM:SS, in 120s.` When the time comes it removes the session marker and logs
  `Automatic retry 1 of 3: session marker removed; starting the BLE uploader (no
  power-off).` The outcome is logged as `Automatic retry 1 of 3 SUCCEEDED` (a frame was
  delivered, even while the uploader keeps running) or `Automatic retry 1 of 3 FAILED
  (code 20, signature …)`. A code 10/11 during a retry doesn't use up a retry: the normal
  reconnect backoff runs and the attempt still counts as the same retry. A delivered
  frame ends the incident and resets the count to 0. After the third failed retry, the
  halt below, with `auto_retries: 3` and the signature in the halt file. A retry that was
  waiting when the supervisor stopped is not resumed (its marker makes the next start
  halt, as before).
- **Halt** (marker present, 20 other than sudden, 20 after 3 retries, 21, 30, or an
  unexplained exit's retries exhausted): the uploader is not restarted. `logs\uploader_halted.json` records the time, code, meaning, class,
  frame ID, last ACKed block, the uploader's detail line, and an instruction. The
  renderer keeps running.
- **Clearing:** only `START_STACK.ps1` (the "Start Display" shortcut), run by hand. It
  deletes the marker and then the halt file. A running supervisor notices within one
  poll (2 s) and starts the uploader again; otherwise `Start-ScheduledTask` starts a
  fresh one. Nothing clears it on a timer.
- `STATUS_STACK.ps1` shows `HALTED - …` with the instruction, in red. While a retry is
  waiting or running it shows `RETRYING - sudden upload failure; automatic retry 1 of 3
  at HH:MM:SS`, and an `Auto-retry` line with the last events from
  `logs\uploader_retry.json`. A halt after retries adds the signature and the number of
  retries used.

## Hardware test plan: run with Michi present, eyes on the keyboard

Nothing in this plan has been run. **Each step can leave the LCD needing a power cycle;
that is the point.** Stop at the first unexpected result and power-cycle before
anything else.

### 0. Setup

The scheduled task on this PC currently runs the protected live copy
(`C:\Users\Regal\source\tiga_ble_re_tools\tiga_startup_supervisor.ps1`). To test this
branch, point the task at the repo, and restore it at the end (step 6).

1. `git checkout fix/a-retry-recovery`
2. In **Developer PowerShell for VS 2022**, in the repo:
   `powershell -NoProfile -ExecutionPolicy Bypass -File .\BUILD_AND_INSTALL.ps1`
   This builds everything, including the uploader from this branch, and re-registers the task to the repo.
3. Dry run, no Bluetooth (from `windows_ble_probe`):
   `.\target\release\tiga_stream_watch_flow.exe --stream ..\extracted\session_01_bulk_stream.bin`
   Expect a first line with a timestamp and `run_id=-`, then
   `Validated: 220649 bytes, 1239 writes, 53 x 4096-byte progress blocks`, then
   `EXIT code=0 (success)`. **If the first line has no timestamp, you are running an
   old binary. Stop.**
4. Offline tests: `py -m unittest discover -s tests`,
   `powershell -File tests\test_supervisor_logging.ps1`,
   `powershell -File tests\test_supervisor_retry.ps1`.
5. Make sure no halt is left over: `Test-Path logs\uploader_halted.json` must be False.

### 1. Baseline

1. Keyboard on, connected. `.\START_STACK.ps1`, wait about 20 s, play a track.
2. Expect in `logs\ble.log`: `UPLOAD START frame=…` then `UPLOAD COMPLETE frame=… in ~8s`.
   While it runs, `.\STATUS_STACK.ps1` shows `PENDING … (media session open: frame …, block N/53)`.
   Afterwards it shows `OK`, and `extracted\session_01_bulk_stream.bin.session` no longer exists.

### 2. Interrupted transfer (the main test)

1. Skip to the next track, and watch `ble.log` (`Get-Content logs\ble.log -Wait -Tail 5`).
2. About **3 s after `UPLOAD START`** (blocks are being ACKed, roar animation showing), kill the uploader only:
   `Stop-Process -Name tiga_stream_watch_flow -Force`
3. **Expect within about 2 s:**
   - `startup_supervisor.log`: `BLE uploader exited with code <N> (unexpected exit (crashed or killed))`.
     N is whatever a forced kill produces through the `cmd.exe` wrapper (1 or -1); any
     code not in the table is treated the same. Then
     `BLE uploader HALTED (session-open): uploader exited (code N: …) while a media session was open on the keyboard. Disconnect the keyboard's battery for at least 30 s …`
   - **No** `Started ble run=…` after it. The uploader is not restarted.
   - `logs\uploader_halted.json` exists, with `class: session-open`,
     `needs_power_cycle: true`, the frame ID from `UPLOAD START`, and a `last_acked_block`
     roughly matching the last `block NN/53 ACKed` line.
   - The renderer is still running (`STATUS_STACK.ps1` lists `tiga_windows_nowplaying_v2.py`).
   - `STATUS_STACK.ps1`: red `HALTED` block with the instruction.
4. Observe and note the keyboard's state **without** doing anything: stuck on the roar,
   white, previous image, or recovered by itself? This is new information. The history
   only says retrying is harmful, not what the idle state looks like.
5. Wait two minutes. The supervisor log must show nothing new about the uploader.
6. Recover. **The TIGA has no power switch, and unplugging USB doesn't reset it** — it
   has a battery. Open the case, disconnect the battery from the PCB, and wait at least
   30 seconds, pressing a few keys while it's disconnected. A 10–15 second disconnect
   was not enough in testing (PROJECT_HISTORY §7.1). Reconnect, then
   run `.\START_STACK.ps1`. Expect it to print the halt reason and `removed …session` /
   `removed …uploader_halted.json`, then a new `Started ble run=…` in the supervisor
   log, then a successful upload of the current track.

### 3. Stack stopped mid-transfer (a variant of 2)

1. Change track. About 3 s after `UPLOAD START`, run `.\STOP_STACK.ps1`.
2. Expect the marker to remain (`Test-Path extracted\session_01_bulk_stream.bin.session` → True).
3. Start the supervisor *without* START_STACK, the way a logon would:
   `Start-ScheduledTask -TaskName "TIGA Windows Media Display"`
4. Expect: the uploader starts, then immediately logs `EXIT code=20 (…): a media session was
   left open by an earlier run …`, and the supervisor halts. Nothing is sent to the
   keyboard (no `UPLOAD START`).
5. Recover as in 2.6.

### 4. Retryable: keyboard unreachable

This is not a halt test, and not the halt recovery of 2.6. Nothing is interrupted, so
nothing needs a reset afterwards.

1. Stack running, and **no upload in progress**: `STATUS_STACK.ps1` says OK (not `PENDING`
   with a media session open), and the last line of `logs\ble.log` is `UPLOAD COMPLETE`
   (or `ALREADY DISPLAYED`), not a `block NN/53` line. Disconnecting during an upload is
   a step-2 case and halts. Then make the keyboard unreachable. The TIGA has no power
   switch, so "off" means disconnecting the battery from the PCB.
2. Change track, so an upload is needed. Keep the battery disconnected for **1–2 minutes**,
   so the uploader logs a few code-10 retries.
3. Expect `EXIT code=10 (keyboard not reachable …)`, then `Restarting BLE uploader in 8s`,
   then 16s, 32s, 60s, then `Restarting BLE uploader in 300s … (no halt)`, repeating.
   **No halt file**, no marker.
   - The first failure may appear only when the next upload is attempted: the uploader
     does not notice a dropped link while idle (§7 "suspected but unproven").
   - If the link drops *during* an upload, after CONTROL2, that is a step-2 case and must halt with session-open.
4. Reconnect the battery and **don't** run anything. Within 5 minutes the next retry
   should connect and upload the current track, with no START_STACK. Here 1–2 minutes is
   only so the retries show up in the log; the at-least-30-s rule (PROJECT_HISTORY §7.1)
   is for recovering a halt, not for this test.

### 5. Code 21 (not induced)

"All blocks ACKed, no COMPLETE" can't be induced on purpose. It's covered by the
offline tests only. If it ever shows up in a real log, keep that log.

### 6. Restore

Re-point the task at the live copy (this registers a task; it does not modify that
folder):
`powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\Regal\source\tiga_ble_re_tools\install_tiga_startup.ps1`
Then check `(Get-ScheduledTask -TaskName "TIGA Windows Media Display").Actions.Arguments`.

### If it goes wrong

| Symptom | Meaning | Do |
|---|---|---|
| A `Started ble run=…` appears after an exit while the marker existed | the halt did not engage: **the bug this branch fixes** | `STOP_STACK.ps1` immediately, disconnect the battery for at least 30 s (not USB), keep all logs |
| No halt file, no marker after killing mid-upload | the kill landed before CONTROL2 (too early) or after COMPLETE (too late) | check `ble.log` for where it was; retry with a different delay |
| Halt file present but STATUS doesn't show HALTED | status script reading the wrong root (dev vs packaged) | compare `$Stream` and `$Logs` in `STATUS_STACK.ps1` with where the files are |
| START_STACK clears but the uploader doesn't start | supervisor not running, or task still pointing at `tiga_ble_re_tools` | check the task action (step 6) and `startup_supervisor.log` |
| LCD broken after recovery | the battery disconnect was too short (10–15 s was not enough on 2026-10-15), or only USB was unplugged, which is not a reset; or simply a new failure: on 2026-10-18 a first upload failed after a ~1.5 min full power-off, and first uploads fail about 1 time in 3 (`docs/investigations/UPLOAD_FAILURES_2026-10-18.md`) | disconnect the battery again for at least 30 s, pressing a few keys; do not upload again until the screen is normal |
