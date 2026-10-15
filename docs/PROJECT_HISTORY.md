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
Both addresses were stable across two keyboard power cycles. **What `1B:A3:A9:10:13:5D`
actually is remains unknown.** It connects and has no media service; the fix on
`fix/device-selection` logs the services it does expose, so the next run against it should
say more. Fixed on that branch by qualifying a device by service rather than name; not yet
run on hardware.

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

Source: `logsle.log`, `renderer.log` and `startup_supervisor.log` from the
`tiga-fresh-test` install (a clone of `main` after the bug A merge), 21:05–21:49 local
time, plus Michi's observations at the keyboard. Times below are local (UTC+2).

**Measured uploads.** Stream 220,649 bytes / 1,239 writes / 53 PROGRESS blocks,
intra-block pace 0 ms. 19 uploads completed, between **8.42 s and 10.01 s**. The two
quoted that evening were 8.71 s and 9.85 s. The 8.71 s run was linear at about
**0.154 s per block** (block 53 ACKed at 8.15 s), then ~0.56 s waiting for completion.
The two fastest (8.71 s, 8.42 s) were each the first upload on a new connection; the
other 17, later uploads on the same connection, took 9.49–10.01 s. That is two samples
and may be coincidence; it is recorded, not explained.

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

So the recovery instruction now says: use the power switch, not USB; if the display stays
stuck, disconnect the battery. Tonight's record does not say whether the power switch
alone would have been enough after an interrupted transfer. Unplugging USB is ruled out;
nothing else about "how deep a power cycle is needed" is established.

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
  boxes, without crashing the renderer. The boxes are a font-fallback gap, not a crash.

**Not verified on hardware:**

- **#5 (bug B):** a restart with Spotify already playing released standby in 2 s through
  the *old* "timeline advancing" path. The new 15-second rule never fired. The original
  bug was only ever reproduced synthetically, and **no real-world provider is known to
  report a frozen timeline.** Fixed in code, unproven in the field.
- **#8 (bug E):** paths with spaces. Not tried.

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
