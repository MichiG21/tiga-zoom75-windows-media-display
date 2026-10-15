# Design: acknowledged "Standby + Shut Down" (issue #7)

**Status: proposal for review. Not merged. Not tested on hardware.**
Branch `review/shutdown-ack`, based on `fixes/beta-2`.

## Problem

`tiga_display_control.exe` writes `standby`, sleeps a fixed 12 s, then runs
`shutdown /s /t 0`. Nothing checks that the renderer handled the command or that the
keyboard received the frame. One upload takes ~8 s on a good link. An upload already
in flight, a slow link or a reconnect can all run past 12 s, and then Windows shuts
down mid-transfer. That is exactly the state that needs a keyboard power cycle (#4).

## Design

Two acknowledgements. Each builds on the plumbing added for #9:

```
GUI                         renderer                      uploader
 | write "standby <id>"  ->  |                              |
 |                           | render + install frame F     |
 |                           |   (<stream>.frame: F, crc)   |
 |  <- tiga_display_command.ack                             |
 |     command_text = "standby <id>", frame_id = F, crc     |
 |                           |                              | upload F
 |                           |                              | keyboard COMPLETE
 |  <- <stream>.delivered: frame_id = F, crc ---------------|
 | shutdown /s /t 0
```

1. **Renderer ack.** After handling any command, the renderer writes
   `tiga_display_command.ack` (atomic tmp + replace) with the exact command text it
   handled plus the frame ID and CRC-32 of the frame it installed. The GUI matches on
   the command text, which includes the unique ID from #6, so an ack left from an
   earlier command never counts.
2. **Delivery ack.** The GUI waits until `extracted\session_01_bulk_stream.bin.delivered`
   names **the same frame ID** (and CRC). The uploader writes that file only after the
   keyboard's completion notification (`88 .. 03 0c 08 06 02`).

### Why match on frame ID, not just CRC

The standby image contains the time to the minute. Two standby frames rendered in the
same minute are byte-identical, so they have the same CRC. Matching on CRC alone would
accept an *earlier* delivery of that image, even while a different frame is
mid-upload. Frame IDs are unique per render.

### The no-op upload case

The uploader skips a stream that is byte-identical to its last completed upload. In
that case it now writes `.delivered` for the new frame ID anyway (logged as
`ALREADY DISPLAYED`). The uploader is sequential: when it is in this branch it is not
uploading, and the keyboard shows exactly those bytes. Without this, pressing
Standby + Shut Down within a minute of Lock Standby would wait out the full timeout.

### Timeouts and failure handling

| Stage | Timeout | Reasoning |
|---|---|---|
| Renderer ack | 15 s | The renderer polls once a second; rendering takes < 1 s. |
| Delivery ack | 90 s | Covers an in-flight upload (~8 s), ours (~8 s), a reconnect, and margin. |

If either stage times out, the GUI **does not shut down automatically**. It shows a
Yes/No warning that explains what was not confirmed, with **No** as the default. On
No, the button is re-enabled and the status line says why. On Yes, it shuts down
(status: "without confirmation").

The button is disabled while a wait is running, so two waits cannot overlap.

## Files changed

- `tiga_windows_nowplaying_v2.py`: `write_display` returns `(frame_id, crc)`; new
  `write_command_ack`; called after every handled command.
- `windows_ble_probe/src/bin/tiga_stream_watch_flow.rs`: `note_already_displayed`.
- `tiga_display_control.cpp`: `write_command_text` returns the text it wrote; the
  shutdown worker waits for both acks and asks on timeout.
- Tests: renderer ack through `main()`; uploader no-op recording.

## Questions for review

1. **On timeout: ask or shut down?** The proposal asks. That's safer for the
   keyboard, but if nobody answers the dialog, the PC stays on. Alternatives: shut
   down after a second, longer timeout; or a countdown dialog that defaults to
   shutting down.
2. **Is 90 s right?** It's long for a "shut down" button, but every second removed
   brings back the original risk. The status line shows which stage is pending.
3. **Uploader not running** (the stack is stopped): the renderer is also stopped, so
   the 15 s renderer timeout catches it early. If only the uploader is down, the user
   waits the full 90 s. Should the GUI check for the uploader process up front?
4. The ack lives next to the command file (`exe_dir`); `.delivered` is under
   `exe_dir\extracted`. This holds for the dev layout (repo root) and the packaged
   layout (`app\_internal`) because the renderer's `HERE` and the GUI's exe directory
   are the same folder in both. Worth a check on an installed build.

## Hardware test plan (with Michi present)

1. Stack running, keyboard idle: Standby + Shut Down. Expect "confirmed", shutdown about 8 s after the click.
2. Start a track change, then immediately Standby + Shut Down. Expect it to wait for both
   uploads; the shutdown follows the standby frame's `UPLOAD COMPLETE` in `ble.log`.
3. Lock Standby, then within the same minute Standby + Shut Down. Expect
   `ALREADY DISPLAYED` in `ble.log` and immediate confirmation.
4. Stop the stack, then Standby + Shut Down. Expect the renderer-timeout dialog at about 15 s. Choose No.
5. Keyboard switched off: expect the delivery-timeout dialog at about 90 s. Choose No.
