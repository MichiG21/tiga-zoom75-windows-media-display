# Clean stop: STOP_STACK asks the uploader instead of killing it

Status: implemented on `feature/clean-stop`, **not verified on hardware**. Not for merge
before the hardware test below.

## Why

On 2026-10-15/16 every failed first upload after a stack restart followed a restart that
did not reset the keyboard (PROJECT_HISTORY §7.1, third part):

| Before the first upload | First upload failed | Completed |
|---|---:|---:|
| Restart, no reset, last upload had completed | 3 | 6 |
| Battery disconnect ≥ 30 s | 0 | 4 |

After such a restart the keyboard was also invisible to scans for 19–67 s (four cases).
`STOP_STACK.ps1` ended the uploader with `Stop-Process -Force`, so every restart dropped a
live BLE connection without a disconnect, and could cut an upload in half. **Hypothesis:**
the keyboard keeps treating the dropped link as connected for a while (so it does not
advertise), and the first upload into it afterwards starts from a bad state. Nothing has
tested that yet; this change makes it testable.

## What it does

1. **Request file.** `STOP_STACK.ps1` writes `<stream>.stop` next to the stream the
   uploader watches (`extracted\session_01_bulk_stream.bin.stop`; packaged:
   `app\_internal\extracted\…`). It holds the time, STOP_STACK's PID and the uploader
   PIDs it waits for, for the log reader; only its existence matters.
2. **Uploader** (`tiga_stream_watch_flow.rs`). It checks for the file **only between
   uploads**, at the top of its watch loop (every ~250 ms when idle). An upload in
   progress is never abandoned for a stop: it completes, or fails the normal way (code 20
   or 21, session marker kept, supervisor halts). Then it logs
   `STOP requested; no upload in progress, disconnecting.`, calls the existing
   `peripheral.disconnect()` (`Disconnected cleanly.`) and exits **0**. If the file is
   already there when it starts, it does not connect at all and exits 0.
3. **STOP_STACK.ps1** (dev and packaged; logic in `Invoke-StackStop` in
   `tiga_supervisor_common.ps1`): finds `tiga_stream_watch_flow.exe` processes, writes the
   request, waits up to **30 s** for them to exit, then does what it always did:
   `Stop-ScheduledTask`, then force-kills every stack process that is left. It removes the
   request file at the end. One line goes to the console and to
   `logs\startup_supervisor.log`:
   - `STOP_STACK: BLE uploader exited on request after 0.4 s.`
   - `STOP_STACK: BLE uploader did not exit within 30 s; force-killing it (pid 1234).`
   - `STOP_STACK: no BLE uploader was running.`
4. **Supervisor.** When the uploader exits while `<stream>.stop` exists, the exit is
   classed `stopped-on-request`: logged, not counted as a failure, not restarted. A session
   marker or code 20/21 still wins and halts, because then the keyboard does need a
   battery disconnect. While the file exists the supervisor starts no uploader. If the
   file disappears while the supervisor is still running (STOP_STACK interrupted), it
   logs `Stop request withdrawn` and starts the uploader again, like a cleared halt.
5. **Stale requests.** A supervisor removes any `<stream>.stop` when it starts (the
   scheduled task at logon does not go through START_STACK), and `START_STACK.ps1`
   removes it before `Start-ScheduledTask`.

### Why a request file

- The supervisor and the uploader already talk through sidecars next to the stream
  (`.session`, `.exit`, `.delivered`, `.frame`, `.device`), and `<stream>.stop` uses the
  same paths in the dev and packaged layouts. The supervisor can see it too, which is
  what stops it restarting the uploader; a signal to one process could not do that.
- Windows has no SIGTERM. `Ctrl+C`/`CTRL_BREAK` needs a shared console, and the uploader
  runs hidden under `cmd.exe /c` from `Start-Process`; attaching to its console from
  STOP_STACK is fragile. A named pipe or event would need new code on both sides for no
  gain at a 250 ms polling interval.
