"""Build one table row per BLE `UPLOAD START` from the stack's logs.

Analysis only: reads log files, never opens Bluetooth. Standard library only.

    py tools\\upload_attempts.py                        # default log locations
    py tools\\upload_attempts.py --out docs\\investigations\\upload_attempts.csv

Inputs (each optional; missing files are skipped):
  --install NAME=DIR   a logs directory holding ble.log*, startup_supervisor.log*,
                       renderer.log*. Default: live=..\\tiga-fresh-test\\logs, dev=logs
  --context CSV        what the logs can't say: how each stack start came about and
                       whether the battery was disconnected before it. One row per
                       supervisor launch / halt clear. Default:
                       docs\\investigations\\stack_events_context.csv

`ble.log` stamps lines in UTC (`...Z`); the run banner carries the local offset, which
is used to print local times. The supervisor and renderer logs are already local.
"""

from __future__ import annotations

import argparse
import csv
import glob
import os
import re
import statistics
import sys
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Overhead between UPLOAD START and the first bulk write when CONTROL ACK times are
# unknown: 150 ms drain + CONTROL1/2 + their ACKs + 2 x 60 ms (§7.1 "Derived figures").
DEFAULT_PRE_BLOCK_S = 0.42
FIXED_PRE_BLOCK_S = 0.27  # drain + the two 60 ms sleeps; add the two ACK times

# Stream geometry (§3.2): the DLX file starts 25 bytes into the stream; PROGRESS is per
# 4096 bytes of DLX. Frame 2 starts at stream offset 110,565.
DLX_OFFSET = 25
FRAME2_STREAM_OFFSET = 110_565

RUN_BANNER = re.compile(
    r"^===== (\d{4}-\d\d-\d\d \d\d:\d\d:\d\d) ([+-]\d\d:\d\d) ble run=(\S+) start ====="
)
STAMP = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)Z (.*)$")
BLOCK = re.compile(r"^\s+block\s+(\d+)/(\d+) ACKed .*elapsed=([\d.]+)s")
CANDIDATE = re.compile(r'^CANDIDATE name="([^"]*)" address=(\S+) rssi=(-?\d+)')
FOUND = re.compile(r"^Found keyboard at (\S+)")
START = re.compile(r"^UPLOAD START frame=(\S+) crc32=(\w+)")
COMPLETE = re.compile(r"^UPLOAD COMPLETE frame=\S+ crc32=\w+ in ([\d.]+)s")
FAILED = re.compile(
    r"^UPLOAD FAILED frame=\S+ crc32=\w+ after ([\d.]+)s: (.*?)(?: \(last ACKed block (\d+)/\d+\))?$"
)
TIMING = re.compile(
    r"^TIMING frame=\S+ blocks_acked=(\d+) median=(\d+)ms(?: p90=(\d+)ms)? max=(\d+)ms "
    r"slowest=block (\d+) control1_ack=(\d+|-)m?s? control2_ack=(\d+|-)m?s?"
)
EXIT = re.compile(r"^EXIT code=(\d+)")
SLOW = re.compile(r"^SLOW block (\d+)/\d+ took (\d+) ms")
WAITING_BLOCK = re.compile(r"PROGRESS after 4K block (\d+)/\d+")

SUP_LINE = re.compile(r"^\ufeff?(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d) (.*)$")
RENDER_FRAME = re.compile(
    r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)[+-]\d\d:\d\d FRAME (\S+) (rendered \((.*)\)|installed crc32=(\w+))"
)


@dataclass
class Upload:
    install: str
    run_id: str
    offset: timedelta
    upload_no: int
    start_utc: datetime
    frame_id: str
    crc32: str
    intra_block_ms: str = ""
    block_timeout_ms: str = ""
    address: str = ""
    rssi: str = ""
    scan_s: str = ""
    connect_s: str = ""
    blocks: dict[int, float] = field(default_factory=dict)
    slow: list[str] = field(default_factory=list)
    end_utc: datetime | None = None
    completed_s: float | None = None
    failed_s: float | None = None
    fail_detail: str = ""
    waiting_block: int | None = None
    last_acked: int | None = None
    timing: tuple | None = None
    exit_code: str = ""


def utc(stamp: str) -> datetime:
    return datetime.strptime(stamp, "%Y-%m-%dT%H:%M:%S").replace(tzinfo=timezone.utc)


def local_naive(stamp: str) -> datetime:
    return datetime.strptime(stamp, "%Y-%m-%d %H:%M:%S")


def files(directory: str, stem: str) -> list[str]:
    # ble.log plus any rotated ble.log.1, ble.log.old, ...; oldest content first is not
    # guaranteed by name, so every parser below sorts by timestamp afterwards.
    return sorted(p for p in glob.glob(os.path.join(directory, stem + "*"))
                  if not p.endswith("_error.log"))


