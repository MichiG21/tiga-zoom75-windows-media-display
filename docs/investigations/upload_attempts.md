# Upload attempts table — summary

`upload_attempts.csv` has one row per `UPLOAD START` in the stack's BLE logs. Regenerate
it with:

```powershell
py tools\upload_attempts.py --out docs\investigations\upload_attempts.csv
```

It reads `..\tiga-fresh-test\logs\` ("live") and `logs\` ("dev") by default, plus
`stack_events_context.csv`, which holds what the logs can't say: how each stack start
came about and whether the battery was disconnected before it (from PROJECT_HISTORY
§7.1, NOTES_FOR_MICHI and Michi's account in TASKS_F). Blank there means unknown. The
parser never opens Bluetooth. Its tests are `tests\test_upload_attempts.py`.

**As read on 2026-10-18 at 14:15 local:** 115 uploads from 2026-10-15 21:05 to
2026-10-18 14:11, **103 completed and 12 failed**. Every failure was a code 20 (no
PROGRESS within the block timeout after CONTROL2). Every upload ran with intra-block pace
0 ms. All went to `07:01:88:B3:66:1B`.

| | Uploads | Failed |
|---|---:|---:|
| First upload on a new connection | 24 | **9** (38 %) |
| Later upload on an open connection | 91 | **3** (3.3 %) |

The 12 failures:

| Start | Conn. | How it started | Battery before | Waiting for block | Last ACK at | Signature | RSSI |
|---|---|---|---|---:|---:|---|---:|
| 10-15 21:05:49 | 1st | stack restart | none recorded | 5 | ~0.76 s | sudden (inferred) | |
| 10-15 21:39:48 | 2nd | open connection | | 3 | ~1.29 s | unclear | |
| 10-15 22:44:31 | 1st | stack restart after a halt | 10–15 s | 11 | 12.02 s | slowing | |
| 10-15 22:46:42 | 1st | stack restart after a halt | none | 11 | 11.58 s | slowing | |
| 10-15 22:59:54 | 1st | stack restart (dev) | unknown | 41 | 6.12 s | sudden | −57 |
| 10-16 18:49:14 | 1st | stack restart | none | 41 | 67.39 s | slowing (10000 ms timeout) | −62 |
| 10-16 20:58:37 | 8th | open connection | | 6 | 1.58 s | sudden | −59 |
| 10-16 21:09:36 | 1st | halt cleared | 25 s, no keys | 5 | ~0.87 s | sudden | −66 |
| 10-18 13:20:06 | 1st | **first after a Windows boot** | none recorded | **27** | ~4.12 s | sudden | −69 |
| 10-18 13:41:21 | 1st | stack restart after a halt | **~1.5 min, full power-off** | **37** | ~5.64 s | sudden | −55 |
| 10-18 14:03:57 | 7th | open connection | | **52** | 8.46 s | sudden | −61 |
| 10-18 14:09:06 | 1st | halt cleared (**wait-then-retry, ~5 min, no power-off**) | none | 7 | 2.93 s | sudden after one slow block (block 6: 1860 ms) | −58 |

"Last ACK at" is seconds after the first bulk write; `~` marks values derived from the
time to failure (the 5-block lines don't cover them). Signatures as defined in §7.1.

**Not in the table:** `tiga_ble_re_tools\logs\` (the fallback). Its `ble.log` is from
before bug F: overwritten at every start, no timestamps and no `UPLOAD START` lines. It
holds three completed uploads (8.23, 8.90, 8.91 s, pace 0 ms) from its last run on
2026-10-15 before 20:45. Its supervisor log records 139 uploader starts since 2026-09-09
but no exit codes, so its failures can't be counted.

**Columns worth knowing:** `how_started` and `battery_disconnect_before` come from the
context file; the rest is from the logs. `latency_source` says whether median/p90/max
are from the `TIMING` line (per block, from 2026-10-16 20:51) or averages over the
5-block log lines (older uploads; those understate single slow blocks). `scan_s` and
`connect_s` have one-second resolution. `frames_installed_during_upload` counts renderer
frames written while the upload ran; the uploader reads the stream into memory before
sending, so these don't change what is sent.

Analysis: `UPLOAD_FAILURES_2026-10-18.md`.