- A file survives anything. The cost is cleanup, which is why it is removed in three
  places (STOP_STACK's end, supervisor start, START_STACK).

### Why 30 s

A healthy upload takes 8.4–10.1 s; a failing one ends within ~15 s at the 2500 ms block
timeout (the slowing failures took 14.5–15 s). Before its first upload the uploader can
also be scanning (up to 15 s) or connecting, which it does not interrupt; it checks the
request only once connected and idle. 30 s covers an upload that has only just started,
with margin. Longer would make a stop feel hung, and a stuck uploader gets the old
force-kill anyway.

## What it does not fix

- **Windows shutdown, sign-out, and the scheduled task being stopped on its own**
  (`Stop-ScheduledTask` run by hand, Task Scheduler's own limits) still end the uploader
  with a kill. The session marker still
  covers a kill mid-upload, as before. "Standby + Shut Down" in the display-control
  utility waits for the standby frame's delivery and then shuts Windows down, which kills
  the uploader: the link is dropped uncleanly there too, but between uploads.
- **Whether `disconnect()` is cleaner than a kill at the radio level is unknown.** In
  btleplug 0.12 on Windows, `disconnect()` drops the device object, which `Close()`s its
  GATT services and the `BluetoothLEDevice`; Windows then drops the link once nothing
  holds it. A killed process releases the same objects without the `Close()` calls, and
  Windows cleans up after it. Both may look the same to the keyboard. The firm gain is
  that **a stop no longer cuts an upload in half**; whether the keyboard also behaves
  better after a restart is exactly what the hardware test has to show.
- No GATT unsubscribe before disconnecting. That would be a CCCD write the keyboard has
  never been sent by this uploader; left out deliberately. Try it as a separate step if
  the plain disconnect makes no difference.
- STOP_STACK of one install does not ask the uploader of another (the request file goes
  next to its own stream). Running `tiga-dev\STOP_STACK.ps1` while `tiga-fresh-test` runs
  falls back to the 30 s wait and the force-kill.
- An installer upgrade runs the new STOP_STACK against the old uploader, which does not
  know the request: up to 30 s wait, then the old force-kill.

## Offline tests

- Rust: `stop_request_is_a_sidecar`. The loop placement (between uploads only) is
  structural and visible in the diff. Driving `run()` itself offline was not done: with a
  bug it would open Bluetooth.
- `tests\test_supervisor_retry.ps1`: decision table (stop after code 0/10 → no restart;
  code 20 or a marker with a stop → halt); the real supervisor loop with dummy uploaders
  (stale request removed at start, requested exit not restarted and not a halt, failed
  upload during a stop still halts, withdrawn request restarts it); `Invoke-StackStop`
  with dummy processes (cooperating uploader exits 0 and the task stop and force-kill run
  only afterwards; an uploader ignoring the request is force-killed after the timeout;
  nothing running means no request and no wait); text checks that both STOP_STACKs go
  through `Invoke-StackStop` and both START_STACKs withdraw a request before
  `Start-ScheduledTask`. The real STOP_STACK.ps1 is never run by the tests.

## Hardware test plan

Run from the install that has this branch (point the scheduled task at it with its
`install_tiga_startup.ps1`). Before starting, do a 30–40 s battery disconnect so the
keyboard starts from a known state. Every restart below is **without** a battery reset.

For each restart:

1. Wait until at least one upload has completed and the uploader is idle (`ble.log`:
   `UPLOAD COMPLETE`, then nothing for a few seconds).
2. `STOP_STACK.ps1`. Note the `STOP_STACK:` line. Expect `exited on request after <1 s`
   when idle. In `ble.log`: `STOP requested; …`, `Disconnected cleanly.`, `EXIT code=0`.
3. Wait 10 s, `START_STACK.ps1`.
4. Record from `ble.log` / `startup_supervisor.log`:
   - **invisible period**: time from the supervisor start to the first scan that sees
     `Zoom75 Tiga`, and the number of code-10 exits before it
   - **first upload**: completed or failed, block 5's time (Task 2's `TIMING` /
     `WARNING first upload is slow` lines, if that branch is in)
5. If the first upload fails: battery disconnect 30–40 s, `START_STACK.ps1`, and start
   counting again.

Also once each, to check the stop path itself:

- **Stop during an upload**: skip a track, run `STOP_STACK.ps1` about 2 s after
  `UPLOAD START`. Expect the upload to complete (`UPLOAD COMPLETE`), then the stop, all
  within ~10 s, and no halt.
- **Stop during the first scan**: `START_STACK.ps1`, then `STOP_STACK.ps1` within 5 s.
  Expect either a clean exit or the 30 s fallback; either is acceptable.

**Comparison.** The baseline is the table above: 3 failed first uploads in 9 restarts
without a reset, and an invisible period after 4 of them (19–67 s). Aim for at least 10
restarts with the clean stop. If there are no first-upload failures and no invisible
periods, that supports the hypothesis (10 of 10 against 3-of-9 is suggestive, not proof);
if both still occur at about the old rate, the kill was not the cause, and the change is
still worth keeping for never cutting an upload in half.