def parse_ble(path: str, install: str) -> list[Upload]:
    uploads: list[Upload] = []
    run_id, offset, upload_no = None, timedelta(hours=2), 0
    header: dict[str, str] = {}
    scan_at = cand_at = found_at = None
    cand_rssi: dict[str, tuple[str, datetime]] = {}
    found_addr = ""
    current: Upload | None = None

    with open(path, encoding="utf-8", errors="replace") as fh:
        for raw in fh:
            line = raw.rstrip("\r\n")
            m = RUN_BANNER.match(line)
            if m:
                run_id = m.group(3)
                sign = 1 if m.group(2)[0] == "+" else -1
                hh, mm = m.group(2)[1:].split(":")
                offset = sign * timedelta(hours=int(hh), minutes=int(mm))
                upload_no, header, current = 0, {}, None
                scan_at = cand_at = found_at = None
                cand_rssi, found_addr = {}, ""
                continue
            if run_id is None:
                continue  # the fallback's pre-bug-F format: no run banners, no stamps
            if line.startswith("Intra-block pace:"):
                header["intra"] = line.split(":", 1)[1].strip().split()[0]
                continue
            if line.startswith("Block timeout   :"):
                header["timeout"] = line.split(":", 1)[1].strip().split()[0]
                continue
            m = BLOCK.match(line)
            if m and current is not None:
                current.blocks[int(m.group(1))] = float(m.group(3))
                continue
            m = STAMP.match(line)
            if not m:
                continue
            at, msg = utc(m.group(1)), m.group(2)

            if msg.startswith("Scanning for"):
                scan_at = scan_at or at
            elif (c := CANDIDATE.match(msg)):
                cand_rssi.setdefault(c.group(2), (c.group(3), at))
            elif (c := FOUND.match(msg)):
                found_at, found_addr = at, c.group(1)
            elif msg.startswith("Connected once"):
                header["connected_at"] = at.isoformat()
            elif (c := START.match(msg)):
                upload_no += 1
                current = Upload(install, run_id, offset, upload_no, at, c.group(1), c.group(2))
                current.intra_block_ms = header.get("intra", "")
                # Before the --block-timeout-ms option existed the timeout was 2500 ms.
                current.block_timeout_ms = header.get("timeout", "2500")
                current.address = found_addr
                if found_addr in cand_rssi:
                    current.rssi, cand_at = cand_rssi[found_addr][0], cand_rssi[found_addr][1]
                else:
                    cand_at = None
                if upload_no == 1:
                    seen = cand_at or found_at
                    if scan_at and seen:
                        current.scan_s = str(int((seen - scan_at).total_seconds()))
                    if seen and "connected_at" in header:
                        conn = datetime.fromisoformat(header["connected_at"])
                        current.connect_s = str(int((conn - seen).total_seconds()))
                uploads.append(current)
            elif current is not None and (c := SLOW.match(msg)):
                current.slow.append(f"b{c.group(1)}={c.group(2)}ms")
            elif current is not None and (c := COMPLETE.match(msg)):
                current.end_utc, current.completed_s = at, float(c.group(1))
            elif current is not None and (c := FAILED.match(msg)):
                current.end_utc, current.failed_s = at, float(c.group(1))
                current.fail_detail = c.group(2)
                w = WAITING_BLOCK.search(msg)
                current.waiting_block = int(w.group(1)) if w else None
                if c.group(3):
                    current.last_acked = int(c.group(3))
            elif current is not None and (c := TIMING.match(msg)):
                current.timing = c.groups()
            elif current is not None and (c := EXIT.match(msg)):
                current.exit_code = c.group(1)
    return uploads


@dataclass
class StackEvent:
    install: str
    at: datetime  # local, naive
    kind: str  # "launched" or "halt cleared"


def parse_supervisor(path: str, install: str) -> tuple[list[StackEvent], list[tuple[datetime, str]]]:
    events, ble_starts = [], []
    with open(path, encoding="utf-8-sig", errors="replace") as fh:
        for raw in fh:
            m = SUP_LINE.match(raw.rstrip("\r\n"))
            if not m:
                continue
            at, msg = local_naive(m.group(1)), m.group(2)
            if "startup supervisor launched" in msg:
                events.append(StackEvent(install, at, "launched"))
            elif msg.startswith("Halt cleared"):
                events.append(StackEvent(install, at, "halt cleared"))
            elif (b := re.match(r"Started ble run=(\S+)", msg)):
                ble_starts.append((at, b.group(1)))
    return events, ble_starts


def parse_renderer(path: str) -> dict[str, dict]:
    frames: dict[str, dict] = {}
    with open(path, encoding="utf-8", errors="replace") as fh:
        for raw in fh:
            m = RENDER_FRAME.match(raw.rstrip("\r\n"))
            if not m:
                continue
            f = frames.setdefault(m.group(2), {})
            if m.group(4) is not None:
                f["reason"] = m.group(4)
            else:
                f["installed"] = datetime.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S")
    return frames


