# Why do uploads fail? Analysis of every logged upload, 2026-10-18

Analysis only. Nothing was sent to the keyboard; the live stack was not touched.
Sources: `tiga-fresh-test\logs\` and `tiga-dev\logs\` (read-only), the context file
`stack_events_context.csv`, PROJECT_HISTORY §7.1, NOTES_FOR_MICHI, Michi's account in
TASKS_F and during the run, the Windows System event log (boot, sleep and shutdown times
only), and the 631-page project archive PDF (for the pacing decision). Table:
`upload_attempts.csv`, built by `tools\upload_attempts.py`.

## The table in short

As read at 14:15 local on 2026-10-18: **115 uploads, 103 completed, 12 failed.** All 12
failures are code 20: the keyboard stopped sending PROGRESS after CONTROL2. There is no
other failure type after CONTROL2 in the record. All 115 ran at intra-block pace 0 ms.

| | Uploads | Failed |
|---|---:|---:|
| First upload on a new connection | 24 | **9** (38 %) |
| Later upload on an open connection | 91 | **3** (3.3 %) |

The per-failure list is in `upload_attempts.md`. Today's first uploads and retries
alone: three failures and two successes, plus one failure on an open connection:

| Time | What it was | Result |
|---|---|---|
| 13:20:06 | first upload after a Windows boot (keyboard idle since the Standby + Shut Down at 2026-10-16 23:07) | failed at block 27/53, sudden |
| 13:41:21 | first upload after STOP_STACK, a ~1.5 min full power-off, START_STACK | failed at block 37/53, sudden |
| 13:42:19 | START_STACK again, no power-off, 46 s after the halt | completed, 8.71 s |
| 13:48–13:59 | five more on that connection | completed, 9.52–10.00 s |
| 14:03:57 | 7th upload on that connection | failed at block 52/53, sudden |
| 14:09:06 | **wait-then-retry**: START_STACK, no power-off, ~5 min after the 14:04:09 halt | failed at block 7/53: block 6 took 1860 ms, then nothing |
| 14:11:43 | START_STACK again, no power-off, ~2.5 min after the 14:09:13 halt | **completed, 8.66 s** (median 150 ms, max 195 ms) |

## Main finding: independent failures fit the data; a "stuck until power-off" keyboard doesn't

Michi asked this question after 14:11. It's answered here first because it changes how
everything below reads.

**Hypothesis I (independent):** every first upload on a connection fails with some
probability p, whatever happened before. **Hypothesis S (stuck):** a failure leaves the
keyboard in a state that only a power-off clears.

- **S in its strict form is refuted.** Two retries with no power-off completed (13:42
  after 46 s; 14:11 after ~2.5 min), and the 13:42 connection then completed five more
  uploads normally. A keyboard that needed a power-off could not do that.
- **I fits everything measured.** First uploads that followed a *failure*: **5 of 12
  failed**. First uploads whose previous upload had *completed*: **4 of 12 failed**. The
  rates are identical (Fisher p = 1.0). The overall first-upload rate is p ≈ 9/24 ≈ 0.38.
  Splitting the after-failure attempts by what was done in between:

  | Between the failure and the attempt | n | Failed | Chance of that many failures or more (≤ for the reset row) under I, p = 0.38 |
  |---|---:|---:|---:|
  | Nothing (22:46, 13:42, 14:09, 14:11) | 4 | 2 | 0.48 |
  | Nothing or a too-short disconnect (adds 22:44, 21:09) | 6 | 4 | 0.15 |
  | A full reset (21:15, 22:49, 23:06, 18:54, 21:14, 13:41) | 6 | 1 | 0.27 (1 or fewer) |

  None of these is surprising under I. "Reset helps" (1/6 against 4/6) points the way S
  would predict, but at p ≈ 0.15 with n = 6 + 6 it is not evidence.
- **Where I fits worst: the slowing pair.** 22:44 and 22:46 on 2026-10-15 both ran
  slowly and both stalled at block 11, with different images, back to back, with no
  proper reset in between. Only 3 of the 12 failures are slowing at all. Two consecutive
  slowing failures at the same block are hard to get by chance. That looks like a real
  keyboard state, possibly the one a short brown-out creates (§7.1). So a weaker S
  stays plausible *for the slowing signature only*: 18:49 on 10-16 is the third slowing
  failure, and it was followed by a 40 s reset, so whether a wait would have cleared it
  is unknown. Every sudden failure is consistent with I.
- **Verdict:** **supported** that sudden failures behave like independent draws at
  roughly 1 in 3 per first upload. **Not supported** that a halted keyboard needs a
  power-off. **Can't tell** whether a power-off lowers the next attempt's risk (n = 6
  each way), or whether the slowing state persists.

### What this would mean for bug A's halt policy

Bug A's rule is that a code 20 halts the uploader until a power-off, because "the
keyboard may hold a partial media session". It rests on the premise that a retry into
that state fails, or breaks the LCD. The record since the halt was introduced doesn't
support the premise for sudden failures. 6 retries without a proper reset produced 2
completions and 4 failures. Each failure was cleanly reported, with no recorded LCD
damage, and was cleared afterwards (by a reset or by a later retry). Under I:

- **One delayed automatic retry** would recover about 62 % of halts (1 − 0.38) with no
  user action; **two** about 86 %. Each extra attempt costs ~10 s and, if it fails,
  exactly what a halt costs now.
- **What a revised policy would look like (proposal, not implemented):** after a code 20
  with the *sudden* signature, wait 60–120 s, retry; at most two retries per halt, then
  halt as now. After a *slowing* failure (slowing blocks before the stall, or `SLOW`
  lines on several blocks), halt at once as now, since that is the one case with a hint of
  a persistent state. Keep the session marker and the attempt count in the halt file.
- **Why not yet:** the premise "a retry can break the LCD" comes from earlier experiments
  (§7, bug A) that are not in these logs, and the after-failure n is 12. This changes a
  safety rule, so it's **Michi's decision**. The cheapest confirmation is to keep doing
  by hand what happened today: on the next halts, wait ~2 min and START_STACK without a
  power-off, and power off only after two such retries fail (experiment 1 below). If 6–8
  such halts recover at about the 1-in-3 failure rate, I is confirmed well enough to
  automate.

**Small numbers throughout.** n is given with every figure. A Fisher exact p-value is
quoted where it helps judge whether a difference could be chance. It is a guide, not
proof: several of the groupings were chosen after looking at the data.

---

## Task 2 — what separates failures from successes

### 1. How the connection started — **supported: the first upload is the risky one; a reset does not make it safe**

First uploads on a connection, by what came before (n = 24, 9 failed):

| Before the first upload | n | Failed |
|---|---:|---:|
| Windows boot or resume (17:52 on 10-16 after sleep; 13:20 today after a cold boot) | 2 | 1 |
| Stack restart, no reset, last upload had completed (21:35, 23:09, 23:16, 23:46, 23:52, 18:49, 20:21) | 7 | 1 |
| Same, but state unclear (21:05: what the old stack last did is unknown; 22:59: battery state unclear) | 2 | 2 |
| Stack restart with a battery disconnect, last upload had completed (20:52, length unknown) | 1 | 0 |
| After a failure: full reset of 30 s or more, or of unknown length (21:15, 22:49, 23:06, 18:54, 21:14, **13:41**) | 6 | **1** |
| After a failure: too-short disconnect (10–15 s at 22:44; 25 s without key presses at 21:09) | 2 | 2 |
| After a failure: no reset at all (22:46 after 116 s; **13:42 after 49 s; 14:09 after ~5 min; 14:11 after ~2.5 min**) | 4 | 2 |

- **First vs later:** 9/24 against 3/91 (p ≈ 0.00002). Restricted to first uploads
  whose previous upload had *completed*, so that no failure state carries over: 4/12 (or
  1/7 for the cleanly documented ones) against 3/91. The first upload on a connection
  is clearly the risky moment, roughly ten times the rate of a later upload.
- **A reset does not make the next upload safe.** After a full reset, 1 of 6 failed
  (today's 13:41). That is about the rate of a first upload after a clean restart (1/7,
  or 3/9 with the unclear ones; p ≈ 0.6). Today's 13:41 failure after a ~1.5 min power-off
  with USB unplugged is real: every reset is a complete power-off (point 7). "Every
  recovery with a disconnect of 30 s or more has worked" no longer holds.
- **Is no reset worse than a reset?** After a failure, no reset or a too-short one: 4 of
  6 failed. A full reset: 1 of 6 failed (p ≈ 0.24). Pointing that way, but **can't tell**
  at this n. The case for "the reset matters" rests on 22:44, 22:46, 21:09 and 14:09.
  The case against is 13:41 (reset, failed), and 13:42 and 14:11 (no reset, worked).
- **Earlier explanations need weakening.** If an ordinary first upload fails about a
  third of the time, 4 of 4 resets of 30 s or more working (2026-10-15/16) had a
  probability of roughly 0.7⁴ ≈ 0.24 by chance alone. So the "30 s working minimum" was
  never well established. The 21:09 failure was put down to the short disconnect; it is
  equally consistent with the base rate.

### 2. Retry delay after a halt — **not supported: the delay doesn't matter**

Every attempt that followed a failure (n = 12):

| Failure | Next attempt | Delay | Reset in between | Result |
|---|---|---:|---|---|
| 10-15 21:05 | 21:15:32 | 579 s | yes, length unknown | completed |
| 10-15 21:39 | 22:44:31 | 3878 s | 10–15 s | failed (slowing) |
| 10-15 22:44 | 22:46:42 | 116 s | none | failed (slowing) |
| 10-15 22:46 | 22:49:35 | 158 s | 30 s | completed |
| 10-15 22:59 | 23:06:33 | 389 s | 30 s | completed |
| 10-16 18:49 | 18:54:09 | 217 s | 40 s | completed |
| 10-16 20:58 | 21:09:36 | 654 s | 25 s, no key presses | failed (sudden) |
| 10-16 21:09 | 21:14:10 | 270 s | about 1 min, key presses | completed |
| 10-18 13:20 | 13:41:21 | 1268 s | ~1.5 min full power-off | **failed** (sudden) |
| 10-18 13:41 | 13:42:19 | **49 s** | **none** | **completed** |
| 10-18 14:03 | 14:09:06 | **~298 s** | **none** (the wait-then-retry experiment) | **failed** (one slow block, then stall) |
| 10-18 14:09 | 14:11:43 | **~150 s** | **none** | **completed**, 8.66 s |

Delay sorts nothing: failures came after 116, 298, 654, 1268 and 3878 s; the successes
without a reset after 49 and ~150 s. **Retry without any reset: 2 of 4 completed**
(13:42, 14:11; 22:46 and 14:09 failed). A partial session doesn't need a power-off, and
nothing suggests that a longer wait is better. Each retry looks like a fresh draw (see
the main finding above).

### 3. Where in the transfer it fails — **can't tell; weak early cluster, nothing at the frame boundary**

Block whose PROGRESS never came, all 12: **3, 5, 5, 6, 7, 11, 11, 27, 37, 41, 41, 52.**

- **Frame-1/frame-2 boundary** (stream offset 110,565, inside block 27): one failure,
  13:20 today. Not a cluster.
- **4096-byte boundaries:** every failure is at one by construction, since that is where
  the uploader waits. Nothing distinguishes particular regions. Block 1 (the header) has
  never failed; blocks 52/53 (the trailer) once.
- **Blocks 3–7:** 5 of 12. Under a uniform spread, 5 or more of 12 in a five-block
  window has p ≈ 0.003. **But the window was picked after looking**, and the five are
  mixed (two later uploads, whose slow block always falls in blocks 1–3; one of unknown
  state; two retries without a proper reset). Worth watching, not a finding.
- The two repeats at 11 were consecutive runs in the same keyboard state (22:44, 22:46).
  The two at 41 are unrelated (22:59 sudden, 18:49 slowing).

### 4. Time since CONTROL2 — **not supported**

When the last PROGRESS arrived, in seconds after the first bulk write (CONTROL2 is
0.1–0.4 s earlier). For sudden stops: 0.76, 0.87, 1.29, 1.58, 2.93, 4.12, 5.64, 6.12,
8.46 s. For slowing ones: 11.58, 12.02, 67.39 s. Sudden stops are spread over the whole
transfer, with no preferred time.

### 5. The image — **not supported**

Two CRCs have both failed and completed: `0714ef6a` (failed 18:49, completed 18:54 on
10-16) and `107c7cee` (completed live 22:57, failed in dev 22:59 on 10-15). Failures
covered startup standby (3), media changed (6), fresh playback (3); every one of those
triggers also completed many times. The other failed frames were never sent again
(standby frames carry the clock, so their CRCs are unique).

### 6. Intra-block pacing — **can't tell: it has never been varied**

- **Every logged upload ran at 0 ms**: all 114 here and the fallback's three. No upload
  with a non-zero pace exists in any log.
- **Why 0 was chosen:** the archive PDF (pages 297–299) records the plan when flow
  control was introduced: first a "conservative" 2 ms, then 1 ms, then 0 ms, "each
  separately". The next entry is Michi: "0 ms klappt ohne Probleme" ("0 ms works without
  problems"), then 7.68 s for that upload. No result for 2 ms or 1 ms is recorded. **So 0 ms rests on
  one successful upload.** The uploader's own default of 2 ms is left over from that
  plan. The October audit listed "launched with zero intra-block delay" as a suspected
  contributor.
- **Why it's a plausible mechanism for the sudden signature:** bulk data goes out as
  ATT Write Commands (`WriteWithoutResponse`), which are not acknowledged. If one is lost
  between Windows and the keyboard, the keyboard never gets its 4096th byte, never sends
  PROGRESS, and the uploader times out at a healthy pace with no warning. That is exactly
  the sudden signature. A lost write is more likely when writes go back to back. **This
  is a hypothesis.** Nothing in the logs shows a lost write. 2 ms would cost about 2.5 s
  per upload (1239 writes).

### 7. Keyboard activity and power — **typing ruled out; resets confirmed real**

- **Typing goes over the USB-C cable** (plugged in at all times), not Bluetooth (Michi,
  2026-10-18). "Typing interferes over the radio" is out. Dropped.
- **Every recorded "battery disconnect" was a full power-off** (Michi, 2026-10-18). The
  front panel can't come off with USB-C plugged in, so the procedure is: unplug USB-C,
  remove the front panel, lift the PCB sandwich so the magnetic battery contacts
  separate, wait (over a minute today), reassemble, plug USB-C back in. Every reset
  therefore also included a USB unplug and replug. "The reset didn't really happen" is
  ruled out, and **today's failure right after a ~1.5 min power-off is a real finding**.
  Recorded in §7.1.

### 8. Anything else

- **Pre-CONTROL2 signal: none found.** CONTROL1/CONTROL2 ACKs on the four first-upload
  failures with timing (21:09, 13:20, 13:41, 14:09): 20/77, 21/106, 26/66, 19/75 ms.
  On the four first-upload successes: 18/76, 32/73, 30/70, 28/69 ms. No separation (n = 4
  vs 4), so the "stop before CONTROL2 if the keyboard looks unwell" idea from §7.1 has
  nothing to work with.
- **No precursor inside the transfer** for most sudden stops. 13:20 and 13:41 ran at
  median 154–157 ms, p90 172–173, max 193–195 ms right up to the stall. That is
  indistinguishable from successful first uploads (max 195–218 ms). **14:09 is the one
  exception:** block 5 at 1.01 s (healthy first uploads 0.68–0.92 s), then block 6 at
  1860 ms, then nothing. It's the first time `TIMING` has caught a single slow block
  directly before a stall.
- **Time of day:** failures at 13:20–14:09, 18:49, 20:58–22:59; successes in the same
  hours. Not supported.
- **Renderer activity:** frames installed while the upload ran: failures 0–2, successes
  0–4. The uploader reads the whole stream into memory before CONTROL1, so a rewrite
  during the transfer can't change what is sent. Not supported.
- **The 20:58 Spotify auto-advance and today's 14:03:** both "media changed", the same
  trigger as 69 completed uploads. Nothing special about the trigger. The three
  open-connection failures came 145, 223 and 268 s after the previous upload; 42 of 88
  completed later uploads also followed a gap of more than 120 s. Not supported.
- **RSSI:** failures −55, −57, −58, −59, −61, −62, −66, −69; successes −56 to −79. The
  strongest signal on record (−55) failed, the weakest (−79) worked four times. Not
  supported.
- **Connection age:** the open-connection failures came 4, 6 and 22 minutes into their
  connections; the 17:52 connection on 10-16 ran 22 uploads over 54 minutes without one.
  Can't tell.
- **The scan stays running during uploads** (code comment in `run()`: "The scan is left
  running, as it always has been during uploads"). On many adapters an active scan
  competes with the connection for radio time. It is constant across all uploads, so the
  logs can't test it. It is a candidate mechanism for lost writes or notifications, and
  cheap to change (proposal B below).
- **Non-PROGRESS notifications are discarded unlogged.** `wait_for_progress` silently
  ignores any notification that isn't exactly PROGRESS or COMPLETE. If the keyboard
  reports anything at the moment it stalls (an error, a different status), we throw it
  away. That is the cheapest unknown to close (proposal A).

---

## Task 3 — proposed experiments, ranked by information per minute of Michi's time

Base rates to plan against: a first upload on a connection fails about **1 in 3**
(9/24); a later upload about **1 in 30** (3/91). Each failure costs a power-off (opening
the keyboard), occasionally two.

### 1. Retry-first recovery on the halts that happen anyway (the manual version of a bounded auto-retry)

- **Hypothesis (I):** after a *sudden* code 20, a retry with no power-off succeeds as
  often as any first upload (~62 %).
- **Do:** at each natural halt, note the signature (`TIMING`/`SLOW` lines). If sudden:
  wait ~2 minutes, START_STACK, no power-off; if that fails, once more; power off only
  after two failed retries. If slowing: power off as now (and note whether that cleared
  it). Record each attempt: time since the halt, result.
- **Repetitions:** **6–8 halts.** Under I, about 6 in 10 first retries complete and
  about 85 % of halts end without a power-off. If S were true for sudden failures,
  first retries would mostly fail. 2 of 4 so far (13:42, 14:11 against 22:46, 14:09).
  Confirms: ≥ 4 of 7 first retries complete. Refutes: ≤ 1 of 7.
- **Cost / risk:** saves power-offs if I holds; costs at most one extra failed upload
  (~10 s) before the power-off if it doesn't. The risk named in bug A, a retry breaking
  the LCD, hasn't occurred in the 6 logged retries without a proper reset. If it does,
  stop and power off.
- **Outcome feeds the policy decision** (main finding above). Today's four attempts
  already count as the first data points.

### 2. Notification and scan logging (code first, then normal use; zero active minutes)

- **Hypothesis:** at a sudden stall, either the keyboard sends something other than
  PROGRESS (which we currently discard), or it sends nothing at all.
- **Do:** proposal A (and optionally B) below on a dev branch, then normal use until the
  next two or three failures.
- **Repetitions:** 2–3 failures, which at the current rate means one or two evenings.
- **Result:** any unexpected notification near the stall is a lead in itself. Silence
  after a block that was fully sent points to a lost write or a stuck keyboard, which is
  what experiment 3 tests.
- **Risk:** none beyond normal use; logging only.

### 3. Intra-block pace 2 ms vs 0 ms, on first uploads

- **Hypothesis:** sudden stalls are lost Write Commands under back-to-back writes; a
  2 ms gap prevents them.
- **Do:** where the signal is (first uploads), deliberately: STOP_STACK, START_STACK (no
  reset), wait for the first upload, note the result, repeat. Run 6 restarts at 2 ms,
  then compare with 6 at 0 ms (or use the existing first-upload record as the 0 ms
  baseline). Switching needs the supervisor's `'--intra-block-ms','0'` changed to `'2'`
  (Michi's call, on a dev branch or by hand). Then optionally leave 2 ms in normal use
  for a few days.
- **Repetitions:** **10–12 first uploads at 2 ms.** If none fail, chance alone at the
  current 1-in-3 rate would give that about 1 % of the time (0.65¹⁰). 2 or more failures
  in 12 refutes a strong effect. For later uploads (1 in 30) the test would need about
  100 uploads to say anything, which normal use provides in a few days.
- **Cost / risk:** each restart costs 20–70 s of invisibility (the known code-10 period)
  plus ~9 s; each failure costs a power-off, and at the base rate about 4 of 12 would
  fail. Uploads get about 2.5 s slower. Bug A's halt protects against a retry loop.

### 4. First upload after a cold boot (only if 1–3 leave it open)

- **Hypothesis:** a keyboard idle for a long time (38 h before 13:20 today) is more
  fragile on its first upload.
- **Do:** note the outcome of the first upload after each Windows boot or resume, with the
  idle time. No extra action.
- **Repetitions:** whatever accumulates; n = 2 so far (17:52 resume worked, 13:20 boot
  failed). Needs ~6–8 before it says anything against the 1-in-3 base rate.
- **Risk:** none beyond normal use.

**Not proposed:** iPad / 2.4 GHz interference. Michi's account doesn't link today's
failures to it, and RSSI doesn't separate outcomes. Cheap to add as a note during
experiment 1 if the iPad is in use.

### Code proposals (text only; no uploader change in this run)

- **A. Log every notification that isn't PROGRESS** during an upload (value in hex,
  characteristic, time since the last write), and on a failure the number of writes
  actually issued for the stalled block and how long since the last one. Logging only;
  no change to what is sent.
- **B. Stop the scan once the device is selected** (`adapter.stop_scan()`). It's a
  behaviour change in the radio's workload, so it belongs in an experiment of its own,
  after A. It shouldn't be mixed with the pace test.
- **C. Halt rule:** a bounded retry (sudden failures only, 60–120 s delay, at most two
  retries, then halt; slowing failures halt at once) as described under the main
  finding. Not before experiment 1 has 6–8 halts, and only on Michi's decision.

---

## Limits

- The battery and start-kind columns depend on the context file. Where §7.1 or Michi
  didn't record something, it is blank, not guessed.
- The fallback install's logs can't be tabulated (no timestamps, overwritten per run).
- The 5-block averages before 2026-10-16 20:51 hide single slow blocks.
- Read up to 14:15 on 2026-10-18 (the 14:11 upload). Rerun the parser for anything
  later.
