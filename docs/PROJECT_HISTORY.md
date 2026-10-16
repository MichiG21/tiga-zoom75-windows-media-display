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
- Michi's older notes say 30–60 s, pressing a few keys while disconnected to drain the
  remaining charge

So **30 s is a working minimum, not a proven threshold.** Every recovery instruction now
says: disconnect the battery for at least 30 s, pressing a few keys.

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
needed more than 2.5 s for any block. The longer wait has rescued nothing so far.

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
4. **Could the uploader spot trouble earlier?** *Proposal only, not implemented.* The
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