def load_context(path: str) -> dict[tuple[str, str], dict]:
    if not os.path.exists(path):
        return {}
    with open(path, encoding="utf-8", newline="") as fh:
        return {(r["install"], r["stack_event_local"]): r for r in csv.DictReader(fh)}


def percentile(values: list[float], p: float) -> float:
    s = sorted(values)
    return s[min(len(s) - 1, max(0, round(p * (len(s) - 1))))]


def block_stats(u: Upload) -> dict[str, str]:
    """Median/p90/max block latency: TIMING when logged, else 5-block span averages."""
    if u.timing:
        acked, med, p90, mx, slowest, c1, c2 = u.timing
        return {"latency_source": "TIMING", "blocks_acked_timing": acked, "median_ms": med,
                "p90_ms": p90 or "", "max_ms": mx, "slowest_block": slowest,
                "control1_ack_ms": "" if c1 == "-" else c1,
                "control2_ack_ms": "" if c2 == "-" else c2}
    marks = sorted(u.blocks.items())
    spans = []
    prev_b, prev_t = 0, 0.0
    for b, t in marks:
        if b > prev_b:
            spans.append((t - prev_t) / (b - prev_b) * 1000)
        prev_b, prev_t = b, t
    if not spans:
        return {"latency_source": ""}
    return {"latency_source": "5-block averages",
            "median_ms": f"{statistics.median(spans):.0f}",
            "p90_ms": f"{percentile(spans, 0.9):.0f}",
            "max_ms": f"{max(spans):.0f}"}


def last_ack_elapsed(u: Upload) -> float | None:
    """Seconds after the first bulk write at which the last PROGRESS arrived (failures)."""
    if u.failed_s is None or u.waiting_block is None:
        return None
    if u.last_acked and u.last_acked in u.blocks:
        return u.blocks[u.last_acked]
    pre = DEFAULT_PRE_BLOCK_S
    if u.timing and u.timing[5] not in ("-", None) and u.timing[6] not in ("-", None):
        pre = FIXED_PRE_BLOCK_S + (int(u.timing[5]) + int(u.timing[6])) / 1000
    return max(0.0, u.failed_s - int(u.block_timeout_ms or 2500) / 1000 - pre)


def signature(u: Upload) -> str:
    """§7.1: 'slowing' = blocks taking ~1-2.5 s before the stall; 'sudden' = normal pace."""
    if u.failed_s is None:
        return ""
    if u.timing and u.timing[1]:
        if int(u.timing[1]) > 500:
            return "slowing"
        # A normal pace, then one slow block right before the stall (2026-10-18 14:09).
        if int(u.timing[3]) > 1000 and u.timing[4] == u.timing[0]:
            return "sudden after one slow block"
        return "sudden"
    marks = sorted(u.blocks.items())
    if len(marks) >= 2:
        spans = [(t2 - t1) / (b2 - b1) for (b1, t1), (b2, t2) in zip(marks, marks[1:])]
        return "slowing" if max(spans) > 0.5 else "sudden"
    t = last_ack_elapsed(u)
    if t is not None and u.last_acked:
        per = t / u.last_acked
        if per < 0.35:
            return "sudden (inferred from time to failure)"
    if marks and marks[0][1] > 2.0:
        return "slowing"
    return "unclear"


def block_region(block: int | None) -> str:
    if not block:
        return ""
    lo = DLX_OFFSET + (block - 1) * 4096
    hi = DLX_OFFSET + block * 4096 - 1
    if lo <= FRAME2_STREAM_OFFSET <= hi:
        return "contains frame-1/frame-2 boundary"
    return "frame 1" if hi < FRAME2_STREAM_OFFSET else "frame 2"


