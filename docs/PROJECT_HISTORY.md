# TIGA 75 / Zoom75 — Project History and Decisions

Distilled from the 631-page ChatGPT development archive (consolidated 2026-10-15).
This file records **why** the project is built the way it is: protocol facts established
from real captures, decisions and their reasons, dead ends that should not be re-walked,
and the open questions at the current frontier.

Code is the source of truth for *what* exists. This file is the source of truth for *why*.

**Collaboration status:** this is a solo project. Two outside contacts — a community tester
with a second TIGA, and the upstream `meletricks` author — contributed observations during the
period the archive covers, but are no longer reachable. Their reports are kept below because
they are useful leads, but each is marked unverified, and **none of them blocks any remaining
work.** See §5.4 for why no permission is outstanding.

**Hardware:** Meletrix / Wuque Studio Zoom75 TIGA, 320×172 LCD
**Public repo:** `github.com/MichiG21/tiga-zoom75-windows-media-display`

---

## 1. Foundational decisions

### 1.1 Do not use `meletrixid.sys` — DECIDED, do not revisit casually

Norton flagged `C:\Program Files (x86)\MeletrixID\meletrixid.sys` as a vulnerable kernel
driver and blocked it. Consequence: CPU / Mbps / fan-RPM values stopped appearing on the LCD.

**Decision: leave it blocked.** The driver is MeletrixID's low-level hardware-sensor reader.
This project needs media metadata, not PC sensor telemetry, so nothing of value is lost. A
few CPU numbers do not justify permitting a kernel driver an AV vendor considers exploitable.

This held up: everything the project needs turned out to be ordinary HID/BLE traffic that
works with the driver blocked.

### 1.2 Division of labour in the official tooling — established early, saved a lot of time

| Tool | What it actually does |
|---|---|
| MeletrixID (Win/Mac) | **Telemetry only.** Feeds CPU/GPU/network values to the LCD. Not an image uploader. |
| PocketWuque (iOS/Android) | **Images, GIFs, weather, time.** This is the one that uploads media. |

Getting this wrong cost an early detour: USB captures of MeletrixID were taken hoping to see
the image protocol, and there was nothing to find — only 32-byte telemetry packets. **If you
want the media protocol, capture PocketWuque, not MeletrixID.**

### 1.3 Windows GSMTC instead of the Spotify Web API — DECIDED

The first working version authenticated to Spotify with OAuth PKCE and polled the
currently-playing endpoint every ~5 s. It was replaced with Windows' GSMTC (Global System
Media Transport Controls) media session.

Reasons:
- Works with **any** app that exposes a Windows media session — browsers, YouTube, etc. — not
  just Spotify.
- No Spotify developer account, client ID, OAuth flow, or cloud dependency for the user.
- Windows has a "current session" concept and raises events when it changes, so switching
  from Spotify to a YouTube tab is handled automatically rather than by polling one service.

Known rough edges: `album` is usually empty for YouTube (fine); the browser source is
sometimes an opaque ID rather than a friendly name.

---

## 2. USB / HID protocol — what was established

Device: **VID `1EA7`, PID `CEDD`**. HID-style interfaces, OUT endpoint `0x02`,
IN endpoint `0x82`, 32-byte packets. MeletrixID emits telemetry roughly every 2 seconds.

### 2.1 Command bytes seen in real captures

| Command | Meaning |
|---|---|
| `0x38` | date / time |
| `0xFD` | telemetry theme (template switch) |
| `0xFE` | weather |
| `0xFF` | CPU / GPU / RPM / network telemetry (SYSINFO) |
| `0xFC` | media / image upload |
| `0x39` | screen navigation |

### 2.2 Packet framing — and the bug that broke `zoom-sync`

A 32-byte raw report is wrapped into a 33-byte HID buffer with a leading `0x00` report ID.

Two different checksum conventions exist, and conflating them is what broke the first
attempts:

- **SYSINFO (`0xFF`)**: the byte immediately after the payload is a **constant `0xFF`**, not a
  computed value. Verified across 16 captured packets — it never varied. Payload length
  field read `0x0B` (11 data bytes), then `FF`.
- **Theme / Weather / Time / Screen**: these *do* use a computed checksum,
  `sum(command + length + payload) XOR 0xFF`. Verified byte-exact against official web-console
  captures for four independent command types.

The experimental `feat/zoom75-tiga` branch of `zoom-sync` got this wrong in two compounding ways:

1. It treated the constant `FF` as a computed checksum.
2. When building the 33-byte HID buffer it copied `packet[..31]`, **dropping raw byte 31**.

For short commands byte 31 is only padding, so nothing visibly broke. For a *full* 16-byte
image chunk the checksum lands exactly on byte 31 — so image uploads, and only image uploads,
lost their checksum. That is the transport-level cause of the original white screen.

### 2.3 Pixel encoding confusion in `zoom-sync`

The TIGA board file claimed RGB565, but the shared encoder (inherited from Zoom65) emitted
`[hi, lo, 0xFF]` — RGB565 plus an 8-bit alpha byte, 3 bytes/pixel:

- `zoom-sync` sent: 320 × 172 × 3 = **165,120 bytes**
- True RGB565 frame: 320 × 172 × 2 = **110,080 bytes**

**Patching this to 2 bytes/pixel alone did NOT fix the white screen.** That result was
valuable: it ruled out pixel encoding as the primary fault and pointed at the framing bug above.

### 2.4 Why the USB media path was abandoned anyway

After the HID framing was corrected, a full live upload produced:

```
10,320 / 10,320 image chunks  → all ACKed
end packet                    → ACKed
```

…and the LCD still rebooted into a broken state. **The transport was correct; the file inside
it was wrong.** This is the single most useful diagnostic result in the whole USB phase: it
cleanly separated "can we deliver bytes?" from "are they the right bytes?"

### 2.5 The hidden web uploader — a FALSE LEAD, do not return to it

Meletrix's official web console bundle contains an image uploader that is **hard-coded with
`display:none`**. Its code uses a 452-byte header, `0xFC`, and RGB565 **little-endian**.

It was reproduced faithfully — and the resulting media slot crashed the LCD. Meletrix's *own*
hidden uploader also stalled at page 341 during a live test.

**Conclusion: this is old/dev/unfinished code left in the bundle, not the current supported
format.** The fact that the controls had to be manually un-hidden supports that. The
little-endian pixel order from this path is wrong; the working path uses big-endian (§3.2).

---

## 3. BLE protocol — the path that works

Established by capturing a **working** PocketWuque upload from an iPhone: enable Apple's
Bluetooth logging profile → perform the upload → `sysdiagnose` → extract the `.pklg`.

This was the decisive move of the project. Everything before it was inference; this gave a
known-good reference stream to reconstruct byte-for-byte.

### 3.1 GATT structure

```
Service   1f40eaf8-aab4-14a3-f1ba-f61f35cddbaa
  0001    1f400001-...  control write    handle 0x0012
  0002    1f400002-...  status notify    handle 0x0014
  0003    1f400003-...  bulk write       handle 0x0017
  0004    1f400004-...  second notify    handle 0x0019
```

Writes use **ATT Write Command `0x52`**. ATT MTU is effectively 185, giving 182 bytes of
characteristic value per write: **181 data bytes + 1 XOR checksum** over those 181. This held
for all 2,478 writes in the capture without exception.

### 3.2 Stream layout (the known-good red/green 2-frame container)

```
220,649 bytes total
├─    8 B   0x88 bulk envelope
├─   17 B   upload preamble
└─ 220,624 B  DLX file
   ├─     8 B   00 44 4C 58 FC FF 00 00      ("DLX" in ASCII)
   ├─   452 B   media / animation header
   ├─ 110,080 B  frame 1  (red   = F8 00)
   ├─ 110,080 B  frame 2  (green = 07 E0)
   └─     4 B   FC FF 00 00  trailer
```

- Pixel format: **RGB565 BIG-endian.** (Red `F8 00`, green `07 E0`.)
- Frame offsets within the stream: **485** and **110,565**.
- Per upload: **1,239 GATT bulk writes**.
- Completion notification: `88 00 00 00 00 00 03 0c 08 06 02`.

PocketWuque deliberately cuts BLE writes so that **no write crosses a 4096-byte boundary of
the DLX file**. That explains the occasional 115 / 140 / 98-byte packets among the regular
182-byte writes — they are not noise.

### 3.3 Flow control — the big speed win

Fixed-sleep pacing was the wrong mechanism. Evidence: at 16 ms pacing the final status was
`53`; at 14 ms it was `13` even though all 1,239 writes had been sent. **53 is exactly the
number of complete 4096-byte blocks in the stream.** Each PROGRESS notification is effectively
a "4K block processed" signal — at 14 ms the sender was simply outrunning the keyboard.

Correct mechanism:

```
write bulk chunks as fast as BLE allows
        ↓
4096 bytes sent
        ↓
wait for PROGRESS notification from the TIGA
        ↓
continue
```

**Result: full-frame upload dropped from ~39 s to ~7.7–8.5 s.** For reference, PocketWuque on
the iPhone took ~23.6 s for the same file, so the hardware was never the limit.

### 3.4 Known remaining inefficiency

The production stream still carries **two identical frames**, because it was derived from a
2-frame GIF capture and only the pixels are replaced. A static image needs one.

A second developer working on the same hardware measured: ~30 s → ~15 s by removing the
duplicate frame → ~11 s by using a proper static-image command recovered by decompiling the
PocketWuque Android app with JADX. Halving the payload naively suggests ~4 s here, though
handshake overhead means it will not scale perfectly.

**This is the most obvious unclaimed speed win, and both halves are solo work.** Removing the
duplicate frame is a local change to code already in this repo. The static-image command was
never shared, but it is independently recoverable: decompile the PocketWuque APK with JADX and
look for the static-image command and its GATT sequence.

---

## 4. The progress bar — the central unsolved problem

The original goal was cover art + title + artist + a **live progress bar**. Everything else
works; the progress bar does not, and the reason is architectural.

Current architecture renders a whole 320×172 image on the PC and ships it over BLE. Moving a
progress bar four pixels therefore means regenerating and re-uploading ~220 KB. At ~8 s per
upload, that is not a progress bar.

Three avenues were explored and closed:

**`0xFC` partial / in-place page writes — ABANDONED.** Addressed page writes into an already
displayed DLX image do not behave like "modify a few pixels in place". Committing an
incomplete medium can leave the custom slot invalid or white. Verdict recorded at the time:
*this was the wrong layer for live updates.* Do not retry without new information.

**`0xFF` SYSINFO telemetry — works, but wrong shape.** Payload structure, all big-endian:

```
byte 0      unknown flag
bytes 1-2   CPU
bytes 3-4   GPU
bytes 5-6   unknown
bytes 7-8   RPM
bytes 9-10  Download / Mbps
```

This channel runs at 1 Hz with essentially no latency, does **not** open a media session, and
leaves the custom image slot completely undisturbed. It proved the MCU can receive and render
live data. But it is a *data-bound telemetry renderer*, not a general pixel or overlay
command — the values only appear on the built-in telemetry screen.

**`0xFD` theme switching — informative, but a dead end.** It switches between three
firmware-built telemetry layouts. The packet carries essentially only a theme ID — no titles,
coordinates, fonts, or icons. So the TIGA definitely has a real runtime UI renderer, but its
templates are fixed in firmware and cannot be repurposed into a custom screen.

**Net finding:** the hardware can do what we want; the stock protocol will not let us ask for it.

### 4.1 The lion/tiger is not our bug

The roaring lion animation shown during transfers is triggered by **Control 2** — it is the
stock firmware's upload state. The Wuque app shows it too. On the stock upload path it can be
made *shorter* by making transfers faster, but it cannot be removed. Stop treating it as a
renderer bug.

---

## 5. Current direction: `meletricks` and the mailbox

`github.com/athdesk/meletricks` — author **hdesk / athdesk**, **GPL-2.0**.

It solves the exact thing that was missing: **code execution on the LCD MCU.** The LCD MCU
exposes BLE endpoints for memory read/write:

```
write characteristic  02f00000-...-ff01
read  characteristic  02f00000-...-ff00
560-byte chunks
```

(Full service UUID: `02f00000-0000-0000-0000-00000000fe00`; see §6 for the characteristic map.)

It loads custom code into RAM, hooks a function the stock firmware runs periodically, and runs
its own code **without reflashing**. On reboot the injected code is gone — which is precisely
why this is a safe thing to experiment with.

### 5.1 Key addresses (Zoom75 TIGA, current upstream)

```
FW_TICK_HOOK       0x110014F4
trampoline         0x11008A60
ELF load base      0x220E0000
sentinel area      near 0x11009000
```

**The hook address is NOT where the application code lives.** The hook is only an interception
point that branches to the trampoline, which calls the relocated application.

### 5.2 Target architecture

```
Windows GSMTC
    ↓  small BLE state messages / RAM writes
shared RAM "mailbox"
    ↓
injected code on the LCD MCU renders locally
(via the firmware's own framebuffer draw function; stock GUI can be paused)
```

Instead of "here is another 320×172 frame with the bar 4 px further along", Windows would
write something like `{state: PLAYING, position: 132000, duration: 252000}` and the MCU would
draw the bar itself. Album artwork would transfer only on track change.

### 5.3 The F2 / F4 question — UNRESOLVED, do not guess

A second unit was reported to run more reliably with the hook at `0x110014F2` than upstream's
`0x110014F4`. That is a difference of two bytes — **one Thumb halfword**. Since the loader
steals and replays the instructions it displaces, moving the entry point by 2 bytes changes
which instructions get relocated.

This was never corroborated and the underlying firmware bytes were never shared, so treat it
as a hint, not a finding. **Do not adopt F2 on that basis.** Dump and disassemble the bytes
around `0x110014F0` on this unit and determine what each address actually lands on — that was
always the plan, and it is entirely solo work. (§6 records what has already been established
about F4.)

Upstream also warned that the TIGA port is not well tested.

### 5.4 Licensing — settled, and requires nobody's permission

`meletricks` is GPL-2.0. This project was originally published MIT.

**The GPL already grants the right to fork, modify and distribute.** The long licensing
discussion in the archive was about obtaining *additional* permission — specifically, whether
meletricks code could be used while keeping this project MIT, which the GPL does not allow.
Once the decision was made to go GPL, that question became moot. **No permission is
outstanding and none needs to be sought.**

