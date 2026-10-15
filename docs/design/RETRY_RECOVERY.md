# Uploader failure classes and recovery (issue #4)

**Status:** implemented on `fix/a-retry-recovery` and merged. The interrupted-transfer
halt (step 2's outcome, from a real PROGRESS timeout rather than a kill) was seen on
hardware on 2026-10-15; the rest of the plan below has not been run. The slow retry for
10 and 11 (below) was added on `fix/device-selection` and is tested offline only.

## The rule

A failure is safe to retry only if no media session was opened on the keyboard. The
boundary is **CONTROL2**: it puts the stock firmware into its upload state (the roar
animation, PROJECT_HISTORY §4.1). Before CONTROL2 the keyboard is untouched. From the
moment CONTROL2 is *attempted*, a failure may leave a partial session, and another
upload on top of it is what led to the broken-LCD states recorded in the history. A
CONTROL2 write that reports an error still counts, because a WriteWithoutResponse may
have gone out anyway.

## Uploader exit codes (`tiga_stream_watch_flow`)

| Code | Class | Supervisor |
|---:|---|---|
| 0 | success (only with `--once`; watch mode normally never exits) | restart |
| 10 | keyboard unreachable: no adapter, not found, connect failed, link lost before CONTROL2 | retry with backoff, then every 5 min; **never halts** |
| 11 | connected, but the media service was missing or unusable: every candidate was reached and none qualified (issue #14), or subscribing to its notifications failed | retry with backoff, then every 5 min; **never halts** |
| 20 | transfer interrupted after CONTROL2 (also: refused to start because a session marker exists) | **halt, power-cycle** |
| 21 | every block ACKed, COMPLETE never arrived | **halt, power-cycle** |
| 30 | bad arguments, missing or invalid stream, internal error | halt, no power cycle |
| other | crashed or killed | decided by the session marker (below) |

Each exit is logged as one line in `ble.log` and `ble_error.log`:
`EXIT code=20 (transfer interrupted after CONTROL2; …): timed out waiting for PROGRESS after 4K block 18/53 (last ACKed block 17/53) frame=…`.
The same information goes to `extracted\session_01_bulk_stream.bin.exit`.

### Block timeout

After each 4096-byte block the uploader waits for the keyboard's PROGRESS. If it does
not come within the **block timeout**, the transfer is abandoned with code 20, and an
abandoned transfer is what leaves the keyboard needing its battery disconnected. A
block that is merely slow should therefore be waited for.

The default is **10000 ms** (`--block-timeout-ms` overrides it; `ble.log` prints the
value in effect as `Block timeout`). It was 2500 ms until 2026-10-15. All five
interrupted transfers that day ran with 2500 ms; the uploads after the change all
completed, but none came near either limit and they followed a full battery reset, so
**the longer wait is an unverified mitigation, not a fix** (PROJECT_HISTORY §7.1). Its
cost is that a keyboard that is truly stuck is detected about 7.5 s later.

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
- **Halt** (marker present, 20, 21, 30, or an unexplained exit's retries exhausted): the uploader is not
  restarted. `logs\uploader_halted.json` records the time, code, meaning, class,
  frame ID, last ACKed block, the uploader's detail line, and an instruction. The
  renderer keeps running.
- **Clearing:** only `START_STACK.ps1` (the "Start Display" shortcut), run by hand. It
  deletes the marker and then the halt file. A running supervisor notices within one
  poll (2 s) and starts the uploader again; otherwise `Start-ScheduledTask` starts a
  fresh one. Nothing clears it on a timer.
- `STATUS_STACK.ps1` shows `HALTED - …` with the instruction, in red.

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
     `BLE uploader HALTED (session-open): uploader exited (code N: …) while a media session was open on the keyboard. Power-cycle the keyboard with its power switch …`
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
6. Recover: power-cycle the keyboard **with its power switch**. Unplugging USB is not a
   power cycle: the TIGA has a battery, and on 2026-10-15 a 5-second USB unplug did not
   reset it. If the LCD stays white, frozen, or stuck on the roar, **disconnect the
   battery** (PROJECT_HISTORY §7.1). Then
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

1. Stack running, with OK status. Switch the keyboard's Bluetooth off, or turn the keyboard off.
2. Change track, so an upload is needed.
3. Expect `EXIT code=10 (keyboard not reachable …)`, then `Restarting BLE uploader in 8s`,
   then 16s, 32s, 60s, then `Restarting BLE uploader in 300s … (no halt)`, repeating.
   **No halt file**, no marker.
   - The first failure may appear only when the next upload is attempted: the uploader
     does not notice a dropped link while idle (§7 "suspected but unproven").
   - If the link drops *during* an upload, after CONTROL2, that is a step-2 case and must halt with session-open.
4. Turn the keyboard back on and **don't** run anything. Within 5 minutes the next retry
   should connect and upload the current track, with no power cycle and no START_STACK.

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
| A `Started ble run=…` appears after an exit while the marker existed | the halt did not engage: **the bug this branch fixes** | `STOP_STACK.ps1` immediately, power-cycle (power switch or battery, not USB), keep all logs |
| No halt file, no marker after killing mid-upload | the kill landed before CONTROL2 (too early) or after COMPLETE (too late) | check `ble.log` for where it was; retry with a different delay |
| Halt file present but STATUS doesn't show HALTED | status script reading the wrong root (dev vs packaged) | compare `$Stream` and `$Logs` in `STATUS_STACK.ps1` with where the files are |
| START_STACK clears but the uploader doesn't start | supervisor not running, or task still pointing at `tiga_ble_re_tools` | check the task action (step 6) and `startup_supervisor.log` |
| LCD broken after recovery | partial session survived a power-switch cycle (or only USB was unplugged, which is not a power cycle) | disconnect the battery; do not upload again until the screen is normal |