def build(installs: dict[str, str], context_path: str) -> list[dict]:
    uploads: list[Upload] = []
    events: list[StackEvent] = []
    ble_started: dict[str, datetime] = {}
    frames: dict[tuple[str, str], dict] = {}
    for name, d in installs.items():
        for p in files(d, "ble.log"):
            uploads += parse_ble(p, name)
        for p in files(d, "startup_supervisor.log"):
            ev, starts = parse_supervisor(p, name)
            events += ev
            for at, run in starts:
                ble_started[run] = at
        for p in files(d, "renderer.log"):
            for fid, info in parse_renderer(p).items():
                frames[(name, fid)] = info
    context = load_context(context_path)
    uploads.sort(key=lambda u: u.start_utc)
    events.sort(key=lambda e: e.at)

    rows = []
    prev_in_run: dict[str, Upload] = {}
    last_failure: Upload | None = None
    for u in uploads:
        local = (u.start_utc + u.offset).replace(tzinfo=None)
        run_local = ble_started.get(u.run_id, local)
        ev = None
        for e in events:
            if e.install == u.install and e.at <= run_local:
                ev = e
        ctx = context.get((u.install, ev.at.strftime("%Y-%m-%d %H:%M:%S"))) if ev else None

        if u.upload_no > 1:
            how = "later upload on an open connection"
        elif ctx and ctx.get("start_kind"):
            how = ctx["start_kind"]
        elif ev:
            how = "halt cleared on a running supervisor" if ev.kind == "halt cleared" else "stack start (kind not recorded)"
        else:
            how = "unknown"
        retried = ev is not None and u.upload_no == 1 and ev.kind == "launched" and \
            (run_local - ev.at).total_seconds() > 2 and any(
                r for r, t in ble_started.items() if ev.at <= t < run_local)
        if retried:
            how += " (after a code 10/11 retry)"

        prev = prev_in_run.get(u.run_id)
        since_prev = ""
        if prev and prev.end_utc:
            since_prev = f"{(u.start_utc - prev.end_utc).total_seconds():.0f}"
        since_fail = ""
        if last_failure and last_failure.end_utc:
            since_fail = f"{(u.start_utc - last_failure.end_utc).total_seconds():.0f}"

        info = frames.get((u.install, u.frame_id), {})
        during = 0
        if u.end_utc:
            s, e = local, (u.end_utc + u.offset).replace(tzinfo=None)
            during = sum(1 for (inst, _), f in frames.items()
                         if inst == u.install and "installed" in f and s <= f["installed"] <= e)

        stats = block_stats(u)
        t_last = last_ack_elapsed(u)
        row = {
            "start_local": local.strftime("%Y-%m-%d %H:%M:%S"),
            "install": u.install,
            "run_id": u.run_id,
            "upload_no": u.upload_no,
            "how_started": how,
            "stack_event_local": ev.at.strftime("%Y-%m-%d %H:%M:%S") if ev else "",
            "battery_disconnect_before": (ctx or {}).get("battery_disconnect", "") if u.upload_no == 1 else "",
            "since_prev_upload_on_conn_s": since_prev,
            "since_prev_failed_upload_s": since_fail,
            "address": u.address,
            "rssi": u.rssi,
            "scan_s": u.scan_s,
            "connect_s": u.connect_s,
            "control1_ack_ms": stats.pop("control1_ack_ms", ""),
            "control2_ack_ms": stats.pop("control2_ack_ms", ""),
            "latency_source": stats.get("latency_source", ""),
            "median_ms": stats.get("median_ms", ""),
            "p90_ms": stats.get("p90_ms", ""),
            "max_ms": stats.get("max_ms", ""),
            "slowest_block": stats.get("slowest_block", ""),
            "b5_s": f"{u.blocks[5]:.2f}" if 5 in u.blocks else "",
            "slow_lines": " ".join(u.slow),
            "outcome": "completed" if u.completed_s is not None else ("failed" if u.failed_s is not None else "no result logged"),
            "duration_s": next((f"{s:.2f}" for s in (u.completed_s, u.failed_s) if s is not None), ""),
            "exit_code": u.exit_code,
            "last_acked_block": u.last_acked if u.last_acked is not None else "",
            "failed_waiting_for_block": u.waiting_block or "",
            "failed_block_region": block_region(u.waiting_block),
            "last_ack_elapsed_s": f"{t_last:.2f}" if t_last is not None else "",
            "signature": signature(u),
            "block_timeout_ms": u.block_timeout_ms,
            "intra_block_ms": u.intra_block_ms,
            "frame_id": u.frame_id,
            "crc32": u.crc32,
            "trigger": info.get("reason", ""),
            "frames_installed_during_upload": during if u.end_utc else "",
        }
        rows.append(row)
        prev_in_run[u.run_id] = u
        if u.failed_s is not None:
            last_failure = u
    return rows


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--install", action="append", default=[], metavar="NAME=DIR")
    ap.add_argument("--context", default=os.path.join(REPO, "docs", "investigations", "stack_events_context.csv"))
    ap.add_argument("--out", help="CSV to write (default: stdout)")
    args = ap.parse_args(argv)

    installs = dict(i.split("=", 1) for i in args.install) or {
        "live": os.path.join(os.path.dirname(REPO), "tiga-fresh-test", "logs"),
        "dev": os.path.join(REPO, "logs"),
    }
    rows = build(installs, args.context)
    if not rows:
        print("no UPLOAD START lines found", file=sys.stderr)
        return 1
    out = open(args.out, "w", encoding="utf-8", newline="") if args.out else sys.stdout
    try:
        w = csv.DictWriter(out, fieldnames=list(rows[0]), lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    finally:
        if args.out:
            out.close()
    failed = sum(r["outcome"] == "failed" for r in rows)
    print(f"{len(rows)} uploads, {failed} failed", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