**Decision: license the derived TIGA-side component GPL-2.0**, matching meletricks exactly.
The upstream author did say GPLv3 would be acceptable downstream, but whether meletricks is
GPL-2.0-*only* or GPL-2.0-*or-later* was never clarified, and GPL-2.0-only code cannot be
cleanly combined with GPLv3. Matching GPL-2.0 sidesteps the question entirely and depends on
no one.

Credit the upstream author and link back to meletricks in the README — the GPL expects this
regardless.

Practical approach:

- Work against the current upstream repo as it stands. A separate SDK-only repo was discussed
  upstream but never materialised; **nothing was ever gated on it.**
- Keep commits narrow and scoped to `sdk/`, `tools/TweakLoader/`, `sdk/boards/zoom75tiga/`.
  Do not reorganise upstream's tree — narrow commits stay easy to rebase if upstream ever moves.
- Bring the SDK in as a submodule rather than vendoring a divergent copy.

*(Licensing summary, not legal advice.)*

---

## 6. Runtime-lab status (most recent work)

Experimental workspace: `C:\Users\Regal\source\tiga-runtime-lab`, deliberately isolated from
the known-good installation. Both targets build; 21 offline tests pass. **No injection has
occurred.**

Three blockers stand between here and a first injection:

**Windows GATT access — DIAGNOSED, workaround known.** The memory service is
`02f00000-0000-0000-0000-00000000fe00`. With the production media stack running, opening it
with `SharedReadAndWrite` returned `GattOpenStatus.SHARING_VIOLATION`. **With the media stack
stopped, the same open succeeded** and both cached and uncached characteristic enumeration
returned identical lists.

| Check | Stack running | Stack stopped |
|---|---|---|
| Device / service permission | ALLOWED | ALLOWED |
| Shared service open | `SHARING_VIOLATION` | **SUCCESS** |
| Characteristic metadata | not attempted | cached + uncached SUCCESS |

This strongly implicates service-handle ownership associated with the media stack, though it
does not identify the owning process or the exact Windows/btleplug handle-sharing mechanism —
and the two probes were not an otherwise identical A/B test.

Characteristic properties found (Windows flags: Read=2, WriteWithoutResponse=4, Write=8,
Notify=16):

| UUID suffix | Properties | Capability |
|---|---:|---|
| `FF00` | 2 | Read |
| `FF01` | 12 | WriteWithoutResponse, Write |
| `FF02` | 18 | Read, Notify |
| `FF03` | 2 | Read |

`FF00` and `FF01` match what upstream TweakLoader expects. **Metadata compatibility only** —
it validates nothing about the MCU memory-command protocol, addresses, instruction boundaries,
or RAM ownership. No firmware bytes were obtained.

Practical consequence: **the production uploader must be stopped before memory inspection.**

**The trampoline reservation overlaps live data.** `0x11008A90` was observed changing from
`0B` to `0A` during a capture. The upstream trampoline reservation must **not** be treated as
free allocation, nor restored wholesale from a stale snapshot. No candidate region has been
certified safe. Stability is not ownership; zeros and apparent gaps prove nothing.

**Possible reentrancy.** The hook function is called from **both** a repeating loop **and** a
UART-handler path. A load/add/store counter increment is not atomic, so nested execution could
lose increments. This needs resolving before injection — and interrupt masking must not be
added without a separate reviewed design.

Also noted: the `F4` literal points to `0x110038D4`, which constrains how the displaced
instruction must be reproduced.

---

## 7. Known bugs in the published release

From an independent audit of the public repo (HEAD `a61af7c`; tags `v0.1.0`,
`v0.2.0-beta.1`). The architecture and frame format were judged sound — these are reliability
and compatibility gaps, not encoder or packet-layout faults.

**A. BLE recovery contradicts the uploader's own requirement — highest priority.**
The uploader exits on a progress/completion timeout and explicitly says to power-cycle before
retrying. **Both supervisors restart it automatically after 8 seconds.** An interrupted
transfer can therefore enter a retry loop against a keyboard still in an incomplete transfer
state.
*Policy revised 2026-10-18 (Michi's decision, `feature/sudden-retry`):* the fix (#4) halts
on every code 20. Since then, a code 20 with the **sudden** signature is retried
automatically after 120 s, at most 3 times per incident, then halts as before; slowing
failures and code 21 still halt at once. Evidence: §7.1 sixth part and
`docs/investigations/UPLOAD_FAILURES_2026-10-18.md`. That is a bounded retry, not the
unbounded 8-second loop this bug was about.

**B. Already-playing media can stay on standby indefinitely — reproduced.**
The freshness gate releases only on changed media identity, a non-playing→playing transition,
or ≥1.5 s of reported timeline advancement. If playback is already active when the renderer
starts and the provider reports a fixed position, none of those happens. This is the direct
explanation for *"it starts working after I pause or change tracks."* The stale-media
protection is intentional; this false negative is its cost.

**C. Reissuing a command can be ignored — reproduced.** Commands are identified by their text,
not by a new write or command ID. Clicking Standby twice writes identical text and is ignored
the second time, while the GUI still reports "requested".

**D. "Standby + Shut Down" does not confirm completion.** It waits a fixed 12 s, then shuts
down, without verifying the renderer processed the command or BLE finished.

**E. Development startup breaks on paths containing spaces.** The dev supervisor passes the
Python script path through an unquoted `Start-Process -ArgumentList`. The packaged renderer
uses a different launch path and is unaffected.

**F. Failure logging is insufficient for intermittent reports.** Child output files are reused
on restart, so earlier failure output is destroyed; Python output is redirected without
unbuffered logging; there is no shared frame identifier linking render → upload start →
completion. A stack can look healthy while failing to update the keyboard.

**G. Smaller defects.** A long unbroken title produced a 720-pixel line in a 174-pixel area;
album text can overlap longer artist text; artwork change is detected by byte count rather than
content; repeated media-query failures bypass idle handling and leave the last media up
indefinitely; the active stream is tracked by Git but overwritten during normal source
execution; the production Rust app has **no committed lockfile**.

**Confirmed on hardware, 2026-10-15: device selection by name.** The uploader took the
first peripheral whose name contained `zoom75`. On this PC **two** devices match, and only
one exposes the media service (issue #14). Every connection attempt that evening:

| Local time | Address | Result |
|---|---|---|
| 21:05:46 | `07:01:88:B3:66:1B` | connected, upload started (then interrupted, see §7.1) |
| 21:08:53 | `1B:A3:A9:10:13:5D` | connected, `control char missing` (exit 11) |
| 21:09:13 | `1B:A3:A9:10:13:5D` | connected, `control char missing` (exit 11) |
| 21:09:37 | `1B:A3:A9:10:13:5D` | connected, `control char missing` (exit 11) |
| 21:15:15 | `1B:A3:A9:10:13:5D` | connected, `control char missing` (exit 11) |
| 21:15:29 | `07:01:88:B3:66:1B` | upload completed in 8.71 s |
| 21:35:54 | `07:01:88:B3:66:1B` | upload completed in 8.42 s |

Every `control char missing` was `1B:A3:A9:10:13:5D`; every upload was `07:01:88:B3:66:1B`.
Both addresses were stable across two keyboard power cycles. **Identified later that
evening:** `1B:A3:A9:10:13:5D` advertises as `"Zoom75 Tiga BT1"`, the display as
`"Zoom75 Tiga"` (§7.1, second part). BT1 is almost certainly the keyboard's Bluetooth
*typing* connection, pairing slot 1. What services it exposes is still not logged: on the
one run that probed it, the connect failed. Fixed on `fix/device-selection` by qualifying
a device by service rather than name, and by trying the exact display name first; the
service check ran on hardware 22:59–23:13 (§7.1).

**Suspected but unproven.** These are concrete implementation limitations, but which one
causes any given field failure was never established: the uploader selects only the first
Bluetooth adapter, scans for a fixed 5 s, always uses 182-byte writes without checking
usable MTU, uses fixed timeouts, and does not actively recover a dropped connection while
the stream is unchanged. Also: only Windows' *current*
session is queried — the helper does not enumerate sessions to find another actively playing
source, and a thumbnail retrieval failure fails the whole snapshot.

Confirming any of these needs a failing machine, and there is no outside tester. **Treat them
as hardening work to do blind, not as a diagnosis to wait on.** A, B, C and E are all
reproducible or visible by inspection on one machine, so there is plenty to fix without field
reports.

**Bug F is now higher priority than its severity suggests.** With no second tester, local logs
are the only diagnostic channel left — and they currently destroy themselves on restart.

**Recommended repair order:** diagnostics first (persistent timestamped logs, adapter/device
details, frame IDs, freshness decisions, last acknowledged block — i.e. fix F) → fix transfer
recovery (A) → fix freshness and command handling (B, C) → replace the fixed shutdown wait
with acknowledgement (D) → harden startup/install (E) → improve discovery and connection
handling → lock dependencies → then rendering defects (G).

### 7.1 First hardware session of the fixed build, 2026-10-15

Source: `logs\ble.log`, `renderer.log` and `startup_supervisor.log` from the
`tiga-fresh-test` install (a clone of `main` after the bug A merge), 21:05–21:49 local
time, plus Michi's observations at the keyboard. Times below are local (UTC+2).

**Measured uploads.** Stream 220,649 bytes / 1,239 writes / 53 PROGRESS blocks,
intra-block pace 0 ms. 19 uploads completed, between **8.42 s and 10.01 s**. The two
quoted that evening were 8.71 s and 9.85 s. The 8.71 s run was linear at about
**0.154 s per block** (block 53 ACKed at 8.15 s), then ~0.56 s waiting for completion.
The two fastest (8.71 s, 8.42 s) were each the first upload on a new connection; the
other 17, later uploads on the same connection, took 9.49–10.01 s. That is two samples
and may be coincidence; it is recorded, not explained. *(Later the same night: six of six
first-on-connection uploads were the fast ones. See "Every upload of 2026-10-15" below.)*

**Interrupted transfers, cause unknown.** Two, both PROGRESS timeouts after CONTROL2:

| Time | Upload on this connection | Failed at | After |
|---|---|---|---|
| 21:05:49 | 1st | block 5/53 (last ACKed 4) | 3.68 s |
| 21:39:48 | 2nd (previous one completed at 21:36:05) | block 3/53 (last ACKed 2) | 4.21 s |

The keyboard stopped acknowledging. Intra-block pace was 0 ms, the same as every
successful upload, so pace is not the obvious cause. The first may be explained by the
keyboard being in a bad state after the previous stack was stopped; that is a guess. The
second came 3.7 minutes after a clean upload on the same connection, so that explanation
does not obviously cover it. **Cause unknown for both.** The second was not part of the
evening's narrative at the time of writing; its halt (`uploader_halted.json`, 21:39:54)
was still in place when the logs were read.

**What happened after the first one — read carefully.** Bug A's halt fired correctly
(21:05:53, `session-open`).

- **First recovery attempt: unplugging USB for about 5 seconds. It did not reset the
  keyboard.** The stock roar stayed visible. **Unplugging USB is not a power cycle:** the
  TIGA has a battery and keeps running without USB power. After this and `START_STACK`
  (halt cleared 21:08:48), three attempts failed and the supervisor halted as `gave-up`
  at 21:09:40. All three were on the **wrong device** (`1B:A3:A9:10:13:5D`), so the logs
  alone say nothing about the keyboard's state. The evidence that it was not reset is
  the roar staying on screen.
- **Second attempt: disconnecting the battery. That did reset it.** After it (supervisor
  start 21:15:10) uploads worked, though even then the first attempt hit the wrong device.

**Correction (written later the same night): the TIGA has no power switch.** Commit
`583faf9` told users to "use the power switch", and an earlier version of this paragraph
asked whether the power switch alone would be enough. Neither makes sense: the only way to
reset the keyboard is to open the case and disconnect the battery from the PCB. How long
it must stay disconnected is recorded in the second part below.

**Verified on hardware:**

- **#4 (bug A):** a *real* interrupted transfer halted, was not retried, wrote
  `uploader_halted.json`, kept the renderer running, and recovered via `START_STACK`.
  The killed-process / session-marker path was **not** tested.
- **#6 (bug C):** repeated identical commands are acted on. Verified by writing the
  command file by hand (`standby test1`, then `standby test2`; renderer log 21:30:31 and
  21:31:38). The GUI path was blocked: the display-control utility reported the request
  but no command file appeared (#15).
- **#9 (bug F):** logs keep history across restarts, with run and frame IDs.
- **#10 (bug G):** a 34-character unbroken title wraps instead of running off the screen.
- **The Unicode crash fix:** a Japanese title (`トーキョーレギー`) rendered, as empty
  boxes, without crashing the renderer. The boxes are a font-fallback gap, not a crash (fixed later that night, §7.2).

**Not verified on hardware:**

- **#5 (bug B):** a restart with Spotify already playing released standby in 2 s through
  the *old* "timeline advancing" path. The new 15-second rule never fired. The original
  bug was only ever reproduced synthetically, and **no real-world provider is known to
  report a frozen timeline.** Fixed in code, unproven in the field.
- **#7 (bug D):** partly exercised. Standby + Shut Down did not shut down on a timer; it
  waited for the renderer's ack, got none, and asked. But the command never reached the
  renderer (#15), so the delivery-ack stage and the actual shutdown were never reached.
- **#8 (bug E):** paths with spaces. Not tried.

#### Second part: battery resets and `fix/device-selection` on hardware, 22:44–23:26

Sources: `tiga-fresh-test\logs\` (the live install, `main`, 2500 ms block timeout) and
`tiga-dev\logs\` (the scheduled task pointed at `fix/device-selection` from 22:59 to
23:13), plus Michi's account of what he did at the keyboard. `ble.log` stamps events in
UTC; times here are local (UTC+2).

**The second "zoom75" device is identified.** At 23:06 the scan logged
`CANDIDATE name="Zoom75 Tiga BT1" address=1B:A3:A9:10:13:5D` and
`CANDIDATE name="Zoom75 Tiga" address=07:01:88:B3:66:1B`. BT1 is almost certainly the
keyboard's own Bluetooth typing connection, pairing slot 1. The old "name contains
zoom75" match caught both. **Hypothesis, unconfirmed:** this may explain the community
tester's failures, since he used the TIGA as a Bluetooth keyboard and would therefore
have had the BT1 device paired.

**Device selection by service works on hardware (#14).**

- *Rejection path, 23:06.* BT1 was seen first, the connect failed
  (`REJECTED … connect failed: Not connected`) about 11 s later, then `07:01…` was
  selected and the upload completed in 8.60 s. Because the connect failed, the
  `exposes …` line that would list BT1's services was never produced.
- *Address cache, 23:09.* The uploader tried `07:01…` first (`last device that
  completed an upload`) and selected it with no detour.
- *Since then:* candidates are ranked by name: cached address, then exactly
  `Zoom75 Tiga`, then other zoom75 names, then `… BT<n>` last (not skipped; a firmware
  variant might name things differently, and the service check stays the real test).
  That removes the 11 s detour. Offline tests only.

**The keyboard is invisible for a while after a stack restart (no battery reset).**
Seen three times, each time with the scan finding no candidate at all (code 10) and the
supervisor's retry recovering without a halt:

| Stack start | Code-10 scans | Keyboard first seen | After start |
|---|---|---|---|
| 21:35:35 | 1 | 21:35:54 | ~19 s |
| 23:09:12 | 2 | 23:09:50 (found within 1 s of that scan starting) | ~38 s |
| 23:15:15 | 2 (this run's first scan began only at 23:15:41) | 23:16:22 | ~67 s |

The most likely explanation is that the keyboard is still holding the previous
connection (the uploader just killed by STOP_STACK) and isn't advertising until that link
times out. **Unconfirmed.** For it: the three starts that followed a battery disconnect
(22:44, 22:46, 22:49) all found the keyboard within 5 s. Against, or at least not
explained: the 22:59 start, a few minutes after the live stack's last upload (22:57),
found it within 2 s. When the live stack was stopped before 22:59 isn't logged.
Since then, a scan that has seen nothing at all keeps going up to 15 s instead of 5 s.
In all three cases above that would most likely have turned two code-10 failures into
one, not zero; absorbing 23:09 or 23:15 completely would need a 30–60 s first scan.
Polish only: the retry already recovers on its own.

**There is no power switch, and the battery-disconnect duration matters.** The TIGA can
only be reset by opening the case and disconnecting the battery from the PCB. One
observation each:

- a **10–15 s** disconnect before 22:44 did **not** clear the keyboard: 22:44 and 22:46
  both stalled at block 11/53
- a **30 s** disconnect before 22:49 did: 22:49 completed normally in 8.70 s
- *(added 2026-10-16)* a **25 s** disconnect **without key presses** did **not** (21:09,
  fifth part below); 40 s and about a minute with key presses did
- Michi's older notes say 30–60 s, pressing a few keys while disconnected to drain the
  remaining charge

So **30 s is a working minimum, not a proven threshold.** Every recovery instruction now
says: disconnect the battery for at least 30 s, pressing a few keys.
*Correction 2026-10-18 (sixth part):* a ~1.5 min full power-off was followed by a failed
first upload (13:41), and a retry with no power-off at all worked (13:42). The duration
record is weaker evidence than it looked: a first upload fails about 1 time in 3
regardless.

**Two failure signatures, both on the correct device (`07:01…`):**

- **Slowing** — 22:44 and 22:46. Block 5 was ACKed at 2.35 s and 2.97 s (healthy: 0.7–1.6 s),
  block 10 at 12.02 s and 11.58 s (healthy: 1.5–2.4 s), then no PROGRESS for block 11
  within 2.5 s. The log prints only every fifth block, so "each ACK slower than the last"
  can't be read directly; but each of blocks 6–10 had to arrive within the 2.5 s
  timeout, and together they took about 9.6 s, so **at least four of those five took
  roughly 1.8–2.5 s each** (healthy: ~0.15 s). Sustained slowness, not one hiccup.
  **Block 11 both times, with different images** (CRC32 `b77aa6c0` at 22:44, `ae3a68d2` at
  22:46), so it is not content-related. This followed the 10–15 s battery disconnect.
- **Sudden** — 23:00 (first `tiga-dev` upload). A healthy pace to block 40 (6.12 s,
  the fast first-on-connection pace), then nothing for block 41.

**The block-timeout experiment — an unverified mitigation, not a fix.** At 23:03 the dev
supervisor was edited by hand to pass `--block-timeout-ms 10000` (default then 2500).
Reasoning: when one block's PROGRESS is late, the uploader abandons the transfer, and
abandoning it is what leaves a half-finished session that needs the battery
disconnected; if the keyboard is sometimes merely slow on one block, waiting longer
avoids that. **Evidence: inconclusive.** All five interrupted transfers of the day (21:05,
21:39, 22:44, 22:46, 23:00) ran with 2500 ms. The five uploads that ran with 10000 ms
(23:06–23:13) all completed, but none came near either limit and they followed a 30 s
battery reset; the live install then completed eight more after 23:15 with 2500 ms. The
longer wait is now the uploader's own default (so the packaged supervisor gets it too),
and `ble.log` prints the value in effect. Its cost is only that a truly stuck keyboard
is detected about 7.5 s later. Open question it does not answer: in the slowing
signature, blocks were taking ~2 s each; with 10 s the 22:44 transfer might have
crawled to completion, or stalled anyway at block 11, only later.
*Update 2026-10-16:* answered by the third part below. A slowing run under 10000 ms
stalled anyway, at block 41 after 78 s, and none of 27 completed uploads at 10000 ms
needed more than 2.5 s for any block. The longer wait has rescued nothing so far. The
default went back to 2500 ms the same day.

**Upload reliability on `fix/device-selection`:** one failure (23:00, the sudden
signature, before a 30 s battery reset), then **five completed uploads in a row**,
8.60–9.81 s, all at a healthy pace.

**Verified on hardware in this part:** #14 (service check, BT1 rejected, address cache);
the code-10 retry recovering from a temporarily invisible keyboard without a halt; bug
A's halt (three more real code-20 halts, 22:44, 22:46 and 23:00, each recovered via
START_STACK after a battery disconnect).

#### Every upload of 2026-10-15, side by side

Every `UPLOAD START` in `tiga-fresh-test\logs\ble.log` ("live") and `tiga-dev\logs\ble.log`
("dev"), read at 23:29. All 42 went to the correct device, `07:01:88:B3:66:1B`. "Conn."
is whether the upload was the first on a new BLE connection (a new uploader process) or a
later one on the same connection. Block columns are the `elapsed` seconds at which that
block's PROGRESS arrived (the log prints every fifth block and the last). "Result" for a
failure gives the block whose PROGRESS never came and the time from `UPLOAD START`. The
"just before" column combines the logs with Michi's account.

| # | Start | Install | Conn. | b5 | b10 | b25 | b40 | b50 | b53 | Result | Timeout | Just before |
|---:|---|---|---|---:|---:|---:|---:|---:|---:|---|---:|---|
| 1 | 21:05:49 | live | 1st |  |  |  |  |  |  | **FAILED** at block 5 (3.68 s) | 2500 | First upload after stopping the old `tiga_ble_re_tools` stack. Keyboard state unknown |
| 2 | 21:15:32 | live | 1st | 0.77 | 1.56 | 3.86 | 6.14 | 7.69 | 8.15 | 8.71 s | 2500 | Battery disconnect, duration unknown. Before it: halt, 5 s USB unplug (not a reset), three wrong-device runs |
| 3 | 21:16:45 | live | later | 1.55 | 2.29 | 4.55 | 6.86 | 8.47 | 8.91 | 10.01 s | 2500 | Previous upload on this connection completed |
| 4 | 21:16:55 | live | later | 1.49 | 2.20 | 4.45 | 6.71 | 8.23 | 8.64 | 9.59 s | 2500 | Previous upload on this connection completed |
| 5 | 21:19:51 | live | later | 1.53 | 2.31 | 4.54 | 6.79 | 8.37 | 8.80 | 9.85 s | 2500 | Previous upload on this connection completed |
| 6 | 21:20:30 | live | later | 1.46 | 2.27 | 4.59 | 6.83 | 8.36 | 8.83 | 9.90 s | 2500 | Previous upload on this connection completed |
| 7 | 21:22:39 | live | later | 1.46 | 2.18 | 4.40 | 6.65 | 8.16 | 8.60 | 9.49 s | 2500 | Previous upload on this connection completed |
| 8 | 21:25:27 | live | later | 1.56 | 2.31 | 4.58 | 6.90 | 8.42 | 8.87 | 9.81 s | 2500 | Previous upload on this connection completed |
| 9 | 21:25:49 | live | later | 1.48 | 2.29 | 4.61 | 6.84 | 8.43 | 8.85 | 9.74 s | 2500 | Previous upload on this connection completed |
| 10 | 21:25:59 | live | later | 1.41 | 2.22 | 4.44 | 6.72 | 8.29 | 8.73 | 9.60 s | 2500 | Previous upload on this connection completed |
| 11 | 21:27:53 | live | later | 1.46 | 2.20 | 4.47 | 6.73 | 8.24 | 8.70 | 9.86 s | 2500 | Previous upload on this connection completed |
| 12 | 21:30:31 | live | later | 1.45 | 2.18 | 4.44 | 6.71 | 8.24 | 8.70 | 9.56 s | 2500 | Previous upload on this connection completed |
| 13 | 21:31:38 | live | later | 1.33 | 2.13 | 4.42 | 6.58 | 8.14 | 8.55 | 9.68 s | 2500 | Previous upload on this connection completed |
| 14 | 21:32:21 | live | later | 1.44 | 2.19 | 4.50 | 6.80 | 8.37 | 8.83 | 9.88 s | 2500 | Previous upload on this connection completed |
| 15 | 21:32:31 | live | later | 1.48 | 2.22 | 4.42 | 6.67 | 8.21 | 8.66 | 9.65 s | 2500 | Previous upload on this connection completed |
| 16 | 21:33:04 | live | later | 1.51 | 2.30 | 4.52 | 6.76 | 8.31 | 8.75 | 9.71 s | 2500 | Previous upload on this connection completed |
| 17 | 21:33:18 | live | later | 1.61 | 2.36 | 4.57 | 6.82 | 8.37 | 8.84 | 9.70 s | 2500 | Previous upload on this connection completed |
| 18 | 21:34:28 | live | later | 1.48 | 2.28 | 4.52 | 6.74 | 8.28 | 8.72 | 9.55 s | 2500 | Previous upload on this connection completed |
| 19 | 21:34:38 | live | later | 1.50 | 2.24 | 4.43 | 6.71 | 8.23 | 8.72 | 9.66 s | 2500 | Previous upload on this connection completed |
| 20 | 21:35:57 | live | 1st | 0.71 | 1.45 | 3.69 | 5.89 | 7.40 | 7.85 | 8.42 s | 2500 | Stack restart 21:35:35 (no reset); one code-10 scan |
| 21 | 21:39:48 | live | later |  |  |  |  |  |  | **FAILED** at block 3 (4.21 s) | 2500 | Upload 20 completed 3.7 min earlier on this connection |
| 22 | 22:44:31 | live | 1st | 2.35 | 12.02 |  |  |  |  | **FAILED** at block 11 (14.95 s) | 2500 | Halted since 21:39, no reset; then a **10–15 s** battery disconnect |
| 23 | 22:46:42 | live | 1st | 2.97 | 11.58 |  |  |  |  | **FAILED** at block 11 (14.49 s) | 2500 | Halt after 22:44, START_STACK; no further reset recorded |
| 24 | 22:49:35 | live | 1st | 0.74 | 1.58 | 3.87 | 6.11 | 7.66 | 8.12 | 8.70 s | 2500 | A **30 s** battery disconnect |
| 25 | 22:52:34 | live | later | 1.41 | 2.18 | 4.48 | 6.74 | 8.24 | 8.70 | 9.78 s | 2500 | Previous upload on this connection completed |
| 26 | 22:55:04 | live | later | 1.42 | 2.16 | 4.45 | 6.67 | 8.16 | 8.58 | 9.50 s | 2500 | Previous upload on this connection completed |
| 27 | 22:57:28 | live | later | 1.35 | 2.11 | 4.36 | 6.62 | 8.18 | 8.64 | 9.65 s | 2500 | Previous upload on this connection completed |
| 28 | 22:59:54 | dev | 1st | 0.77 | 1.53 | 3.80 | 6.12 |  |  | **FAILED** at block 41 (9.04 s) | 2500 | Live stack stopped (last upload 22:57); task pointed at `tiga-dev`. Battery state unclear |
| 29 | 23:06:33 | dev | 1st | 0.74 | 1.52 | 3.73 | 6.05 | 7.55 | 8.03 | 8.60 s | 10000 | Halt after 23:00; a 30 s battery reset; dev supervisor now passes 10000 ms |
| 30 | 23:08:48 | dev | later | 1.46 | 2.21 | 4.46 | 6.66 | 8.23 | 8.68 | 9.81 s | 10000 | Previous upload on this connection completed |
| 31 | 23:09:53 | dev | 1st | 0.71 | 1.46 | 3.69 | 5.95 | 7.57 | 8.03 | 8.61 s | 10000 | Stack restart 23:09:12 (no reset); two code-10 scans |
| 32 | 23:10:02 | dev | later | 1.34 | 2.10 | 4.39 | 6.59 | 8.26 | 8.72 | 9.74 s | 10000 | Previous upload on this connection completed |
| 33 | 23:13:15 | dev | later | 1.32 | 2.18 | 4.43 | 6.72 | 8.28 | 8.77 | 9.62 s | 10000 | Previous upload on this connection completed |
| 34 | 23:16:25 | live | 1st | 0.83 | 1.61 | 3.83 | 6.13 | 7.63 | 8.10 | 8.69 s | 2500 | Task back on the live install, restart 23:15:15 (no reset); two code-10 scans |
| 35 | 23:18:55 | live | later | 1.57 | 2.36 | 4.57 | 6.85 | 8.46 | 8.93 | 10.03 s | 2500 | Previous upload on this connection completed |
| 36 | 23:21:40 | live | later | 2.04 | 2.80 | 5.04 | 7.29 | 8.79 | 9.25 | 9.93 s | 2500 | Previous upload on this connection completed |
| 37 | 23:21:50 | live | later | 1.33 | 2.07 | 4.36 | 6.62 | 8.13 | 8.60 | 9.56 s | 2500 | Previous upload on this connection completed |
| 38 | 23:23:54 | live | later | 1.63 | 2.38 | 4.63 | 6.87 | 8.46 | 8.92 | 10.08 s | 2500 | Previous upload on this connection completed |
| 39 | 23:25:48 | live | later | 1.49 | 2.24 | 4.52 | 6.79 | 8.36 | 8.82 | 9.98 s | 2500 | Previous upload on this connection completed |
| 40 | 23:25:58 | live | later | 1.47 | 2.23 | 4.44 | 6.73 | 8.28 | 8.72 | 9.68 s | 2500 | Previous upload on this connection completed |
| 41 | 23:26:08 | live | later | 1.58 | 2.35 | 4.62 | 6.86 | 8.38 | 8.81 | 9.75 s | 2500 | Previous upload on this connection completed |
| 42 | 23:28:59 | live | later | 1.46 | 2.27 | 4.54 | 6.83 | 8.41 | 8.89 | 10.03 s | 2500 | Previous upload on this connection completed |

**Derived figures.**

- **Pre-block overhead is ~0.42 s.** For the three failures with block data, the time to
  failure equals the last ACKed block's `elapsed` + 2.5 s timeout + 0.41–0.43 s
  (23:00: 9.04 = 6.12 + 2.5 + 0.42; 22:44: 14.95 = 12.02 + 2.5 + 0.43; 22:46: 14.49 =
  11.58 + 2.5 + 0.41). That overhead is the stale-notification drain, CONTROL1/CONTROL2
  and their ACKs. It lets the two failures *without* block data be placed:
  - **21:05:** last ACKed block 4 at about 3.68 − 2.5 − 0.42 = **0.76 s**. The healthy
    first-on-connection pace puts block 4 at about 0.6–0.7 s.
  - **21:39:** last ACKed block 2 at about 4.21 − 2.5 − 0.42 = **1.29 s**.
- **First-on-connection uploads are faster, now with six samples, not two.** All six that
  completed (rows 2, 20, 24, 29, 31, 34) took 8.42–8.71 s, with block 5 at 0.71–0.83 s.
  All 31 later uploads took 9.49–10.08 s, with block 5 at 1.32–2.04 s. The difference,
  about 0.7 s, is already there by block 5 and stays constant to block 53: later uploads
  lose about 0.7 s somewhere in their first five blocks and then run at the same
  ~0.15 s per block. Where exactly in blocks 1–5 isn't visible at this granularity.
- **Failures cluster at the start of a connection.** Of the five failures, four were the
  first upload on a connection and one (21:39) the second. Of 32 later uploads, none
  failed except that one. This is confounded: a first upload usually follows a restart,
  and restarts followed failures and resets, so it says as much about the keyboard's state
  at those moments as about the connection.

**Questions (confidence in brackets).**

1. **Do 21:05 and 21:39 look like the slowing signature or the sudden one?**
   - **21:05: sudden** *(medium-high)*. Block 4 at ~0.76 s is at or near the healthy
     first-on-connection pace, then block 5 never came. A slowing run is already well
     behind by then (22:44 and 22:46 reached block 5 only at 2.35 and 2.97 s).
   - **21:39: probably sudden** *(low-medium)*. Block 2 at ~1.29 s, then nothing for
     block 3. A later upload normally reaches block 5 at 1.3–1.6 s, but because its
     0.7 s penalty sits somewhere in blocks 1–5, a healthy block 2 could be anywhere from
     about 0.5 to 1.3 s. 1.29 s is at the slow edge of healthy, not clearly slowing.
2. **Do successful uploads drift slower over the evening ahead of a failure?** **No**
   *(high, at 5-block granularity)*. Later uploads stay at 9.49–10.08 s, block 5 at
   1.32–1.63 s, from 21:16 to 23:29, with one outlier that completed (23:21:40, block 5
   at 2.04 s, total 9.93 s). The upload just before 21:39 was the fastest of the evening
   (8.42 s); the three before the 23:00 failure were 9.50–9.78 s. Nothing in the
   successful uploads warns of what's coming. Single slow blocks inside a five-block
   span would not show.
3. **Were 22:44 and 22:46 just the 21:39 state, never properly cleared?** **Can't tell;
   possible** *(low)*. For: nothing reset the keyboard between 21:39 and the 10–15 s
   disconnect, a 30 s disconnect cleared it, and both runs stalled at exactly block 11
   with different images, which looks like a stable state on the keyboard's side.
   Against: the signature changed, from a sudden stop at block 3 to a slowing run that
   reached block 10. So at least the short disconnect changed *something*. The record
   fits two stories equally well: the 21:39 state, only partly cleared by a too-short
   disconnect; or a new bad state produced by the short disconnect itself (a brown-out
   rather than a clean reset). Nobody has ever uploaded into a keyboard left *un*-reset
   after a failure and seen the result: the 21:08–21:09 attempts went to the wrong
   device. That missing comparison is what would separate the two.
   *Update 2026-10-18:* it has now been done three times. 13:42 (no reset, 46 s after a
   halt) and 14:11 (~2.5 min) completed; 14:09 (~5 min) failed (sixth part). A halted
   keyboard does not need a reset to accept a new upload.
4. **Could the uploader spot trouble earlier?** *Proposal at the time. The logging part
   (per-block latency, `SLOW` lines, the block-5 warning, CONTROL ACK times) was added
   in #22, 2026-10-16, and first ran on hardware that evening (fourth part below); see
   RETRY_RECOVERY.md.* The
   catch is that anything after CONTROL2 cannot avoid the problem: abandoning a transfer
   is what leaves the partial session. So:
   - **Log more, decide later.** Log every block's ACK latency (or per-upload min/median/
     max), and the CONTROL1 and CONTROL2 ACK latencies. Cheap, no protocol change, and the
     only way to answer the questions above with data rather than inference.
   - **Look for a signal before CONTROL2.** If the CONTROL1 ACK is measurably slow when the
     keyboard is in a bad state, the uploader could stop *before* CONTROL2, which is safe
     (code 10/11 territory, no session opened), and ask for a battery reset without making
     things worse. Whether such a signal exists is unknown until the latencies are logged.
   - **Inside the transfer, don't abort early; warn.** In the slowing signature at least
     four of blocks 6–10 took ~1.8–2.5 s against ~0.15 s healthy. A per-block latency over
     ~1 s is a clean marker for a `SLOW` log line. Using it to abort sooner would only
     produce the partial session sooner. A threshold on block 5's `elapsed` instead
     would be marginal: 2.35 s in a failure against 2.04 s in a success.

#### Third part: the 2026-10-16 restart, 18:48–18:55

Source: `tiga-fresh-test\logs\` (`startup_supervisor.log`, `ble.log`, `renderer.log`) and
`git reflog`, plus Michi's account of the battery disconnect. The live install was pulled
to `e8f7211` (the font-fallback merge, #17) at 18:48:32. Its uploader binary dates from
2026-10-15 23:51 and prints `Block timeout   : 10000 ms`, so **this is the first failure
with the 10000 ms timeout.**

Before it: the stack started at 17:52 completed **22 uploads in a row** (9.31–10.35 s),
the last at 18:46:00 (`ee1eead6`). The stack was then stopped for the pull. No upload was
running at the time, and there had been no failure or reset since the previous night.

| Time | What happened |
|---|---|
| 18:48:44 | Supervisor launched (restart after the pull, no battery reset) |
| 18:48:45 | Uploader scans, `Nothing seen in 5 s; scanning up to 15 s` |
| 18:49:00 | **Code 10**: `no candidate seen in a 15 s scan`. Retry in 8 s, no halt |
| 18:49:12 | Second scan sees `Zoom75 Tiga` (cached address) after 3 s, so the keyboard was invisible for about 28 s after the restart |
| 18:49:14 | `UPLOAD START` frame `0714ef6a` (the Morse band, first frame with the Morse subtitle) |
| 18:50:32 | **Code 20** after **77.83 s**: no PROGRESS for block 41/53 (last ACKed 40) |
| 18:50:34 | Halt (`session-open`) |
| (between) | **40 s battery disconnect** (Michi) |
| 18:54:07 | START_STACK |
| 18:54:18 | **The same frame `0714ef6a` completes in 9.06 s**, block 5 at 0.85 s |
| 18:55:08 | Next frame (`4bf3daed`, Takanaka) completes in 9.52 s |

**The failed upload, block by block** (`elapsed` at each logged block; per-block figure =
average over that five-block span; healthy ~0.15 s):

| b5 | b10 | b15 | b20 | b25 | b30 | b35 | b40 | b41 |
|---:|---:|---:|---:|---:|---:|---:|---:|---|
| 2.42 | 11.51 | 20.43 | 28.80 | 40.21 | 49.11 | 61.36 | 67.39 | never (failed at 77.83) |
| | 1.82/blk | 1.78 | 1.67 | 2.28 | 1.78 | 2.45 | 1.21 | > 10 s |

- **The slowing signature** of 22:44/22:46, but this time the 10 s timeout let it run on:
  35 blocks at roughly 1.2–2.5 s each, never recovering towards 0.15 s, then a **complete
  stall**. Time to failure = 67.39 + 10.0 + 0.44 s, consistent with the ~0.42 s pre-block
  overhead derived above.
- **Not content-related, now shown directly:** the identical frame (same CRC32) completed
  at a healthy pace after the reset. This also confirms that a 40 s disconnect clears the
  state, in line with the 30 s minimum.
- **Block 5 at 2.42 s** on a first-on-connection upload. The healthy first-on-connection
  range so far is 0.71–0.85 s (now including 9.06 s at 0.85 s and 17:52's 0.82 s); all three
  slowing runs were at 2.35–2.97 s. For first uploads that's a clean separation, unlike the
  2.04 s later-upload outlier mentioned in question 4 above. It is still after CONTROL2,
  so it can only warn, not prevent.
- **Code 10 after the restart, again.** A fourth case of the keyboard being invisible for a while after
  a restart (about 28 s, against ~19, ~38 and ~67 s on 2026-10-15). The 15 s scan did not
  absorb it, as predicted; the supervisor's 8 s retry did, with no halt.

**Failures follow stack restarts.** Counting every uploader start on 2026-10-15/16 by
what preceded it (`ble.log`, both installs):

| Before the first upload | First upload failed | Completed |
|---|---:|---:|
| Restart, no reset, last upload had completed (21:05\*, 21:35, 22:59, 23:09, 23:15, 23:46, 23:51, 17:52, 18:48) | **3** (21:05, 22:59, 18:49) | 6 |
| Battery disconnect ≥ 30 s (22:49, 23:06, 18:54; 21:15 duration unknown) | 0 | 4 |
| Battery disconnect 10–15 s, or START_STACK after a halt with no reset (22:44, 22:46) | 2 | 0 |
| Later upload on an open connection | 1 (21:39) | 62 |

\* 21:05 followed stopping the old `tiga_ble_re_tools` stack; what it had last done isn't in these logs.

*Update 2026-10-18 (sixth part):* the ≥ 30 s row now has a failure (13:41, after a
~1.5 min full power-off), and a first upload after a cold Windows boot failed too (13:20).
Across all 23 first uploads, 9 failed; a reset does not make the next one safe.

Small numbers, but it sharpens "failures cluster at connection start". The risky moment
seems to be the **first upload after a restart that did not reset the keyboard**.
`STOP_STACK.ps1` ends the uploader with `Stop-Process -Force`, so every restart drops a
live BLE connection without a clean disconnect. That might also be why the keyboard is
invisible for 20–70 s afterwards (a keyboard still holding a connection doesn't
advertise). **Hypothesis only; nothing above tests it.** The test would be a clean
disconnect before exit, then compare first-upload failures after restarts.

**Block-timeout verdict, updated.** With 10000 ms: 27 completed uploads (5 on 2026-10-15
in `tiga-dev`, 22 on 2026-10-16 live), **none with any block anywhere near 2.5 s**. After
block 5, the slowest five-block span averaged 0.21 s per block, so no single block in it
took more than ~1.05 s. Up to block 5 the largest `elapsed` was 1.65 s. **Every one of them
would also have completed with 2500 ms.** The one slowing run under 10000 ms did not
recover. It crawled to block 40, stalled completely, and ended exactly like the 2500 ms
failures: code 20 and a battery disconnect. It just took 78 s instead of failing at the
first block over 2.5 s. When that would have been can't be placed at five-block
granularity: the 20→25 and 30→35 spans averaged 2.28 and 2.45 s per block, so probably
by block 25–35, possibly earlier. **The longer wait has not rescued a single upload, and
in its one test it only delayed the halt.** One slowing run is n = 1, but it answers
the open question above: with 10 s, the transfer stalled anyway, only later.

**Decision, 2026-10-16: the default is back to 2500 ms.** `--block-timeout-ms` and the
`Block timeout   :` line in `ble.log` stay, so a longer wait can still be tried per run.
What 10000 ms was meant to reveal (does a slowing run recover?) is better answered by
per-block latency logging than by waiting longer.

#### Fourth part: 2500 ms and per-block timing live, 2026-10-16 20:16–20:56

Source: `tiga-fresh-test\logs\` (`startup_supervisor.log`, `ble.log`, `renderer.log`),
`tiga_display_command.ack`, `<stream>.delivered` and `git reflog`, plus Michi's account
of the battery disconnect and the Display Control buttons. `ble.log` stamps are UTC;
times here are local (UTC+2).

**Before it: #18 alone, 20:16–20:49.** The live install was pulled to `019da37` (#18) at
20:16 and started at 20:20:54: `Block timeout   : 2500 ms`. Another code 10 after the
restart (no candidate in the 15 s scan), and the retry found the keyboard at once. Six uploads
completed, 8.31–10.58 s, except one: **20:46:17, 20.01 s, the slowest completed upload on
record.** Block 5 came at 3.90 s, and every five-block span after it took 0.9–2.3 s
(about 0.2–0.46 s per block, against ~0.15 s healthy), with no recovery towards
the end (block 53 at 18.67 s). No single block reached the 2.5 s timeout. This was
before the timing lines existed, so the per-block figures are averages. The same image
(CRC32 `525691c8`) then completed in 9.17 s after the deploy below. So a transfer can
be uniformly slow for a whole upload and still finish, and again it wasn't the content.

**The #22 deploy.** Pulled to `5ca54b3` (includes #22) at 20:49:06, uploader rebuilt
20:49:26, a battery disconnect and reconnect (Michi), stack started 20:51:27.

- **Code 11 on the first attempt after the reconnect: the first code 11 from the real
  keyboard** (every earlier one was the BT1 typing device, §7). The scan saw the cached
  address with an **empty name** (`CANDIDATE name="" address=07:01:88:B3:66:1B`); it was
  a candidate only because of the address cache. The connect succeeded, and the
  **media service `1f40eaf8…` was present but had no characteristics yet**: `missing
  1f400001, 1f400002, 1f400003, 1f400004`, with three services listed, all empty. The rescan
  found nothing else, and the uploader exited 11 at 20:52:02. The supervisor's retry 8 s later
  saw `Zoom75 Tiga` with its name, qualified it, and the upload started at 20:52:14. The
  most likely reading is that the keyboard was still booting after the reconnect: it was
  advertising before its GATT table was complete. The code-11 retry handled it as
  designed, with no halt.
- **First upload: 9.17 s**, `TIMING … median=158ms p90=180ms max=210ms slowest=block 2
  control1_ack=18ms control2_ack=76ms`. Block 5 at 0.90 s, a little slower than the
  0.71–0.85 s range of earlier first uploads, and under the 1.5 s warning threshold.
- **Startup delay of a freshly built uploader.** At both starts after a rebuild (20:20:54
  and 20:51:27) the uploader printed its first line about 25 s after the supervisor
  launched it. At the code-11 retry, with the same binary, it started at once. This
  fits antivirus inspecting a new executable on first run (compare #15's CyberCapture
  theory). Harmless here, but it adds ~25 s to the first start after every rebuild.

**Display Control on hardware: #15 and #6's GUI path verified.** Three commands from the
utility (the surviving ack's command ID ends in `-9728-3`: process 9728's third command). The renderer acted on each
within a second and each resulting frame was uploaded:

| Time | Button | Renderer | Upload |
|---|---|---|---|
| 20:54:25 | Lock Standby | `CONTROL -> manual standby`, f0004 | completed 11.38 s |
| 20:55:00 | Lock Standby again | `CONTROL -> manual standby`, f0005 | completed 10.68 s |
| 20:55:17 | Resume Automatic | `CONTROL -> automatic mode re-armed`, f0006; Now Playing again at 20:55:20 (f0007) | completed 9.97 s, then 9.69 s |

The surviving `tiga_display_command.ack` belongs to the third command (`command_text=auto
…-9728-3`, `frame_id …-f0006`). It was written 0.9 s after the command file. Each
command was confirmed by the renderer and its frame uploaded. That verifies on hardware the #15 fix (the GUI
confirms a command actually landed, via read-back and the renderer's ack) and, through
the second Lock Standby, the #6 fix from the GUI side as well as by hand. **Not covered:**
Standby + Shut Down (#7's delivery-ack and shutdown stages), which wasn't pressed.

**What the timing lines show.** All seven uploads after the deploy:

| Upload | Conn. | Total | b5 | median | p90 | max (block) | control1_ack | control2_ack |
|---|---|---:|---:|---:|---:|---|---:|---:|
| 20:52:14 | 1st | 9.17 s | 0.90 | 158 | 180 | 210 (2) | 18 | 76 |
| 20:53:19 | later | 9.91 s | 1.48 | 157 | 180 | 697 (2) | 166 | 291 |
| 20:54:26 | later | 11.38 s | 2.39 | 157 | 195 | **1079 (2)** | 577 | 294 |
| 20:55:00 | later | 10.68 s | 1.52 | 158 | 187 | 892 (1) | 653 | 357 |
| 20:55:18 | later | 9.97 s | 1.51 | 157 | 173 | 767 (1) | 178 | 283 |
| 20:55:28 | later | 9.69 s | 1.13 | 157 | 180 | 421 (3) | 170 | 172 |
| 20:56:02 | later | 9.95 s | 1.51 | 150 | 172 | 703 (1) | 357 | 297 |

Times in ms unless marked; b5 is block 5's `elapsed` in seconds.

- **The steady pace is the same on every upload:** median 150–158 ms per block, p90
  172–195 ms. Later uploads are not slower throughout.
- **Later uploads have a slow block at the very start.** The slowest block is always
  block 1, 2 or 3: 0.7–1.1 s in five of the six later uploads (0.42 s in the sixth),
  against 0.21 s on the first upload. 20:54:26's block 2 at 1079 ms is the **first `SLOW`
  line ever logged**. The summary gives only the slowest block, so whether one or two early
  blocks are slow can't be told, but a p90 of 172–195 ms allows at most five blocks over
  ~0.2 s. **This answers §7.1's "where in blocks 1–5 do later uploads lose ~0.7 s":** one
  slow block at the start, after which the pace is normal. Block 5 at ~1.5 s on later
  uploads, against 0.9 s here on the first, matches the slow block's size.
- **control1_ack: 18 ms on the first upload, 166–653 ms on later ones.** It isn't a steady
  rise: 166, 577 and 653 ms, then back to 178, 170 and 357 ms. Both are in the time before
  `elapsed` starts, so they add to the total but not to b5. Whether the CONTROL1 delay
  and the slow first block share a cause (the keyboard still busy with the previous image?)
  is open. Eight minutes on one connection is a small sample.
- **Block 5 at 2.39 s did not mean trouble here.** 20:54:26 reached block 5 at 2.39 s,
  inside the 2.35–2.97 s range of the three slowing failures, and then ran at the normal
  pace (block 10 at 3.20 s) to complete in 11.38 s. It was a later upload whose one
  slow block was 1.08 s. The early warning is deliberately limited to first-on-connection
  uploads (healthy 0.71–0.90 s), and this upload shows why: on a later upload, b5 alone
  doesn't separate a slowing run from a slow first block.

**Verified on hardware in this part:** the 2500 ms default and the timing lines (#22);
#15; #6 through the GUI; code-11 retry recovering a keyboard that wasn't ready yet.

#### Fifth part: a failure on an open connection, and two recoveries, 20:58–21:15

Same sources, plus Michi's account of the battery disconnects, the iPad and how
START_STACK/STOP_STACK were used.

| Time | What happened |
|---|---|
| 20:56:02 | Healthy upload on the 20:52 connection, 9.95 s (median 150 ms) |
| 20:58:37 | Spotify auto-advanced (renderer: `media changed`, f0009); upload starts on the **same open connection** |
| 20:58:42 | **Code 20 after 4.86 s**: block 5 at 1.58 s (a normal later-upload pace), then no PROGRESS for block 6 within 2.5 s. `TIMING blocks_acked=5 median=188ms max=802ms slowest=block 3 control1_ack=139ms control2_ack=296ms`. Halt (`session-open`) |
| 21:00–21:09 | Renderer kept rendering (f0010–f0014); the uploader stayed halted |
| (before 21:09) | Battery disconnect of **25 s, no key presses**. **START_STACK with the supervisor still running**: it cleared the halt, and the running supervisor started the uploader (21:09:33) |
| 21:09:35 | Keyboard seen at once, **RSSI −66** |
| 21:09:40 | **Code 20 after 3.74 s**, no PROGRESS for block 5 (last ACKed 4). `TIMING blocks_acked=4 median=185ms max=300ms slowest=block 4 control1_ack=20ms control2_ack=77ms`. Halt |
| (before 21:14) | STOP_STACK, a battery disconnect of **about a minute, pressing keys**, START_STACK |
| 21:14:06 | New supervisor and renderer; keyboard seen at once, **RSSI −58** |
| 21:14:10–21:14:42 | Three uploads completed: 8.95 s (first on the connection, block 5 at 0.92 s, control1_ack 32 ms), 9.93 s, 10.06 s; medians 156–157 ms |

**20:58: the sudden signature on an open connection.** It is the second failure ever of a
later upload on a connection that had already completed uploads. The first was 21:39 on
2026-10-15. It came 2.5 minutes after a healthy upload and looks like 21:39 too: a normal
pace, then nothing. The one hint before the stall is block 3 at 802 ms. That is within
the 0.42–1.08 s slow early block seen on every later upload in the fourth part, so
**it's not a usable warning**. The `TIMING` line on a failure did its job: without it,
the slow block 3 and the CONTROL ACK times wouldn't be known.

**21:09: the first recovery failed after a 25 s disconnect without key presses, and the
keyboard did not look stuck.** Its CONTROL1
and CONTROL2 ACKs (20 / 77 ms) are what a first upload on a new connection looks like
(18 / 76 ms at 20:52, 32 / 73 ms at 21:14). Whether they would also look like that from
a keyboard that was *not* reset is unknown; nothing has logged that case yet. So the
ACK times are consistent with the disconnect having worked, but don't prove it. The
transfer then stopped at block 5 after a normal start (slowest block 300 ms), which is
the sudden signature again, not the slowing one. The 25 s was below the 30 s working
minimum and had no key presses, so this is the second short disconnect that failed to
clear the keyboard: 10–15 s on 2026-10-15 22:44 was the first. That one stalled with
the slowing signature; this one stalled suddenly.

**21:14: the second recovery worked.** It differed from the first in three ways at
once:

1. **The disconnect was longer and drained:** about a minute with key presses, against 25 s
   without.
2. **STOP_STACK came first.** A new supervisor and a new renderer started.
3. **The signal was stronger:** RSSI −58 against −66. Healthy starts that evening were
   −58/−59 (20:51, 20:52); the 21:09 failure was −66. One reading each.

**What this says about "STOP_STACK before the battery disconnect": not enough to change
the advice.** During the 20:58–21:09 halt the uploader had already exited. No process
held a Bluetooth connection to the keyboard, and the supervisor and renderer never talk
to it. So STOP_STACK has no mechanism here by which it would change the keyboard's
state. The disconnect duration has a known record, though: 10–15 s failed, 25 s without
key presses failed, and every disconnect of 30 s or more worked. **That explains the 21:09
failure without STOP_STACK.** *Correction 2026-10-18:* it is one possible explanation,
not the only one. A first upload fails about 1 time in 3 even after a long power-off
(sixth part), so 21:09 is equally consistent with that base rate. **The recovery advice stays as it is:** disconnect the
battery for at least 30 s, pressing a few keys (about a minute with key presses is what
worked here), then START_STACK.
Running STOP_STACK first does no harm and gives a fresh supervisor, so it's a reasonable
habit, but nothing shows it's needed. Separately, STOP_STACK *while an upload is running*
remains harmful (it kills the uploader mid-transfer). See the clean-stop design,
`docs/design/CLEAN_STOP.md`.

**Open question: 2.4 GHz interference. Hypothesis only.** An iPad was streaming nearby
during the 20:58 and 21:09 failures (Michi). BLE shares the 2.4 GHz band with Wi-Fi, so
heavy Wi-Fi traffic could cost PROGRESS notifications or acknowledgements, and a
lower RSSI (−66) makes that more likely. Nothing in the logs measures interference.
Whether the iPad was still streaming at 21:14 isn't recorded, and nobody noted Wi-Fi
conditions for any of the 2026-10-15 failures. **Test, if wanted:** repeat
uploads with the iPad streaming next to the keyboard and with it switched off, and log
RSSI (already in `ble.log`'s `CANDIDATE` line) and any failures.

**Tally update** (§7.1's restart table and the counts above): later uploads on an open
connection, **2 failed** (21:39 on 10-15, 20:58 on 10-16). Recoveries after a halt: one
failed (21:09: 25 s, no key presses, supervisor not restarted, RSSI −66). Every recovery
with a disconnect of 30 s or more has worked (22:49 and 23:06 on 10-15 at 30 s; 18:54 on
10-16 at 40 s; 21:14 at about a minute with key presses). Disconnects that were too short:
10–15 s (22:44 on 10-15) and 25 s without key presses (21:09 on 10-16).
*No longer true as of 2026-10-18:* the ~1.5 min power-off before 13:41 was followed by
a failure (sixth part).

#### Sixth part: 2026-10-18, failures after a boot and after a power-off, 13:20–14:11

Sources: `tiga-fresh-test\logs\` (`ble.log`, `startup_supervisor.log`,
`renderer.log`), `tiga_display_command.ack`, the Windows System event log (boot and
shutdown times), GitHub (#7), and Michi's account. Full table and analysis:
`docs/investigations/upload_attempts.csv` and `UPLOAD_FAILURES_2026-10-18.md`.

**Resets are real power-offs (Michi, 2026-10-18).** With USB-C plugged in, the front
panel won't come off, so every "battery disconnect" in this section was: unplug USB-C,
pop off the front panel, lift the PCB sandwich so the magnetic battery contacts
separate, wait, reassemble, plug USB-C back in. **Every reset was a complete power-off
and also included a USB unplug and replug.** "The reset didn't really happen" is ruled
out for all of them. Also: the keyboard types over that USB-C cable, which is always
plugged in, so typing doesn't use Bluetooth.

**#7 verified and closed.** On 2026-10-16 at 23:07:01 Standby + Shut Down rendered the
standby frame (f0010). The ack was written at 23:07:02, the upload completed at
23:07:13 (10.80 s, one `SLOW` block 1 of 1009 ms), and Windows shut down at 23:07:27
(event 6006). Shutdown came after the delivery, as designed. #7 was closed on GitHub on
2026-10-18 at 13:31.

| Time | What happened |
|---|---|
| 13:14:34 | Windows boots (keyboard idle since 2026-10-16 23:07) |
| 13:20:02 | Supervisor launched at logon; keyboard seen at once, RSSI −69 |
| 13:20:06 | Startup standby frame (`c3b19bf6`). **Failed after 7.02 s: no PROGRESS for block 27/53.** `TIMING median=157ms p90=173ms max=195ms control1_ack=21ms control2_ack=106ms`, a perfectly healthy pace. Halt 13:20:15 |
| 13:29:25 | Renderer installs the Morse band frame (`0714ef6a`); not uploaded (halted) |
| (before 13:41) | STOP_STACK; **full power-off for about 1.5 min**; reconnect; START_STACK |
| 13:41:20 | New supervisor and renderer; keyboard seen at once, **RSSI −55** |
| 13:41:21 | Startup standby (`a3f02116`). **Failed after 8.50 s: no PROGRESS for block 37/53.** `median=154ms p90=172ms max=193ms control1_ack=26ms control2_ack=66ms`. Halt 13:41:30 |
| 13:42:16 | START_STACK again, **no power-off**, 46 s after the halt (halt cleared on the running supervisor). RSSI −61 |
| 13:42:19 | **Completed in 8.71 s** (`control1_ack=30ms control2_ack=70ms`), then five more, 9.52–10.00 s |
| 14:03:57 | 7th upload on that connection (media changed). **Failed after 11.69 s: no PROGRESS for block 52/53**, healthy pace (median 150 ms). Halt 14:04:09 |
| 14:09:03 | **Wait-then-retry:** START_STACK, no power-off, ~5 min after the halt. RSSI −58 |
| 14:09:06 | **Failed after 5.79 s**: block 5 at 1.01 s, then block 6 took **1860 ms** (`SLOW`), then no PROGRESS for block 7. `control1_ack=19ms control2_ack=75ms`. Halt 14:09:13 |
| 14:11:39 | START_STACK again, **no power-off**, ~2.5 min after the halt. Cached address seen with an empty name, RSSI −70 |
| 14:11:43 | **Completed in 8.66 s**, `median=150ms max=195ms control1_ack=28ms control2_ack=69ms` |

Michi's account of the recovery: "stop stack, battery disconnect for approx.
1.5 minutes, put back in and connect, then start stack. Waited 20 seconds and nothing
appeared. Ran START_STACK again, and then it caught itself within 10 or 15 seconds."
The logs agree: 13:41:30 halt, 13:42:16 START_STACK, 13:42:28 completed.

**What today changes:**

- **A full power-off does not make the next upload safe.** 13:41 failed after ~1.5 min
  without power. "Every recovery with a disconnect of 30 s or more has worked" no longer
  holds: now 4 of 5 known-length resets of 30 s or more were followed by a completed
  upload.
- **A halted keyboard doesn't need a power-off.** 13:42 (46 s after a halt) and 14:11
  (~2.5 min after one) completed with no reset, and the 13:42 connection completed five
  more. 14:09 (~5 min) and 22:46 on 2026-10-15 (116 s) failed the same way. Retry
  without any reset: **2 of 4**; the delay sorts nothing.
- **Signal strength doesn't explain it.** −55 (the strongest on record) failed; −79
  worked four times on 2026-10-15.
- **The first upload on a connection fails about 1 time in 3.** Across all 24 first
  uploads, 9 failed; across 91 later uploads, 3 failed. **Attempts that followed a
  failure failed 5 of 12; attempts whose previous upload had completed, 4 of 12.**
  Sudden failures fit independent draws at that rate. "Stuck until a power-off" is
  refuted by 13:42 and 14:11. The one hint of a persisting state is the slowing pair at
  block 11 (2026-10-15 22:44/22:46). With that base rate, the old
  "30 s works, shorter doesn't" record (4 of 4 and 0 of 2) could have arisen by chance
  (0.7⁴ ≈ 0.24). It remains the best guess for the slowing failures of 2026-10-15 22:44/22:46.
- **No warning before the stall** at 13:20, 13:41 and 14:03: block latencies were
  normal right up to it, and the CONTROL ACK times match successful first uploads.
  14:09 is the first case where `TIMING` caught a single slow block immediately before a
  stall.
- **Block numbers:** 27 (the block containing the frame-1/frame-2 boundary), 37, 52 and 7.
  Across all 12 failures there is no clustering at the boundary.

**Recovery advice, unchanged in the code for now:** the halt instruction still says
power off, then START_STACK. A power-off was followed by a completed upload 5 of 6
times after a failure, a plain retry 2 of 4. The difference is within chance (n = 6 and 4).
If the first upload after a power-off fails, it's a new failure at the ordinary
first-upload rate, not proof the power-off was too short. **What it would mean for bug
A:** if independence holds, a bounded automatic retry after a *sudden* code 20 (wait
60–120 s, at most two retries, then halt; slowing failures halt at once) would recover
most halts without a power-off. That revises bug A's "never retry after CONTROL2" and is
Michi's decision. The proposal and the confirming experiment (retry-first recovery on
the next 6–8 halts) are in `docs/investigations/UPLOAD_FAILURES_2026-10-18.md`.
*Decided later on 2026-10-18:* Michi approved an automatic retry after a sudden code 20:
120 s, up to 3 retries, then the halt as before (`feature/sudden-retry`; see the note
under bug A in §7).

**Answers to run F's questions (Michi, 2026-10-18):**

- **The delay between reconnecting at 13:41 and START_STACK is not known.** So "the
  keyboard wasn't settled yet after power-on" can neither be confirmed nor ruled out for
  the 13:41 failure. To test it, note that delay at future power-offs.
- **The LCD showed the roar at every failure today** (13:20, 13:41, 14:03, 14:09). The
  roar is the stock firmware's upload state, entered at CONTROL2 (§4.1). So at each stall
  the keyboard was still in the transfer it had accepted, not reset, crashed or showing
  the previous image. That fits the uploader's view (an open session, PROGRESS stopped).
  It also fits 13:42 and 14:11: a new CONTROL1/CONTROL2 on a keyboard still in that state
  started a transfer that completed.

#### Seventh part: 2026-10-22, clean stops and automatic retry on the live install, a slowing halt, 21:11–22:27

Source: `logs\startup_supervisor.log`, `logs\ble.log` and `logs\uploader_retry.json` of the
live install (`tiga-fresh-test`, `main`). Times are local (UTC+2); `ble.log` stamps are UTC.

Earlier the same evening, 20:30–20:49, Michi tested PR #20 with the task pointed at
`tiga-dev` (`feature/clean-stop`). That test had four clean stops (idle, mid-upload, during
a retry wait, idle) and the **first automatic retry on hardware** (start D: a sudden failure
at block 7, recovered by retry 1 in 8.73 s, no power-off). Its record is in
`docs/NOTES_FOR_MICHI.md`, "Hardware test of PR #20 (clean stop), 2026-10-22"; its logs were
in `tiga-dev`, not in the live install's logs used below. Everything below is the first time
on the live install, after #20 and #28 were merged.

**Clean stop on the live install (#20): worked both times.** 21:11:24 and 22:23:41: `STOP_STACK: BLE
uploader exited on request after 0,3 s.` `ble.log`: `STOP requested; no upload in progress,
disconnecting.` At 22:23:41 the supervisor also logged code 0 (success), reset the retry
counter (a frame had been delivered), and `not restarting it while the request stands`.
Neither stop came during an upload.

**First automatic retry on the live install (#28): a sudden failure recovered without a power-off.**

| Time | Event |
|---|---|
| 21:23:23 | Stack started. Cached address seen with an empty name |
| 21:23:29 | First upload on the connection |
| 21:23:33 | **Failed after 4.02 s**: no PROGRESS for block 5/53 (last ACKed 4). `median=171ms p90=608ms max=608ms`, `control1_ack=21ms control2_ack=77ms`. `SIGNATURE sudden`, code 20 |
| 21:23:33 | Supervisor: `automatic retry 1 of 3 at 21:25:33, in 120s. No power-off` |
| 21:25:34 | `Automatic retry 1 of 3: session marker removed; starting the BLE uploader (no power-off).` |
| 21:25:47 | **Completed in 8.61 s**, `median=150ms max=187ms`. 21:25:48 `Automatic retry 1 of 3 SUCCEEDED` |
| 22:23:18 | The next upload on that connection also completed (9.86 s) |

During the 21:23 stall the 28-byte status notification came **10 times in about 3 s**
(19:23:30, then 9 repeats 19:23:31–33), much faster than its usual ~2 s interval. One
case; noted, not interpreted.

**22:24: a slowing failure, halted as designed.**

| Time | Event |
|---|---|
| 22:24:15 | Stack started |
| 22:24:17 | First upload on the connection |
| 22:24:20–24 | `SLOW` blocks 5, 6, 7: **1845, 2355, 1470 ms**. `WARNING first upload is slow (block 5 at 2.97 s)` |
| 22:24:27 | **Failed after 9.72 s**: no PROGRESS for block 8/53 (last ACKed 7). `median=645ms p90=2355ms`, `control1_ack=31ms control2_ack=71ms`. `SIGNATURE slowing`, code 20 |
| 22:24:27 | `BLE uploader HALTED (session-open) … signature slowing, so no automatic retry.` Renderer kept running |
| 22:27:08 | Halt cleared by `START_STACK`; **no power-off** (Michi) |
| 22:27:12 | **Completed in 8.53 s**, `median=150ms p90=165ms max=187ms` |

**The keyboard kept sending its 28-byte status notification during the stall:** once at
22:24:18, then 3 repeats 22:24:21–25, about every 2 s, as between uploads. It is not
stall-specific: the healthy 22:27 upload received it 4 times too. So during this slowing
stall the keyboard was not silent on `1f400002`, but it sent nothing new either: no
unknown value, no `1f400004` traffic.

**What this changes:**

- **A slowing failure recovered without a power-off** for the first time: 22:27, ~2.7 min
  after the halt, one plain START_STACK. Until now the slowing failures (2026-10-15
  22:44/22:46, 2026-10-16 18:49) were the one hint of a state that persists until a
  reset. One case does not refute that; the policy (slowing halts at once) is unchanged.
- **Slowing failures now 4.** This one's block 5 came at 2.97 s, inside the 2.35–2.97 s
  range of the earlier three.
- **The automatic retry worked both times it ran to completion** (2 of 2: start D at
  20:30–20:49 on `tiga-dev`, and 21:25 here; start C's retry was cancelled by a stop and
  doesn't count). Retries without a power-off, all sources (the sixth part's 2 of 4 plus
  today's three): **5 of 7**. After a sudden failure 4 of 5 (2026-10-18 13:42 and 14:11,
  today start D and 21:25; 14:09 failed); after a slowing failure 1 of 2 (2026-10-15 22:46
  failed, today 22:27).

**Observation only: both failures were first uploads shortly after a RAM read
session.** Michi ran read-only memory sessions from the runtime lab (meletricks, §5–6)
with the stack stopped. They send memory-read requests (`0x08`) to the `fe00` service, no
writes. Times from the lab's `summary.json` files:

| RAM session | Ended | Stack started | First upload | Result |
|---|---|---|---|---|
| 1 | 20:58:24 | 20:58:53 (+29 s) | 20:59:32 (+68 s) | completed |
| 2 | 21:23:15 | 21:23:23 (+8 s) | 21:23:29 (+14 s) | **sudden failure** |
| 3 | 22:24:08 | 22:24:15 (+7 s) | 22:24:17 (+9 s) | **slowing failure** |

Two of three, with no control: first uploads fail about 1 time in 3 anyway (sixth part),
and the two failures have different signatures. Not a finding. To test it, wait a minute
or more after a RAM session before START_STACK, and note the gap at every session.

#### Eighth part: 2026-10-25, the first Phase 8 run of the installer

Source: `C:\Users\Regal\Downloads\tiga-phase8\` (setup logs #002 and #003, `logs-packaged\`),
`docs/NOTES_FOR_MICHI.md` and `docs/RELEASE_CHECKLIST.md`. Build `vpr-32` (installer CI run
39853101947, `4d06fdf`) over v0.2.0-beta.1. Times are local (UTC+1 from this day on);
`ble.log` stamps are UTC.

**What the logs show:**

| Time | Event |
|---|---|
| 09:26:34 | v0.2's packaged supervisor starts. It truncates its logs, so `ble.log` keeps only its last run: 14 completed uploads, 8.34–9.25 s |
| 09:27:29–09:31:00 | **v0.2's renderer exits 16 times**, each restarted after 8 s |
| 09:30:56 | Setup log #002 ends with "Exit Setup? Yes", before any file was copied |
| 09:33:00 | Setup log #003: all files copied. 09:33:02 `STOP_STACK: no BLE uploader was running` |
| 09:33:06 | v0.3 stack starts. 09:33:23 **`UPLOAD START frame=unknown(no-sidecar) crc32=57e8e0cf`: the red/green template**, because the renderer's first frame came only at 09:33:42 (36 s) |
| 09:34:27 | That upload **completes in 64.08 s**: `median=1140ms p90=1725ms max=2407ms`, every block slow, no stall. The slowing pattern without a timeout |
| 09:34:28 | The next upload fails with code 10: `control ACK pair timed out` after CONTROL1. Restarted after 8 s; completes at 09:34:49 in 8.83 s |
| 09:36:43, 09:38:24 | `OH! TENGO SUERTE` 10.72 s; `トーキョーレギー` 9.62 s. `FONT fallbacks: YuGothB.ttc, msyhbd.ttc, msjhbd.ttc, malgunbd.ttf, seguisym.ttf; not installed: meiryo.ttc`, and no `FONT no font has` line |
| 09:41:21 | *Stop Display*: `exited on request after 0,3 s`, `Disconnected cleanly`, code 0 |
| 09:44:46 | First upload after *Start Display*: **slowing failure**, last ACKed block 8, `median=203ms p90=2198ms`. Halted |
| 09:50:36 | Halt cleared. 09:50:44 first upload: **sudden failure**, last ACKed block 4 |
| 09:52:44 | Automatic retry 1 of 3. 09:52:55 completes in 8.64 s. **The first automatic retry on the packaged build** |

**What only Michi reported** (not in these logs):

- **The upgrade failure itself.** "DeleteFile failed; code 5. Access is denied" on
  `app\_internal\tiga_stream_watch_flow.exe` was on screen, and it appears in neither setup
  log here: #002 quits before copying and #003 ran after v0.2 had been stopped. The cause
  doesn't depend on the log, because `[Files]` always ran before the `[Run]` stop. Fixed in
  `e0add1b`.
- **The crash loop came from a Japanese title** (bug G, fixed in this release). The logs show
  the exits, with a blank exit code, but not the title.
- **The Japanese title displayed as glyphs.** The logs show the fallback fonts and no missing
  character, not the LCD.
- **The *Start Display*, *Stop Display* and *Status* windows closed at once.** Fixed in
  `7d086bc`.
- **Uninstalling and re-pointing the task at `tiga-fresh-test`.** The live log only shows the
  result: its stack starts at 09:57:56.

**The logs can't show** whether the battery was disconnected between the 09:44 halt and the
09:50:36 *Start Display*, so that start isn't counted in the no-power-off tally below.

**What this changed:** three fixes (`e0add1b` stop before copying, `319b426` hold a stream
with no `.frame`, `7d086bc` keep the windows open) and `b0c207b` (`[InstallDelete]` for a
leftover `.frame`). Slowing failures: now 5.

#### Ninth part: 2026-11-02, the retest and the release

Source: `C:\Users\Regal\Downloads\tiga-phase8-retest\` (setup logs #001–#005,
`logs-packaged\`), the live install's logs, `gh`. Build at `b0c207b` (installer CI run
40182232342); the files it installs are stamped 09:49:34 UTC on 2026-10-25, seconds after
that run started. Its artifact keeps the old name, `…-vpr-32.exe`.

**What the logs show:**

| Time | Event |
|---|---|
| 14:58:53 | The live stack is stopped (`STOP_STACK: no BLE uploader was running`; it had been halted since 2026-10-25) |
| 15:00:51 | Setup #001, the new build: `no installed version`. Display Control's executable is run from Setup at 15:01:03 |
| 15:06:53 | Setup #002, v0.2: **`Creating directory`**, so the install folder had been removed in between |
| 15:08:52 | Setup #003, **the old build** from `Downloads\tiga-phase8` over the running v0.2: **"DeleteFile failed; code 5. Access is denied"** on `app\tiga_windows_nowplaying.exe`. Aborted and rolled back at 15:10:27 |
| 15:13:10 | Setup #004, v0.2 again. Its uploader exits four times by 15:14:45 and the last run, from 15:14:53, completes 3 uploads (8.40–9.04 s). The logs don't say why it exited, and the exit code is blank |
| 15:16:28 | **Setup #005, the new build over the running v0.2:** `Stopping the installed version before copying files`, `STOP_STACK.ps1 exit code: 0`, `No TIGA stack process runs from … (waited 0 ms)`, then `Starting the installation process`. No DeleteFile line |
| 15:16:59 | `UPLOAD HELD frame=unknown(no-sidecar) crc32=57e8e0cf`. 15:17:02 `RENDERER sidecar appeared after 3.2s` |
| 15:17:12 | First upload, the standby frame: `UPLOAD COMPLETE … in 9.08s`. 15:17:22 second upload, 10.34 s |
| 15:19:56 | The live stack starts again, without `BLE uploader is halted`: the halt file was gone. No `Halt cleared` line, because no supervisor was running to notice |
| 15:47:29 | PR #33 merged as `601d325`. The v0.3.0-beta.1 tag points at that commit; its installer run is green |
| 15:55:20 | Release published with `TIGA-Windows-Media-Display-Setup-v0.3.0-beta.1.exe`. Its text matches `docs/RELEASE_TEXT_v0.3.0-beta.1.md` |

So #003 and #005 are the same step with the old and the new build, minutes apart: a locked
file before the fix, none after it.

**What only Michi reported** (not in these logs):

- No red/green flash on the LCD, and the *Status* window staying open and showing OK.
- The Known problems taken from this session: Norton CyberCapture holding or sandboxing the
  programs, with "Access is denied" lines during that time; Display Control keeping its own
  file locked; "The device is not ready for use" with Bluetooth off. None of these strings is
  in the retest's logs.
- The decision to skip Phase 4.

**Not tested by the retest:**

- **`[InstallDelete]` with a leftover `.frame`.** The folder was recreated at 15:06 and v0.2
  never writes that file, so there was none at #005. `UPLOAD HELD` proves the hold, not the
  deletion. An upgrade from v0.3.0-beta.1 to a later version is the first real test.
- *Start Display* and *Stop Display* on the new build: none in the logs up to 15:18. The
  release text lists their windows under "Not yet verified".
- The killed-uploader halt (Phase 4), skipped by Michi's decision; offline tests only.
- Recovery after Bluetooth is turned back on. The class comes from the code: a failure of the
  Bluetooth manager, adapter list or scan start is code 10.

#### Tenth part: retry outcomes on the live install, 2026-10-22 to 2026-11-02

Source: `tiga-fresh-test\logs\startup_supervisor.log`, `ble.log` and `uploader_retry.json`,
read at 16:04 on 2026-11-02 (last supervisor line 15:52:21), recounted from those lines.
Only the live install; the packaged runs above are separate.

**The count depends on where the window starts:**

| | From 2026-10-22 on | After 2026-10-22 (not in the seventh part) |
|---|---|---|
| `Automatic retry … SUCCEEDED` (recoveries) | **7** | **6** |
| `Automatic retry … FAILED` | 2 | 2 |
| Automatic retries started | 9 | 8 |
| `HALTED`, signature slowing | **3** | **2** |
| Halted after all 3 retries | 0 | 0 |

Michi's figures were 6 recoveries and 3 slowing halts. Each matches one column, not the same
one: the 3 includes 22:24 on 2026-10-22 and the 6 leaves out 21:25 that evening.

| Failure | Signature | Last ACKed | First upload of the run? | Outcome |
|---|---|---|---|---|
| 10-22 21:23:33 | sudden | 4 | yes | retry 1 OK, 21:25:48 |
| 10-22 22:24:27 | slowing | 7 | yes | **halted** |
| 10-23 21:54:13 | sudden | 4 | yes | retry 1 failed (21:56:22, sudden, block 5, also a first upload), retry 2 OK, 21:58:37 |
| 10-23 22:23:39 | sudden | 3 | no, after 4 | retry 1 failed (22:25:50, sudden, block 24, also a first upload), retry 2 OK, 22:28:03 |
| 10-25 10:31:07 | sudden | 4 | no, after 9 | retry 1 OK, 10:33:20 |
| 10-25 10:36:29 | slowing | 7 | yes, the run after a code 10 at 10:36:05 (`control ACK pair timed out`) | **halted** |
| 10-25 11:04:53 | slowing | 2 | no, after 4 | **halted, until 2026-11-02** |
| 11-02 15:25:48 | sudden | 2 | no, after 4 | retry 1 OK, 15:28:01 |
| 11-02 15:44:47 | sudden | 25 | no, after 5 | retry 1 OK, 15:47:00 |
| 11-02 15:50:07 | sudden | 21 | no, after 2 | retry 1 OK, 15:52:21 |

**What this shows:**

- **Every sudden incident up to this read recovered without a halt: 7 of 7.** Two needed a second
  retry; none needed the third. That no longer holds: see 17:41 in the eleventh part.
- **The automatic retry, all sources:** the seventh part's 2 of 2, then these 8 retries (6 OK)
  and the packaged one at 09:52 on 2026-10-25. Retries without a power-off in the seventh
  part's tally go from 5 of 7 to **12 of 16**; after a sudden failure from 4 of 5 to
  **11 of 14**.
- **Slowing failures: now 7** (4 up to the seventh part, the packaged 09:44, and 10:36 and
  11:04 here). After 10:36 the halt was cleared at 10:52:05 and 4 uploads completed. After
  11:04 the next start was on 2026-11-02, and 4 uploads completed. The logs can't show
  whether the battery was disconnected either time, so neither is in the tally.
- **A halt survives reboots, on the record:** the 11:04 halt lasted until 2026-11-02. The
  supervisor started 5 times in between (10-25 14:06, 10-27, 10-28, 11-01, 11-02 13:47), logged
  `BLE uploader is halted` and kept the renderer running each time.
- **84 uploads completed in these runs (from 21:23 on 2026-10-22):** median 9.71 s, 8.53 s at
  the fastest. Three took longer than 11 s and still completed: 12.43, 13.25 and 30.12 s (the
  last on 2026-10-23, `median=240ms max=5440ms`).

**Observations, not findings:**

- **Later uploads failed more often than the release text says.** First uploads of a run: 6
  of 19 failed, which fits "about 1 in 3". Later uploads: 7 of 78, about 1 in 11, against
  "about 1 in 30" in the text. One of the 7 is the code 10, which never opened a transfer;
  without it, 6 of 77. Three of the 7 came within 25 minutes on 2026-11-02. A small sample,
  and a different period from the one the 1 in 30 came from. The text hasn't been changed.
- **With two ACKed blocks the signature turns on a few milliseconds.** 10-25 11:04 has
  `median=502ms` and is slowing, so it halted. 11-02 15:25 has `median=492ms` and is sudden, so
  it was retried and recovered. Both stopped after block 2. The rule (median over 500 ms)
  wasn't made for so few blocks. Policy unchanged.

#### Eleventh part: 2026-11-02, the merged refactors on the live install, and that day's failures

Source: `tiga-fresh-test\logs` and its files, read again at 17:52 (last entry 17:52:14). The stack
was still running, so later uploads aren't in these counts. Times are local (UTC+1); upload times are converted from `ble.log`'s UTC. `#36`–`#38` were merged by 17:19,
ending at `b1fc213`.

**Which code ran.** The live checkout is clean at `b1fc213` and its files are stamped 17:20. The
`tiga_render_*.pyc` files were compiled at 17:20, so the split renderer was imported. The uploader
exe is stamped 17:20 and has the size of the split build, 1303552 bytes (`main` before the split:
1297920). So everything from 17:20:48 on is new code, everything before it old.

**What the logs show from 17:20:**

| Time | Event |
|---|---|
| 17:20:44 | `STOP_STACK: no BLE uploader was running` (it had halted at 17:14) |
| 17:20:48 | **Start 1.** The new supervisor launches, with no `BLE uploader is halted` line: the halt file was gone |
| 17:21:15 | The uploader's first line, **26 s after it was started**; every other run that day took at most 1 s. It was this exe's first run. Cause not known |
| 17:21:31 | Code 10: `Zoom75 Tiga not found (no candidate seen in a 15 s scan)`. Restarted after 8 s |
| 17:21:41 | Connected at once. First upload: **sudden failure**, last ACKed block 4, after 3.70 s |
| 17:23:48 | Automatic retry 1 of 3. 17:23:59 completes in 8.68 s. 17:24:00 `SUCCEEDED`. No power-off |
| 17:27:29 | **Clean stop:** `STOP requested; no upload in progress, disconnecting.`, `Disconnected cleanly.`, code 0; `exited on request after 0,3 s` |
| 17:27:31 | **Start 2.** 17:27:35 standby frame, 8.79 s |
| 17:31:12 | **Real media:** `FRESH PLAYBACK (playback started)`, `Source: Browser Media`, `Title: Arrival at Kraghammer \| Critical Role: VOX MACHINA \| Episode 1`, duration 3:03:34, artwork. 10.52 s. `FONT fallbacks: YuGothB.ttc, msyhbd.ttc, msjhbd.ttc, malgunbd.ttf, seguisym.ttf; not installed: meiryo.ttc` |
| 17:31:38 | **Display Control:** `CONTROL -> manual standby`, frame completes in 9.89 s |
| 17:31:52 | **Display Control:** `CONTROL -> automatic mode re-armed`, frame completes in 10.56 s. The ack file names `auto …-23428-2` and frame `f0004`; 23428 is the running `tiga_display_control.exe`, the build of 2026-10-15, which these refactors didn't touch |
| 17:32:08 | `FRESH PLAYBACK (steady playback)`. Upload: **sudden failure**, last ACKed block 21, after 7.35 s |
| 17:34:17 | Automatic retry 1 of 3. 17:34:28 completes in 8.77 s. 17:34:29 `SUCCEEDED`. No power-off |
| 17:41:27 | Another browser video, `NEW WINDOWS MEDIA`: completes in 10.33 s |
| 17:41:51 | Next upload: **sudden failure**, last ACKed block 19, after 6.37 s, `median=158ms`. From 17:41:53 to 17:42:10 the renderer writes 5 more frames as the browser changes media; only the newest is uploaded next |
| 17:43:59 | Automatic retry 1 of 3. 17:44:03 upload starts |
| 17:44:34 | **Slowing failure**, last ACKed block 21, after 31.03 s (per-block times below) |
| 17:44:36 | `Automatic retry 1 of 3 FAILED (code 20, signature slowing)`, then `BLE uploader HALTED`. **The first sudden incident on the live install to end in a halt, and the first retry to turn slowing** |
| 17:49:22 | Idle standby frame, written while the uploader is halted |
| 17:49:53 | `Halt cleared`; the running supervisor starts the uploader again |
| 17:49:57 | First upload: **sudden failure**, last ACKed block 8, after 4.23 s |
| 17:52:02 | Automatic retry 1 of 3. 17:52:03 upload starts, completes in 9.05 s. 17:52:14 `SUCCEEDED` |

So on new code: 2 supervisor starts, 13 uploads, 8 complete (8.68–10.56 s), 5 failed (4 sudden, 1
slowing), 4 automatic retries (3 recovered, 1 failed and halted), 1 halt, 1 clean stop, and nothing
in `renderer_error.log`.

**The 17:44 retry, block by block.** `SLOW` lines give each block over 1000 ms; the rest come from
the `elapsed` values, so they are derived and rounded.

| Block | ms | Block | ms | Block | ms |
|---|---|---|---|---|---|
| 1–4 | about 770 in all | 10 | 2152 | 16 | 1102 |
| 5 | 2288 | 11 | about 973 | 17 | 2115 |
| 6 | 1732 | 12 | 1447 | 18 | 1283 |
| 7 | 1792 | 13 | about 857 | 19 | 1305 |
| 8 | 2183 | 14 | 1733 | 20 | 1454 |
| 9 | 1478 | 15 | 1920 | 21 | 1545 |

Block 22 got no PROGRESS within 2500 ms. `TIMING`: `blocks_acked=21 median=1454ms p90=2152ms
max=2288ms slowest=block 5 control1_ack=18ms control2_ack=73ms`. The control ACKs were normal, and
so were blocks 1–4; then `WARNING first upload is slow (block 5 at 3.06 s; healthy 0.7–0.85 s)`, and
15 of the next 17 blocks were over 1 s. The keyboard's 28-byte status message kept coming: once at
17:44:05, then 14 repeats from 17:44:07 to 17:44:33. Nothing else arrived. The retry began 126 s after the
sudden failure before it, as the policy says. The uploads either side of it were healthy until they
stopped: 17:41:51 `median=158ms`, 17:49:57 `median=157ms`.

**The 15-second rule fired for the first time.** The `steady playback` line at 17:32:08 is the
only one in `renderer.log`. It came 16 s after the re-arm, so this browser session reported Playing
without its position advancing 1.5 s in that time. The v0.3.0-beta.1 release text lists the rule
under "Not yet verified on the keyboard" and says no such player is known. The text hasn't been
changed.

**What the logs don't show:**

- `START_STACK.ps1` and `STATUS_STACK.ps1`. They leave no line of their own, so nothing here says
  which script removed the halt before 17:20:48, or whether it was the new one. At 17:49:53 the
  checkout was already new, and both the halt file and the session marker were gone (the uploader
  refuses to start with a marker). That fits the new `START_STACK.ps1`, the only script that
  removes both, but the log doesn't name it.
- What the LCD and Display Control's window showed.
- Whether the battery was disconnected before 17:08:43, 17:20 or 17:49:53, so none of those starts
  is in the no-power-off tally below.

**10 of 40 uploads failed that day.** Recounted from `ble.log`: 40 `UPLOAD START` lines on
2026-11-02, 30 `UPLOAD COMPLETE`, 10 `UPLOAD FAILED`. This is the live install only; the retest's
logged uploads on the installed builds (3 on v0.2, 2 on the new build, all complete) are separate.
A read at 17:37 gave 7 of 35; three failures came after it.

| Upload started | Code | Signature | Last ACKed | Gap since previous upload | Nth of its run | Outcome |
|---|---|---|---|---|---|---|
| 15:25:42 | old | sudden | 2 | 0 s | 5th | retry 1 OK |
| 15:44:39 | old | sudden | 25 | 340 s | 6th | retry 1 OK |
| 15:49:59 | old | sudden | 21 | 146 s | 3rd | retry 1 OK |
| 16:36:14 | old | slowing | 2 | 261 s | 9th | **halted**, cleared 17:08:43 |
| 17:14:42 | old | slowing | 1 | 258 s | 4th | **halted**, gone by 17:20:48 |
| 17:21:41 | new | sudden | 4 | 415 s, across a restart | 1st | retry 1 OK |
| 17:32:08 | new | sudden | 21 | 5 s | 5th | retry 1 OK |
| 17:41:51 | new | sudden | 19 | 14 s | 3rd | retry 1 failed, slowing (next row) |
| 17:44:03 | new | slowing | 21 | 126 s | 1st, the retry | **halted**, cleared 17:49:53 |
| 17:49:57 | new | sudden | 8 | 323 s | 1st | retry 1 OK |

- **Old and new code: too few uploads to compare, and the new code isn't ruled out.** 5 of 27 failed
  on old code that day, 5 of 13 on new. The new code's rate is higher, but 13 uploads can't settle it:
  a split this uneven would arise by chance about one time in four (Fisher's exact test, p = 0.25),
  and that test assumes independent uploads, which these may not be. The refactors are not shown
  to be harmless, and not shown to be the cause. What can be said: uploads failed in both ways on both,
  and 5 failures came before the refactors. By upload: first of a run, 0 of 5 on old and 3 of 7 on
  new; later, 5 of 22 on old and 2 of 6 on new. The pin test covers what `upload_stream` writes, not how
  the keyboard answers. Several days of live logs on `b1fc213` are needed; the release checklist requires
  that comparison before the next tag.
- **No pattern in the gap since the previous upload.** By gap, failed of all uploads: up to 5 s, 2
  of 4; 6–60 s, 1 of 5; 61–300 s, 4 of 22; over 300 s, 3 of 9. Failures sit among successes at
  both ends, and the first two bands are small. The 30 completed uploads had gaps from 1 s up.
- **No known cause.** A slowing failure shows in its block times as it happens; a sudden one shows
  nothing before it stops. Nothing in these logs says why either begins.
- **7 sudden, 3 slowing.** Of the 7 sudden incidents, 6 recovered on retry 1 and 1 ended in a halt.
- First upload of a run: 3 of 12 failed. Later uploads: 7 of 28. The day's later-upload rate is
  again well above "about 1 in 30" (tenth part). One day, and 7 of the 12 runs were automatic
  retries.
- **The 17:14 halt rested on one ACKed block of 993 ms,** under the 1000 ms `SLOW` line and halted
  only because the median was over 500 ms. Same issue as in the tenth part. The 17:44 halt is
  not that case: 21 blocks, 15 of them over 1 s. Policy unchanged.

**Totals on the live install from 2026-10-22 on, updating the tenth part:** 10 recoveries, 3 failed
retries, 13 retries started, 6 slowing halts. **Of 11 sudden incidents, 10 recovered and 1 ended in a
halt** (17:41, whose retry turned slowing); none used all 3 retries. Retries without a power-off, all
sources: **15 of 20**; after a sudden failure **14 of 18**. Slowing failures: **now 10**.

**The installed version of these refactors has not been run, and it must be, with a new setup file,
before the next tag.** All of the above is the development install, on Python and a locally built
exe. The frozen renderer with its new modules and the packaged supervisor and Start, Stop and Status
scripts have only been built and checked offline. The v0.3.0-beta.1 setup file is older than them,
and no install folder exists on this PC right now. The steps are at the top of
`docs/RELEASE_CHECKLIST.md`.

#### Twelfth part: 2026-11-02, the Bluetooth adapter, and the short-sample signature

Source: `tiga-fresh-test\logs` (`ble.log`, `upload_soak.log`, `startup_supervisor.log`), read at
19:31 (last upload 19:30). Times are local.

**What Michi reported:** at about 18:20 he found the Bluetooth adapter lying right behind the PC
case, instead of centred under the monitor, and put it back. The logs can't say when it got there.

**What the logs show.** Signal at each connection, from the `CANDIDATE` lines:

| Period | Readings (dBm) |
|---|---|
| 2026-10-15 to 2026-10-25 | -55 to -71, with -79 and -81 once each (36 readings) |
| 2026-11-02, up to 18:19 | -72 to -96 |
| 2026-11-02, 18:21:12 and 19:19:30 | -56, -57 |

| 2026-11-02, live install | Uploads | Failed |
|---|---:|---:|
| Before the move (15:20–18:19) | 44 | 11 |
| After the move (18:21–19:30) | 49 | 1 |
| Whole day | 93 | 12 |

- **`tools/upload_soak.ps1` delivered 30 of 30** from 19:22 to 19:29, each in 10.2–11.2 s.
- **That was its second run after the move.** The first, from 18:30, delivered 11 and then halted at
  upload 12, at 18:33:19, at -56 dBm. That halt rested on one block of 987 ms; see below. It was
  cleared at 19:19:30, and the logs can't show whether the battery was disconnected.

**What this does and doesn't show:**

- The move came with a clear change: 11 of 44 failed before it, 1 of 49 after. The one failure
  after it would not have been a halt under the new rule, though it would still have been a failure.
- **30 of 30 is strong against the day's earlier rate, not against the older one.** Later uploads
  before the move failed 8 of 32, about 1 in 4, and 30 in a row at that rate would be about 1 chance
  in 5600. At the "about 1 in 30" of the release text, 30 in a row happens about 1 time in 3.
- **The soak doesn't cover first uploads.** All 30 ran on one connection, back to back.
- **A weak signal isn't the whole cause.** Failures on 2026-10-15 to 2026-10-25 came at -55 to -71
  dBm, and first uploads failed about 1 in 3 then.
- **The old-vs-new comparison in the eleventh part is confounded.** All 27 old-code uploads that day
  and the first 17 new-code ones ran on the weak signal (5 of 27 and 6 of 17 failed). New code on
  the good signal: 1 of 49. Same-signal old code hasn't been measured since the move.

**Corrections to what the eleventh part's read could see:** the day ended at 12 of 93, not 10 of 40.
Live-install totals from 2026-10-22 on, with the labels as logged: 11 recoveries, 3 failed retries,
14 retries started, 7 slowing halts. Retries without a power-off, all sources: 16 of 21; after a
sudden failure 15 of 19.

**The short-sample signature.** Four of those halts rested on one or two ACKed blocks. Healthy
uploads often start that slowly. Replaying the rule on every failure in `ble.log`, the old rule
matches every logged `SIGNATURE` line. With a minimum of 5 blocks, 4 of the 27 PROGRESS timeouts
with timing data change, all from slowing to sudden:

| Failure | Blocks ACKed | Median | Next upload |
|---|---:|---:|---|
| 2026-10-25 11:04 | 2 | 502 ms | complete, 8 days later |
| 2026-11-02 16:36 | 2 | 1019 ms (block 1: 1865 ms) | complete, 33 min later |
| 2026-11-02 17:14 | 1 | 993 ms | failed sudden, 7 min later; the retry after it completed |
| 2026-11-02 18:33 | 1 | 987 ms | complete, 46 min later |

Those next uploads came after a manual clear, and whether the battery was disconnected isn't
logged, so this doesn't prove a retry at 120 s would have worked. **Slowing failures, on the new
rule: 7**, all with at least 7 ACKed blocks (2026-10-15 22:44 and 22:46, 2026-10-16 18:49,
2026-10-22 22:24, the packaged 2026-10-25 09:44, 10:36, and 2026-11-02 17:44). The full table,
and why 5, are in the PR that made the change and in `docs/design/RETRY_RECOVERY.md`.

The new rule has not run on the keyboard: a short-sample stall can't be induced, and it takes
effect on the live install only once it is merged and the uploader is rebuilt.

### 7.2 Non-Latin titles: the crash fix, then the font fallback

Two separate fixes, easy to conflate:

1. **The Unicode crash fix** (bug F work, on `main` before 2026-10-15) stopped a title the
   console code page can't encode from **crashing** the renderer. It did nothing for the
   display. On hardware that evening, `トーキョーレギー` rendered as empty boxes.
2. **The font fallback** (`7c93850`, #10, merged with `fix/device-selection`) is what makes
   the characters **display**. Segoe UI has no kana, Han, Hangul or `♪`, and Pillow does no
   font fallback of its own. Metadata text is now drawn character by character from a
   chain: Segoe UI, then Yu Gothic, Meiryo, Microsoft YaHei, Microsoft JhengHei, Malgun
   Gothic, Segoe UI Symbol, whichever are installed. All runs share Segoe UI's baseline.
   Confirmed on hardware 2026-10-15 23:52: the Morse band and Takanaka both displayed.

**What the "Morse band" actually sends** (`renderer.log`): dot U+FF65 HALFWIDTH KATAKANA
MIDDLE DOT, dash U+FF0D FULLWIDTH HYPHEN-MINUS, letters separated by single ASCII spaces,
no word separator. Segoe UI has neither character; Yu Gothic has both.

**Decisions (feature/font-fallback, 2026-10-16):**

- **No fontTools.** Whether a font has a glyph is decided by drawing the character and
  comparing it with the font's own missing-glyph box, cached per (font, character). It is
  indirect, but it worked on hardware and needs nothing beyond Pillow, so the PyInstaller
  build and `requirements.txt` stay unchanged. Revisit only if a concrete misclassified
  character turns up.
- **CJK line breaks.** A line may break between any two kana or Han characters, with a
  minimal kinsoku rule (no line starts with `ー`, `・`, `、`, `。` or a small kana). Hangul
  still breaks at spaces. U+FF65 is deliberately *not* a break-anywhere character, so a
  Morse letter is never split across lines.
- **Latin-only frames are pixel-identical** to `main` before the fallback (tested).
- **Han unification is not handled.** Yu Gothic comes before YaHei, so a Chinese title is
  drawn with Japanese glyph shapes. They are readable, but not the native forms. Picking a font per string by
  language would need a language guess the metadata doesn't provide.
- **Morse subtitle:** a field that is entirely Morse and decodes completely gets the
  decoded text as a smaller line (`--no-morse-subtitle` turns it off). With single spaces
  only, the band's title decodes as `LETITGO`; word gaps can't be recovered.
  A field with room for one line only (in practice the album, under a Morse title and
  artist) shows just the decoded text: a truncated Morse line said almost nothing.

---

## 8. Intentional behaviour — not bugs

Do not "fix" these without reading §4 first:

- Static total duration rather than a live progress bar.
- Standby clock captured on entry rather than refreshed every minute.
- Two identical frames used to display a static image.
- Full-frame uploads and continuous watch mode.
- A separate scan-only BLE probe.
- Historical filenames and retained reference experiments.

---

## 9. Method that worked — worth preserving

The project stopped making progress by guessing and started making progress when it adopted a
strict discipline:

1. **Reconstruct offline before touching hardware.** Build the packet model in a dry-run lab,
   prove it reproduces a *real captured packet byte-for-byte*, and only then consider sending
   anything.
2. **Change one variable at a time.** The RGB565 patch was deliberately reverted before testing
   the HID framing fix, so the result was interpretable.
3. **Prefer a known-good reference over inference.** The `.pklg` capture of a working
   PocketWuque upload was worth more than all the speculative protocol reasoning before it.
4. **Treat a clean failure as a result.** "All 13,789 chunks ACKed and the screen still broke"
   was the finding that redirected the whole project.
5. **Keep the working installation isolated** from experimental work.

---

## 10. Open questions

- Does removing the duplicate second frame cut upload time to ~4 s? (Most likely quick win.)
- What is the proper static-image GATT command inside the PocketWuque APK, and how does it
  differ from the GIF-derived path in use now? (Recoverable solo with JADX.)
- What do `0x110014F2` and `0x110014F4` actually land on in this unit's firmware?
- Where is there RAM that is *provably* owned and safe for a trampoline and counter?
- Is the tick hook reentrant via the UART path, and how is quiescence established before
  restoring?
- Can the memory service be opened **without** stopping the production uploader, or must the
  stack always be stopped first? (Stopping it works; sharing it is unresolved.)
- What is in the 452-byte DLX media header? Pixels are replaced in a captured header that is
  otherwise not understood — a frame integrity field or metadata in there is a candidate cause
  of the occasional fragmentation glitch.
- What is the unknown 5th field (bytes 5–6) in the `0xFF` telemetry payload?
