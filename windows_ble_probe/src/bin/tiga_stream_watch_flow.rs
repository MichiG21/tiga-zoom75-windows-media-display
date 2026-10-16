use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::Manager;
use futures::StreamExt;
use std::{
    error::Error,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::time::sleep;
use uuid::Uuid;

const TARGET_NAME: &str = "Zoom75 Tiga";
const CHAR_CONTROL: Uuid = Uuid::from_u128(0x1f400001_aab4_14a3_f1ba_f61f35cddbaa);
const CHAR_NOTIFY: Uuid = Uuid::from_u128(0x1f400002_aab4_14a3_f1ba_f61f35cddbaa);
const CHAR_BULK: Uuid = Uuid::from_u128(0x1f400003_aab4_14a3_f1ba_f61f35cddbaa);
const CHAR_NOTIFY2: Uuid = Uuid::from_u128(0x1f400004_aab4_14a3_f1ba_f61f35cddbaa);

const STREAM_LEN: usize = 220_649;
const FILE_START_IN_STREAM: usize = 25;
const CAPTURED_BULK_WRITES: usize = 1_239;

/// Experimental one-frame containers built by tiga_dlx.py (layouts "index" and
/// "fixed"; see docs/DLX_FORMAT.md). Rejected unless --allow-single-frame is given.
/// Never sent to a keyboard yet.
const SINGLE_FRAME_STREAM_LENS: [usize; 2] = [110_565, 110_569];

const CONTROL1: [u8; 24] = [
    0x88,0,0,0,0,0,0x10,0,
    0x09,0x03,0x04,0,0x08,0,0,0xff,
    0xfc,0,0x03,0x5d,0xd0,0x05,0,0,
];
const CONTROL2: [u8; 18] = [
    0x88,0,0,0,0,0,0x0a,0,
    0x08,0x03,0,0xff,0xff,0xff,0,0xff,0xff,0xff,
];

const C1_ACK_A: &[u8] = &[0x88,0,0,0,0,0,0x05,0x0e,0x07,0x02,0x01,0x09,0x03];
const C1_ACK_B: &[u8] = &[0x88,0,0,0,0,0,0x03,0x0f,0x09,0x04,0x02];
const C2_ACK_A: &[u8] = &[0x88,0,0,0,0,0,0x05,0x0f,0x07,0x02,0x01,0x08,0x03];
const C2_ACK_B: &[u8] = &[0x88,0,0,0,0,0,0x03,0x0e,0x08,0x04,0x02];

const PROGRESS: &[u8] = &[0x88,0,0,0,0,0,0x03,0x0b,0x08,0x06,0x05];
const COMPLETE: &[u8] = &[0x88,0,0,0,0,0,0x03,0x0c,0x08,0x06,0x02];

#[derive(Clone)]
struct Chunk {
    value: Vec<u8>,
    ends_4k_block: bool,
}

fn xor8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, b| acc ^ b)
}

// ---- Diagnostics (issue #9) ------------------------------------------------------
//
// The renderer writes `<stream>.frame` (frame_id + CRC-32 of the stream) before it
// replaces the stream. The uploader logs that frame ID at upload start, completion and
// failure, and records each successful delivery in `<stream>.delivered`, which
// STATUS_STACK.ps1 reads. The CRC-32 matches Python's zlib.crc32.

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// UTC timestamp, `YYYY-MM-DDTHH:MM:SSZ`, without pulling in a date crate.
fn utc_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

macro_rules! log {
    ($($arg:tt)*) => { println!("{} {}", utc_now(), format!($($arg)*)) };
}

fn sidecar_path(stream_path: &Path, ext: &str) -> PathBuf {
    let mut name = stream_path.as_os_str().to_owned();
    name.push(ext);
    PathBuf::from(name)
}

fn sidecar_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim())
}

/// Frame ID for a stream, taken from the renderer's sidecar when its CRC matches.
fn frame_id_for(stream_path: &Path, crc: &str) -> String {
    let Ok(text) = fs::read_to_string(sidecar_path(stream_path, ".frame")) else {
        return "unknown(no-sidecar)".into();
    };
    match (sidecar_value(&text, "frame_id"), sidecar_value(&text, "crc32")) {
        (Some(id), Some(c)) if c.eq_ignore_ascii_case(crc) => id.to_string(),
        (Some(id), Some(_)) => format!("unknown(sidecar-is-{id})"),
        _ => "unknown(bad-sidecar)".into(),
    }
}

fn run_id() -> String {
    std::env::var("TIGA_RUN_ID").unwrap_or_default()
}

/// Atomically writes `<stream><ext>` (tmp + rename) so readers never see half a file.
fn write_sidecar(stream_path: &Path, ext: &str, text: &str) {
    let path = sidecar_path(stream_path, ext);
    let tmp = sidecar_path(stream_path, &format!("{ext}.tmp"));
    if let Err(e) = fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, &path)) {
        log!("warning: could not write {}: {e}", path.display());
    }
}

fn record_delivery(stream_path: &Path, frame_id: &str, crc: &str, elapsed_s: f64, timing: Option<&TimingSummary>) {
    write_sidecar(
        stream_path,
        ".delivered",
        &format!(
            "delivered_at={}\nframe_id={frame_id}\ncrc32={crc}\nelapsed_s={elapsed_s:.2}\nrun_id={}\n{}",
            utc_now(),
            run_id(),
            timing.map(TimingSummary::sidecar_lines).unwrap_or_default()
        ),
    );
}

// ---- Per-block timing (PROJECT_HISTORY §7.1) --------------------------------------
//
// Logging only: nothing here changes a write, a timeout or an exit code. A block's
// latency is the time from the previous PROGRESS (for block 1, the start of the bulk
// writes) to its own PROGRESS, so it includes that block's writes. Healthy blocks take
// ~0.15 s; in the slowing failures they took ~1.2-2.5 s each.

/// A block slower than this gets a `SLOW` line.
const SLOW_BLOCK_MS: f64 = 1000.0;
/// On the first upload of a connection, block 5 ACKed later than this (elapsed since the
/// bulk writes started) gets an early warning. Healthy first uploads: 0.71-0.85 s; all
/// three slowing runs: 2.35-2.97 s.
const FIRST_UPLOAD_BLOCK5_WARN_S: f64 = 1.5;
const EARLY_WARNING_BLOCK: usize = 5;

#[derive(Default)]
struct UploadTimings {
    control1_ack_ms: Option<f64>,
    control2_ack_ms: Option<f64>,
    /// Latency of each ACKed block, in block order (index 0 = block 1).
    block_ms: Vec<f64>,
}

struct TimingSummary {
    blocks: usize,
    median_ms: f64,
    p90_ms: f64,
    max_ms: f64,
    slowest_block: usize,
    control1_ack_ms: Option<f64>,
    control2_ack_ms: Option<f64>,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn fmt_ms(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "-".into())
}

/// Median (mean of the two middle values for an even count) and nearest-rank p90.
fn median_p90(values: &[f64]) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    let median = if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 };
    let rank = (n * 9).div_ceil(10).max(1);
    Some((median, v[rank - 1]))
}

impl UploadTimings {
    fn summary(&self) -> TimingSummary {
        let (median_ms, p90_ms) = median_p90(&self.block_ms).unwrap_or((0.0, 0.0));
        // The first block wins a tie, so a stall is attributed where it started.
        let (slowest_block, max_ms) = self
            .block_ms
            .iter()
            .enumerate()
            .fold((0, 0.0f64), |best, (i, &v)| if v > best.1 { (i + 1, v) } else { best });
        TimingSummary {
            blocks: self.block_ms.len(),
            median_ms,
            p90_ms,
            max_ms,
            slowest_block,
            control1_ack_ms: self.control1_ack_ms,
            control2_ack_ms: self.control2_ack_ms,
        }
    }
}

impl TimingSummary {
    fn log_line(&self, frame_id: &str) -> String {
        format!(
            "TIMING frame={frame_id} blocks_acked={} median={:.0}ms p90={:.0}ms max={:.0}ms slowest=block {} \
             control1_ack={}ms control2_ack={}ms",
            self.blocks,
            self.median_ms,
            self.p90_ms,
            self.max_ms,
            self.slowest_block,
            fmt_ms(self.control1_ack_ms),
            fmt_ms(self.control2_ack_ms)
        )
    }

    fn sidecar_lines(&self) -> String {
        format!(
            "block_median_ms={:.0}\nblock_p90_ms={:.0}\nblock_max_ms={:.0}\nslowest_block={}\n\
             control1_ack_ms={}\ncontrol2_ack_ms={}\n",
            self.median_ms,
            self.p90_ms,
            self.max_ms,
            self.slowest_block,
            fmt_ms(self.control1_ack_ms),
            fmt_ms(self.control2_ack_ms)
        )
    }
}

/// The `SLOW` line for one block, if it deserves one.
fn slow_block_line(block_no: usize, total_blocks: usize, latency_ms: f64, elapsed_s: f64) -> Option<String> {
    (latency_ms > SLOW_BLOCK_MS).then(|| {
        format!("SLOW block {block_no}/{total_blocks} took {latency_ms:.0} ms (elapsed={elapsed_s:.2}s)")
    })
}

/// The early warning for a first-on-connection upload whose block 5 came late.
fn first_upload_warning(first_on_connection: bool, block_no: usize, elapsed_s: f64) -> Option<String> {
    (first_on_connection && block_no == EARLY_WARNING_BLOCK && elapsed_s > FIRST_UPLOAD_BLOCK5_WARN_S).then(|| {
        format!("WARNING first upload is slow (block 5 at {elapsed_s:.2} s; healthy 0.7–0.85 s)")
    })
}

// ---- Failure classes and exit codes (issue #4) ------------------------------------
//
// The boundary is CONTROL2: it puts the stock firmware into its upload state (the roar
// animation). Before it, the keyboard is untouched and a retry is safe. From the moment
// CONTROL2 is *attempted* (a WriteWithoutResponse that errors may still have gone out),
// the keyboard may hold a partial media session and must be power-cycled first.
//
// The session marker `<stream>.session` is written just before CONTROL2, updated after
// every ACKed block and removed on COMPLETE. It survives a kill or crash, so the
// supervisor can tell an open session apart from a clean failure even when the exit
// code is meaningless (taskkill gives 1).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    /// No adapter, keyboard not found, connect failed, or the link failed before CONTROL2.
    Unreachable,
    /// Connected, but service discovery / characteristics / subscriptions failed.
    Gatt,
    /// CONTROL2 was sent and the transfer did not finish.
    Interrupted,
    /// Every block was ACKed but COMPLETE never arrived.
    NoCompletion,
    /// Bad arguments, missing or invalid stream, internal error.
    Fatal,
}

impl Failure {
    fn code(self) -> u8 {
        match self {
            Failure::Unreachable => 10,
            Failure::Gatt => 11,
            Failure::Interrupted => 20,
            Failure::NoCompletion => 21,
            Failure::Fatal => 30,
        }
    }

    fn meaning(self) -> &'static str {
        match self {
            Failure::Unreachable => "keyboard not reachable (no adapter, not found, connect failed, or link lost before the media session opened); safe to retry",
            Failure::Gatt => "connected, but the keyboard's GATT service was missing or unusable; safe to retry",
            Failure::Interrupted => "transfer interrupted after CONTROL2; the keyboard may hold a partial media session; disconnect the keyboard's battery for at least 30 s before retrying (there is no power switch; unplugging USB doesn't count)",
            Failure::NoCompletion => "all blocks sent but the keyboard never confirmed completion; disconnect the keyboard's battery for at least 30 s before retrying (there is no power switch; unplugging USB doesn't count)",
            Failure::Fatal => "bad arguments, missing or invalid stream, or internal error; not retried",
        }
    }
}

#[derive(Debug)]
struct Failed {
    kind: Failure,
    detail: String,
    frame_id: Option<String>,
    last_acked_block: Option<usize>,
}

impl Failed {
    fn new(kind: Failure, detail: impl Display) -> Self {
        Failed { kind, detail: detail.to_string(), frame_id: None, last_acked_block: None }
    }
}

/// `result.class(Failure::Gatt)?` turns any error into a classified failure.
trait Classify<T> {
    fn class(self, kind: Failure) -> Result<T, Failed>;
}

impl<T, E: Display> Classify<T> for Result<T, E> {
    fn class(self, kind: Failure) -> Result<T, Failed> {
        self.map_err(|e| Failed::new(kind, e))
    }
}

/// Which class a failure inside an upload belongs to, given how far it got.
fn classify_upload_failure(control2_attempted: bool, all_blocks_acked: bool) -> Failure {
    match (control2_attempted, all_blocks_acked) {
        (false, _) => Failure::Unreachable,
        (true, false) => Failure::Interrupted,
        (true, true) => Failure::NoCompletion,
    }
}

fn write_session_marker(stream_path: &Path, frame_id: &str, last_acked: usize, total: usize) {
    write_sidecar(
        stream_path,
        ".session",
        &format!(
            "updated_at={}\nframe_id={frame_id}\nlast_acked_block={last_acked}\ntotal_blocks={total}\nrun_id={}\n",
            utc_now(),
            run_id()
        ),
    );
}

fn read_session_marker(stream_path: &Path) -> Option<String> {
    fs::read_to_string(sidecar_path(stream_path, ".session")).ok()
}

fn clear_session_marker(stream_path: &Path) {
    let path = sidecar_path(stream_path, ".session");
    if let Err(e) = fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            log!("warning: could not remove {}: {e}", path.display());
        }
    }
}

/// Human-readable record of why this run exited; the supervisor copies it into its
/// halt state when the run_id matches the process it started.
fn record_exit(stream_path: &Path, f: &Failed) {
    write_sidecar(
        stream_path,
        ".exit",
        &format!(
            "exited_at={}\nrun_id={}\ncode={}\nmeaning={}\ndetail={}\nframe_id={}\nlast_acked_block={}\n",
            utc_now(),
            run_id(),
            f.kind.code(),
            f.kind.meaning(),
            f.detail.replace(['\r', '\n'], " "),
            f.frame_id.as_deref().unwrap_or(""),
            f.last_acked_block.map(|b| b.to_string()).unwrap_or_default()
        ),
    );
}

fn validate_stream(stream: &[u8], allow_single_frame: bool) -> Result<(), Box<dyn Error>> {
    let single = SINGLE_FRAME_STREAM_LENS.contains(&stream.len());
    if single && !allow_single_frame {
        return Err(format!(
            "stream size {} is an experimental single-frame container; pass --allow-single-frame",
            stream.len()
        ).into());
    }
    if stream.len() != STREAM_LEN && !single {
        return Err(format!("stream size {} != expected {}", stream.len(), STREAM_LEN).into());
    }
    // 88 00 00 00 <u24 BE: bytes after the envelope> 00. For the captured stream: 03 5d e1.
    if stream.get(0..4) != Some(&[0x88,0,0,0][..])
        || stream[7] != 0
        || be24(&stream[4..7]) != stream.len() - 8
    {
        return Err("outer 0x88 envelope mismatch".into());
    }
    if stream.get(8..25) != Some(&[
        0x08,0x05,0x01,0,0x09,0,0xff,0xff,0xff,0,0xff,0xff,0xff,0x02,0x02,0xff,0xff
    ][..]) {
        return Err("upload preamble mismatch".into());
    }
    if stream.get(25..33) != Some(&[0,0x44,0x4c,0x58,0xfc,0xff,0,0][..]) {
        return Err("DLX prefix mismatch".into());
    }
    if stream.get(stream.len()-4..) != Some(&[0xfc,0xff,0,0][..]) {
        return Err("DLX trailer mismatch".into());
    }
    Ok(())
}

fn be24(b: &[u8]) -> usize {
    ((b[0] as usize) << 16) | ((b[1] as usize) << 8) | b[2] as usize
}

/// CONTROL1 announces the DLX file length (u24 BE at bytes 18..21). For the captured
/// 2-frame stream this returns CONTROL1 unchanged.
fn control1_for(stream_len: usize) -> [u8; 24] {
    let file_len = stream_len - FILE_START_IN_STREAM;
    let mut c = CONTROL1;
    c[18] = (file_len >> 16) as u8;
    c[19] = (file_len >> 8) as u8;
    c[20] = file_len as u8;
    c
}

fn build_chunks(stream: &[u8], allow_single_frame: bool) -> Result<Vec<Chunk>, Box<dyn Error>> {
    validate_stream(stream, allow_single_frame)?;
    let mut chunks = Vec::new();
    let mut off = 0usize;

    while off < stream.len() {
        let mut n = 181usize.min(stream.len() - off);

        if off >= FILE_START_IN_STREAM {
            let file_rel = off - FILE_START_IN_STREAM;
            let to_boundary = 4096 - (file_rel % 4096);
            n = n.min(to_boundary);
        }

        let data = &stream[off..off+n];
        let mut value = Vec::with_capacity(n + 1);
        value.extend_from_slice(data);
        value.push(xor8(data));

        let new_off = off + n;
        let ends_4k_block = new_off >= FILE_START_IN_STREAM
            && ((new_off - FILE_START_IN_STREAM) % 4096 == 0);

        chunks.push(Chunk { value, ends_4k_block });
        off = new_off;
    }

    // The captured stream must chunk exactly as PocketWuque did.
    if stream.len() == STREAM_LEN {
        if chunks.len() != CAPTURED_BULK_WRITES {
            return Err(format!("chunk count {} != expected {}", chunks.len(), CAPTURED_BULK_WRITES).into());
        }

        let blocks = chunks.iter().filter(|c| c.ends_4k_block).count();
        if blocks != 53 {
            return Err(format!("detected {blocks} full 4096-byte blocks; expected 53").into());
        }
    }

    Ok(chunks)
}

async fn wait_for_ack_pair<S>(
    notifications: &mut S,
    a_expected: &[u8],
    b_expected: &[u8],
    timeout_ms: u64,
) -> Result<(), Box<dyn Error>>
where
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    let timeout = sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout);
    let (mut a, mut b) = (false, false);

    loop {
        tokio::select! {
            _ = &mut timeout => break,
            n = notifications.next() => {
                let Some(n) = n else { return Err("notification stream ended".into()); };
                if n.value.as_slice() == a_expected { a = true; }
                if n.value.as_slice() == b_expected { b = true; }
                if a && b { return Ok(()); }
            }
        }
    }

    Err("control ACK pair timed out".into())
}

async fn wait_for_progress<S>(
    notifications: &mut S,
    timeout_ms: u64,
) -> Result<bool, Box<dyn Error>>
where
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    let timeout = sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => return Ok(false),
            n = notifications.next() => {
                let Some(n) = n else { return Err("notification stream ended".into()); };
                if n.value.as_slice() == PROGRESS {
                    return Ok(true);
                }
                if n.value.as_slice() == COMPLETE {
                    // Completion before the final partial block would be odd, but do not
                    // treat it as a progress ACK.
                    println!("warning: completion arrived while waiting for progress");
                }
            }
        }
    }
}

async fn wait_for_completion<S>(
    notifications: &mut S,
    timeout_ms: u64,
) -> Result<bool, Box<dyn Error>>
where
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    let timeout = sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => return Ok(false),
            n = notifications.next() => {
                let Some(n) = n else { return Err("notification stream ended".into()); };
                if n.value.as_slice() == COMPLETE {
                    return Ok(true);
                }
            }
        }
    }
}

/// How far one upload got; turns any error into a classified failure.
struct UploadProgress<'a> {
    stream_path: &'a Path,
    frame_id: &'a str,
    total_blocks: usize,
    control2_attempted: bool,
    last_acked_block: usize,
    all_blocks_acked: bool,
}

impl UploadProgress<'_> {
    fn fail(&self, detail: impl Display) -> Failed {
        let kind = classify_upload_failure(self.control2_attempted, self.all_blocks_acked);
        Failed {
            kind,
            detail: if self.control2_attempted {
                format!("{detail} (last ACKed block {}/{})", self.last_acked_block, self.total_blocks)
            } else {
                detail.to_string()
            },
            frame_id: Some(self.frame_id.to_string()),
            last_acked_block: self.control2_attempted.then_some(self.last_acked_block),
        }
    }
}

/// The renderer re-rendered pixels identical to the last completed upload (Standby
/// pressed twice in the same minute, say). Nothing is sent, but the new frame ID is
/// recorded as delivered: the keyboard already shows exactly these bytes, and the
/// control utility waits for its frame ID before shutting down (issue #7).
fn note_already_displayed(stream_path: &Path, last_crc: &str, last_frame_id: &mut String) -> bool {
    if last_crc.is_empty() {
        return false;
    }
    let id = frame_id_for(stream_path, last_crc);
    if id.starts_with("unknown(") || id == *last_frame_id {
        return false;
    }
    log!("ALREADY DISPLAYED frame={id} crc32={last_crc} (identical to the last upload)");
    record_delivery(stream_path, &id, last_crc, 0.0, None);
    *last_frame_id = id;
    true
}

#[allow(clippy::too_many_arguments)]
async fn upload_stream<S>(
    peripheral: &btleplug::platform::Peripheral,
    control: &btleplug::api::Characteristic,
    bulk: &btleplug::api::Characteristic,
    notifications: &mut S,
    chunks: &[Chunk],
    intra_block_ms: u64,
    block_timeout_ms: u64,
    stream_path: &Path,
    frame_id: &str,
    first_on_connection: bool,
    timings: &mut UploadTimings,
) -> Result<(), Failed>
where
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    let mut p = UploadProgress {
        stream_path,
        frame_id,
        total_blocks: chunks.iter().filter(|c| c.ends_4k_block).count(),
        control2_attempted: false,
        last_acked_block: 0,
        all_blocks_acked: false,
    };

    // Drain stale notifications.
    let drain = sleep(Duration::from_millis(150));
    tokio::pin!(drain);
    loop {
        tokio::select! {
            _ = &mut drain => break,
            n = notifications.next() => {
                if n.is_none() { return Err(p.fail("notification stream ended")); }
            }
        }
    }

    let stream_len = chunks.iter().map(|c| c.value.len() - 1).sum::<usize>();
    let control1_sent = Instant::now();
    peripheral.write(control, &control1_for(stream_len), WriteType::WithoutResponse).await
        .map_err(|e| p.fail(e))?;
    wait_for_ack_pair(notifications, C1_ACK_A, C1_ACK_B, 1500).await
        .map_err(|e| p.fail(e))?;
    timings.control1_ack_ms = Some(ms(control1_sent.elapsed()));

    sleep(Duration::from_millis(60)).await;

    // ---- Point of no return: from here a failure leaves a partial media session. ----
    write_session_marker(p.stream_path, p.frame_id, 0, p.total_blocks);
    p.control2_attempted = true;
    let control2_sent = Instant::now();
    peripheral.write(control, &CONTROL2, WriteType::WithoutResponse).await
        .map_err(|e| p.fail(e))?;
    wait_for_ack_pair(notifications, C2_ACK_A, C2_ACK_B, 1500).await
        .map_err(|e| p.fail(e))?;
    timings.control2_ack_ms = Some(ms(control2_sent.elapsed()));

    sleep(Duration::from_millis(60)).await;

    let started = Instant::now();
    let mut previous_ack = started;
    let total_blocks = p.total_blocks;

    for (idx, chunk) in chunks.iter().enumerate() {
        peripheral.write(bulk, &chunk.value, WriteType::WithoutResponse).await
            .map_err(|e| p.fail(e))?;

        if intra_block_ms > 0 {
            sleep(Duration::from_millis(intra_block_ms)).await;
        }

        if chunk.ends_4k_block {
            let block_no = p.last_acked_block + 1;

            let ok = wait_for_progress(notifications, block_timeout_ms).await
                .map_err(|e| p.fail(e))?;
            if !ok {
                return Err(p.fail(format!(
                    "timed out waiting for PROGRESS after 4K block {block_no}/{total_blocks}"
                )));
            }
            let acked = Instant::now();
            let latency_ms = ms(acked - previous_ack);
            previous_ack = acked;
            timings.block_ms.push(latency_ms);
            p.last_acked_block = block_no;
            write_session_marker(p.stream_path, p.frame_id, block_no, total_blocks);

            let elapsed_s = (acked - started).as_secs_f64();
            if let Some(line) = slow_block_line(block_no, total_blocks, latency_ms, elapsed_s) {
                log!("{line}");
            }
            if let Some(line) = first_upload_warning(first_on_connection, block_no, elapsed_s) {
                log!("{line}");
            }

            if block_no % 5 == 0 || block_no == total_blocks {
                println!(
                    "  block {:2}/{} ACKed  writes={:4}/{}  elapsed={:.2}s",
                    block_no,
                    total_blocks,
                    idx + 1,
                    chunks.len(),
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }

    p.all_blocks_acked = true;
    println!("All {total_blocks} progress blocks ACKed; waiting for final completion...");

    if !wait_for_completion(notifications, 8000).await.map_err(|e| p.fail(e))? {
        return Err(p.fail(format!("no completion after all {total_blocks} block ACKs")));
    }

    // The keyboard closed the session itself.
    clear_session_marker(p.stream_path);
    Ok(())
}

// ---- Device selection (issue #14) -------------------------------------------------
//
// Two peripherals near this keyboard advertise a name containing "zoom75"; only one
// exposes the media service. A name match therefore only makes a device a candidate.
// A candidate qualifies once it is connected and shows the media service with all four
// 1f4000xx characteristics. One that does not is disconnected and skipped for the rest
// of the run.
//
// Order: the device that last completed an upload (cached in `<stream>.device`) is
// tried as soon as the scan sees it. Everything else waits until the scan window has
// closed, so every candidate is logged, and is then tried by name: exactly
// "Zoom75 Tiga" first, other "zoom75" names next, "... BT<n>" names (the keyboard's
// typing connections) last; discovery order within each group.
//
// If every candidate seen was rejected, the scan runs one more window in case the
// keyboard was slow to advertise; it is still rejected-for-this-run devices only that
// are skipped.

const SERVICE_MEDIA: Uuid = Uuid::from_u128(0x1f40eaf8_aab4_14a3_f1ba_f61f35cddbaa);
const SCAN_WINDOW: Duration = Duration::from_secs(5);
/// If nothing at all has been seen when the window closes, keep scanning this long and
/// try the first candidate that appears. Right after a stack restart on 2026-10-15 the
/// keyboard stayed invisible for 20-38 s, so two 5 s scans failed with code 10.
const NOTHING_SEEN_LIMIT: Duration = Duration::from_secs(15);
const SCAN_POLL: Duration = Duration::from_millis(250);

struct Seen<D> {
    address: String,
    name: String,
    rssi: Option<i16>,
    dev: D,
}

enum Probe<L> {
    /// Connected and has the media service; `L` is what the upload needs from it.
    Qualified(L),
    /// Could not connect. The device may still be the right one.
    Unreachable(String),
    /// Connected, but it is not the keyboard's media endpoint. Disconnected again.
    Unqualified(String),
}

/// What device selection needs from Bluetooth. Faked in the tests.
trait Radio {
    type Dev;
    type Link;
    /// Every peripheral the running scan has seen so far.
    async fn visible(&mut self) -> Result<Vec<Seen<Self::Dev>>, String>;
    async fn probe(&mut self, dev: &Self::Dev) -> Probe<Self::Link>;
    /// Time since the scan started.
    fn elapsed(&self) -> Duration;
    async fn pause(&mut self, d: Duration);
    /// Start a fresh scan window: `elapsed` counts from zero again.
    async fn rescan(&mut self);
}

const SCAN_ROUNDS: u32 = 2;

struct Chosen<D, L> {
    address: String,
    name: String,
    dev: D,
    link: L,
}

fn same_address(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn is_candidate(name: &str, address: &str, cached: Option<&str>) -> bool {
    name == TARGET_NAME
        || name.to_ascii_lowercase().contains("zoom75")
        // The cached device qualifies to be tried even before its name has been seen.
        || cached.is_some_and(|c| same_address(c, address))
}

/// `"Zoom75 Tiga BT1"`, `"... BT2"`, ...: the keyboard's Bluetooth typing connections
/// (pairing slots), seen on hardware 2026-10-15. Still candidates, in case a firmware
/// variant names things differently, but tried last.
fn is_typing_slot_name(name: &str) -> bool {
    let Some((_, suffix)) = name.rsplit_once(' ') else { return false };
    let suffix = suffix.as_bytes();
    suffix.len() > 2
        && suffix[..2].eq_ignore_ascii_case(b"BT")
        && suffix[2..].iter().all(u8::is_ascii_digit)
}

/// Lower is tried first: the cached device, then the exact display name, then any
/// other "zoom75" name, then the typing slots.
fn candidate_tier(name: &str, address: &str, cached: Option<&str>) -> u8 {
    if cached.is_some_and(|c| same_address(c, address)) {
        0
    } else if name == TARGET_NAME {
        1
    } else if is_typing_slot_name(name) {
        3
    } else {
        2
    }
}

async fn select_device<R: Radio>(radio: &mut R, cached: Option<&str>) -> Result<Chosen<R::Dev, R::Link>, Failed> {
    let mut known: Vec<String> = Vec::new(); // candidate addresses in the order first seen
    let mut tried: Vec<String> = Vec::new();
    let mut outcomes: Vec<String> = Vec::new();
    let mut any_unreachable = false;
    let mut extended = false;
    let mut round = 1;

    loop {
        let mut untried = Vec::new();
        for s in radio.visible().await.class(Failure::Unreachable)? {
            if !is_candidate(&s.name, &s.address, cached) {
                continue;
            }
            if !known.iter().any(|k| same_address(k, &s.address)) {
                let is_cached = cached.is_some_and(|c| same_address(c, &s.address));
                log!(
                    "CANDIDATE name={:?} address={} rssi={}{}",
                    s.name,
                    s.address,
                    s.rssi.map(|r| r.to_string()).unwrap_or_else(|| "?".into()),
                    if is_cached { " (last device that completed an upload)" } else { "" }
                );
                known.push(s.address.clone());
            }
            if !tried.iter().any(|t| same_address(t, &s.address)) {
                untried.push(s);
            }
        }

        let window_closed = radio.elapsed() >= SCAN_WINDOW;
        let rank = |s: &Seen<R::Dev>| {
            let order = known.iter().position(|k| same_address(k, &s.address));
            (candidate_tier(&s.name, &s.address, cached), order)
        };
        untried.sort_by_key(rank);
        let next = match untried.into_iter().next() {
            Some(s) if window_closed || cached.is_some_and(|c| same_address(c, &s.address)) => Some(s),
            _ => None,
        };

        let Some(s) = next else {
            if window_closed && tried.is_empty() {
                if radio.elapsed() >= NOTHING_SEEN_LIMIT {
                    break;
                }
                if !extended {
                    extended = true;
                    log!(
                        "Nothing seen in {} s; scanning up to {} s (the keyboard can be invisible for a while after a restart)",
                        SCAN_WINDOW.as_secs(),
                        NOTHING_SEEN_LIMIT.as_secs()
                    );
                }
            } else if window_closed {
                if round >= SCAN_ROUNDS {
                    break;
                }
                round += 1;
                log!(
                    "RESCAN: all {} candidate(s) seen so far were rejected; scanning another {} s for others",
                    tried.len(),
                    SCAN_WINDOW.as_secs()
                );
                radio.rescan().await;
                continue;
            }
            radio.pause(SCAN_POLL).await;
            continue;
        };

        tried.push(s.address.clone());
        match radio.probe(&s.dev).await {
            Probe::Qualified(link) => {
                log!(
                    "SELECTED {} ({:?}) after trying {} candidate(s)",
                    s.address,
                    s.name,
                    tried.len()
                );
                return Ok(Chosen { address: s.address, name: s.name, dev: s.dev, link });
            }
            Probe::Unreachable(why) => {
                log!("REJECTED {} for this run: {why}", s.address);
                outcomes.push(format!("{} {why}", s.address));
                any_unreachable = true;
            }
            Probe::Unqualified(why) => {
                log!("REJECTED {} for this run: {why}", s.address);
                outcomes.push(format!("{} {why}", s.address));
            }
        }
    }

    if tried.is_empty() {
        return Err(Failed::new(
            Failure::Unreachable,
            format!("{TARGET_NAME} not found (no candidate seen in a {} s scan)", NOTHING_SEEN_LIMIT.as_secs()),
        ));
    }
    // 11 only when every candidate was reached and none had the service. If any could
    // not be reached, that one may have been the keyboard.
    let kind = if any_unreachable { Failure::Unreachable } else { Failure::Gatt };
    Err(Failed::new(
        kind,
        format!(
            "tried {} candidate(s) over {round} scan(s), none qualified: {}",
            tried.len(),
            outcomes.join("; ")
        ),
    ))
}

/// Every service a device exposes, with its characteristics, for the rejection log.
/// The second "zoom75" device on the test PC is "Zoom75 Tiga BT1", the typing connection;
/// on 2026-10-15 it failed to connect, so what it exposes has still not been logged.
fn describe_services(services: &std::collections::BTreeSet<btleplug::api::Service>) -> String {
    if services.is_empty() {
        return "exposes no services".into();
    }
    let list: Vec<String> = services
        .iter()
        .map(|s| {
            let chars: Vec<String> = s.characteristics.iter().map(|c| c.uuid.to_string()).collect();
            format!("{} [{}]", s.uuid, chars.join(", "))
        })
        .collect();
    format!("exposes {} service(s): {}", services.len(), list.join("; "))
}

/// The four characteristics of the media service, or why this device lacks them.
fn media_link(services: &std::collections::BTreeSet<btleplug::api::Service>) -> Result<GattLink, String> {
    let Some(service) = services.iter().find(|s| s.uuid == SERVICE_MEDIA) else {
        return Err(format!("connected, but no media service {SERVICE_MEDIA}; {}", describe_services(services)));
    };
    let find = |uuid: Uuid| service.characteristics.iter().find(|c| c.uuid == uuid).cloned();
    match (find(CHAR_CONTROL), find(CHAR_NOTIFY), find(CHAR_BULK), find(CHAR_NOTIFY2)) {
        (Some(control), Some(notify1), Some(bulk), Some(notify2)) => Ok(GattLink { control, notify1, bulk, notify2 }),
        _ => {
            let missing: Vec<String> = [CHAR_CONTROL, CHAR_NOTIFY, CHAR_BULK, CHAR_NOTIFY2]
                .iter()
                .filter(|u| find(**u).is_none())
                .map(|u| u.to_string()[..8].to_string())
                .collect();
            Err(format!(
                "connected, media service present but missing {}; {}",
                missing.join(", "),
                describe_services(services)
            ))
        }
    }
}

#[derive(Debug)]
struct GattLink {
    control: btleplug::api::Characteristic,
    notify1: btleplug::api::Characteristic,
    bulk: btleplug::api::Characteristic,
    notify2: btleplug::api::Characteristic,
}

struct BleRadio<'a> {
    adapter: &'a btleplug::platform::Adapter,
    started: Instant,
}

impl Radio for BleRadio<'_> {
    type Dev = btleplug::platform::Peripheral;
    type Link = GattLink;

    async fn visible(&mut self) -> Result<Vec<Seen<Self::Dev>>, String> {
        let mut seen = Vec::new();
        for p in self.adapter.peripherals().await.map_err(|e| e.to_string())? {
            // One peripheral whose properties cannot be read is not a reason to stop.
            let Ok(props) = p.properties().await else { continue };
            let props = props.unwrap_or_default();
            seen.push(Seen {
                address: p.address().to_string(),
                name: props.local_name.unwrap_or_default(),
                rssi: props.rssi,
                dev: p,
            });
        }
        Ok(seen)
    }

    async fn probe(&mut self, p: &Self::Dev) -> Probe<GattLink> {
        if !p.is_connected().await.unwrap_or(false) {
            if let Err(e) = p.connect().await {
                return Probe::Unreachable(format!("connect failed: {e}"));
            }
        }
        let verdict = match p.discover_services().await {
            Ok(()) => media_link(&p.services()),
            Err(e) => Err(format!("connected, but service discovery failed: {e}")),
        };
        match verdict {
            Ok(link) => Probe::Qualified(link),
            Err(why) => {
                if let Err(e) = p.disconnect().await {
                    log!("warning: could not disconnect from {}: {e}", p.address());
                }
                Probe::Unqualified(why)
            }
        }
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    async fn pause(&mut self, d: Duration) {
        sleep(d).await;
    }

    async fn rescan(&mut self) {
        // Restarting the scan is best effort: the one already running keeps reporting.
        let _ = self.adapter.stop_scan().await;
        if let Err(e) = self.adapter.start_scan(ScanFilter::default()).await {
            log!("warning: could not restart the scan: {e}");
        }
        self.started = Instant::now();
    }
}

/// Address of the device that last completed an upload, from `<stream>.device`.
fn read_cached_device(stream_path: &Path) -> Option<String> {
    let text = fs::read_to_string(sidecar_path(stream_path, ".device")).ok()?;
    sidecar_value(&text, "address").filter(|a| !a.is_empty()).map(str::to_string)
}

fn write_cached_device(stream_path: &Path, address: &str, name: &str) {
    write_sidecar(
        stream_path,
        ".device",
        &format!("address={address}\nname={name}\nlast_completed_at={}\nrun_id={}\n", utc_now(), run_id()),
    );
}

/// How long to wait for one block's PROGRESS before abandoning the transfer. Healthy
/// blocks peak around 1 s. 10000 was tried 2026-10-15/16: it rescued no upload, and its
/// one slowing run stalled anyway, only later (PROJECT_HISTORY §7.1, third part).
const BLOCK_TIMEOUT_MS: u64 = 2_500;

struct Opts {
    stream_path: PathBuf,
    intra_block_ms: u64,
    block_timeout_ms: u64,
    once: bool,
    live: bool,
    allow_single_frame: bool,
}

fn parse_opts(args: &[String]) -> Result<Opts, Failed> {
    fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a String> {
        args.windows(2).find(|w| w[0] == flag).map(|w| &w[1])
    }
    fn number(args: &[String], flag: &str, default: u64) -> Result<u64, Failed> {
        match value(args, flag) {
            None => Ok(default),
            Some(v) => v.parse().class(Failure::Fatal).map_err(|mut f| {
                f.detail = format!("{flag} {v}: {}", f.detail);
                f
            }),
        }
    }
    Ok(Opts {
        stream_path: value(args, "--stream")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"..\extracted\session_01_bulk_stream.bin")),
        intra_block_ms: number(args, "--intra-block-ms", 2)?,
        block_timeout_ms: number(args, "--block-timeout-ms", BLOCK_TIMEOUT_MS)?,
        once: args.iter().any(|a| a == "--once"),
        live: args.iter().any(|a| a == "--confirm-flow-watch"),
        allow_single_frame: args.iter().any(|a| a == "--allow-single-frame"),
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    log!(
        "TIGA flow-controlled stream uploader starting (run_id={})",
        std::env::var("TIGA_RUN_ID").unwrap_or_else(|_| "-".into())
    );

    let opts = match parse_opts(&args) {
        Ok(o) => o,
        Err(f) => return exit_with(None, f),
    };
    match run(&opts).await {
        Ok(()) => {
            log!("EXIT code=0 (success)");
            ExitCode::SUCCESS
        }
        Err(f) => exit_with(Some(&opts.stream_path), f),
    }
}

fn exit_with(stream_path: Option<&Path>, f: Failed) -> ExitCode {
    let line = format!(
        "EXIT code={} ({}): {}{}",
        f.kind.code(),
        f.kind.meaning(),
        f.detail,
        f.frame_id.as_deref().map(|id| format!(" frame={id}")).unwrap_or_default()
    );
    log!("{line}");
    eprintln!("{} {line}", utc_now());
    if let Some(path) = stream_path {
        record_exit(path, &f);
    }
    ExitCode::from(f.kind.code())
}

async fn run(opts: &Opts) -> Result<(), Failed> {
    let stream_path = &opts.stream_path;
    let (intra_block_ms, block_timeout_ms) = (opts.intra_block_ms, opts.block_timeout_ms);
    let (once, allow_single_frame) = (opts.once, opts.allow_single_frame);

    println!("Stream          : {}", stream_path.display());
    println!("Intra-block pace: {} ms/write", intra_block_ms);
    println!("Flow control    : WAIT for one PROGRESS after every 4096-byte DLX block");
    println!("Block timeout   : {} ms", block_timeout_ms);
    println!("Mode            : {}", if once { "one upload" } else { "watch + coalesce" });

    let initial = fs::read(stream_path)
        .class(Failure::Fatal)
        .map_err(|mut f| { f.detail = format!("{}: {}", stream_path.display(), f.detail); f })?;
    let initial_chunks = build_chunks(&initial, allow_single_frame).class(Failure::Fatal)?;
    println!(
        "Validated: {} bytes, {} writes, {} x 4096-byte progress blocks{}",
        initial.len(),
        initial_chunks.len(),
        initial_chunks.iter().filter(|c| c.ends_4k_block).count(),
        if allow_single_frame { "  [--allow-single-frame: EXPERIMENTAL]" } else { "" }
    );

    // A session left open by an earlier run (killed, crashed, or the PC shut down
    // mid-transfer) means the keyboard has not been power-cycled since.
    if let Some(marker) = read_session_marker(stream_path) {
        let frame = sidecar_value(&marker, "frame_id").unwrap_or("?").to_string();
        let block = sidecar_value(&marker, "last_acked_block").and_then(|b| b.parse().ok());
        let detail = format!(
            "a media session was left open by an earlier run (frame {frame}, last ACKed block {}); \
             disconnect the keyboard's battery for at least 30 s (there is no power switch; \
             unplugging USB doesn't count), then run START_STACK.ps1 (or delete {})",
            block.map(|b: usize| b.to_string()).unwrap_or_else(|| "?".into()),
            sidecar_path(stream_path, ".session").display()
        );
        if opts.live {
            return Err(Failed {
                kind: Failure::Interrupted,
                detail,
                frame_id: Some(frame),
                last_acked_block: block,
            });
        }
        println!("WARNING: {detail}");
    }

    if !opts.live {
        println!();
        println!("DRY RUN ONLY — no Bluetooth connection was opened.");
        println!("Live mode requires --confirm-flow-watch");
        return Ok(());
    }

    let manager = Manager::new().await.class(Failure::Unreachable)?;
    let adapters = manager.adapters().await.class(Failure::Unreachable)?;
    let adapter = adapters.first().ok_or("No Bluetooth adapter found").class(Failure::Unreachable)?;
    log!(
        "Using adapter 1/{}: {}",
        adapters.len(),
        adapter.adapter_info().await.unwrap_or_else(|e| format!("(info unavailable: {e})"))
    );

    let cached = read_cached_device(stream_path);
    match &cached {
        Some(a) => log!("Scanning for {TARGET_NAME} (trying {a} first: it completed the last upload)..."),
        None => log!("Scanning for {TARGET_NAME}..."),
    }
    adapter.start_scan(ScanFilter::default()).await.class(Failure::Unreachable)?;
    let mut radio = BleRadio { adapter, started: Instant::now() };
    // The scan is left running, as it always has been during uploads.
    let Chosen { address, name, dev: peripheral, link } = select_device(&mut radio, cached.as_deref()).await?;
    let GattLink { control, notify1, bulk, notify2 } = link;
    log!("Found keyboard at {address}");

    let mut notifications = peripheral.notifications().await.class(Failure::Gatt)?;
    peripheral.subscribe(&notify1).await.class(Failure::Gatt)?;
    peripheral.subscribe(&notify2).await.class(Failure::Gatt)?;
    log!("Connected once. Notifications active.");

    let mut last_uploaded: Option<Vec<u8>> = None;
    let mut last_crc = String::new();
    let mut last_frame_id = String::new();
    let mut device_recorded = false;
    let mut first_on_connection = true;

    loop {
        let stream = fs::read(stream_path).class(Failure::Fatal)?;
        validate_stream(&stream, allow_single_frame).class(Failure::Fatal)?;

        if last_uploaded.as_deref() == Some(stream.as_slice()) {
            note_already_displayed(stream_path, &last_crc, &mut last_frame_id);
            if once { break; }
            sleep(Duration::from_millis(250)).await;
            continue;
        }

        let chunks = build_chunks(&stream, allow_single_frame).class(Failure::Fatal)?;
        let crc = format!("{:08x}", crc32(&stream));
        let frame_id = frame_id_for(stream_path, &crc);
        println!();
        log!("UPLOAD START frame={frame_id} crc32={crc}");

        let started = Instant::now();
        let mut timings = UploadTimings::default();
        let result = upload_stream(
            &peripheral,
            &control,
            &bulk,
            &mut notifications,
            &chunks,
            intra_block_ms,
            block_timeout_ms,
            stream_path,
            &frame_id,
            first_on_connection,
            &mut timings,
        )
        .await;
        let elapsed = started.elapsed().as_secs_f64();
        first_on_connection = false;
        let summary = timings.summary();
        if let Err(f) = result {
            log!("UPLOAD FAILED frame={frame_id} crc32={crc} after {elapsed:.2}s: {}", f.detail);
            log!("{}", summary.log_line(&frame_id));
            return Err(f);
        }

        log!("UPLOAD COMPLETE frame={frame_id} crc32={crc} in {elapsed:.2}s");
        log!("{}", summary.log_line(&frame_id));
        record_delivery(stream_path, &frame_id, &crc, elapsed, Some(&summary));
        if !device_recorded {
            write_cached_device(stream_path, &address, &name);
            device_recorded = true;
        }

        last_uploaded = Some(stream);
        last_crc = crc;
        last_frame_id = frame_id;

        if once {
            break;
        }

        sleep(Duration::from_millis(100)).await;
    }

    peripheral.disconnect().await.class(Failure::Unreachable)?;
    log!("Disconnected cleanly.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_and_p90() {
        assert!(median_p90(&[]).is_none());
        assert_eq!(median_p90(&[5.0]), Some((5.0, 5.0)));
        assert_eq!(median_p90(&[4.0, 1.0, 3.0, 2.0]), Some((2.5, 4.0)));
        // 1..=10: nearest-rank p90 is the 9th value.
        let v: Vec<f64> = (1..=10).rev().map(f64::from).collect();
        assert_eq!(median_p90(&v), Some((5.5, 9.0)));
        // 53 healthy blocks with one slow one: p90 stays healthy, max does not.
        let mut v = vec![150.0; 53];
        v[40] = 2400.0;
        assert_eq!(median_p90(&v), Some((150.0, 150.0)));
    }

    #[test]
    fn summary_finds_the_slowest_block() {
        let t = UploadTimings {
            control1_ack_ms: Some(42.4),
            control2_ack_ms: None,
            block_ms: vec![150.0, 1800.0, 900.0, 1800.0],
        };
        let s = t.summary();
        assert_eq!((s.blocks, s.slowest_block), (4, 2));
        assert_eq!((s.median_ms, s.max_ms), (1350.0, 1800.0));
        let line = s.log_line("r-f0001");
        assert!(line.starts_with("TIMING frame=r-f0001 blocks_acked=4 median=1350ms p90=1800ms max=1800ms slowest=block 2"), "{line}");
        assert!(line.ends_with("control1_ack=42ms control2_ack=-ms"), "{line}");
        let side = s.sidecar_lines();
        assert_eq!(sidecar_value(&side, "slowest_block"), Some("2"));
        assert_eq!(sidecar_value(&side, "control2_ack_ms"), Some("-"));

        // Failed before any block: no panic, zeros.
        let s = UploadTimings::default().summary();
        assert_eq!((s.blocks, s.slowest_block, s.max_ms), (0, 0, 0.0));
    }

    #[test]
    fn slow_block_threshold() {
        assert!(slow_block_line(7, 53, 1000.0, 3.0).is_none());
        assert!(slow_block_line(7, 53, 150.0, 3.0).is_none());
        assert_eq!(
            slow_block_line(7, 53, 1834.4, 4.214).as_deref(),
            Some("SLOW block 7/53 took 1834 ms (elapsed=4.21s)")
        );
    }

    #[test]
    fn first_upload_warning_threshold() {
        // The slowing runs: 2.35, 2.42, 2.97 s. Healthy first uploads: 0.71-0.85 s.
        for s in [2.35, 2.42, 2.97] {
            assert!(first_upload_warning(true, 5, s).is_some(), "{s}");
        }
        for s in [0.71, 0.85, 1.5] {
            assert!(first_upload_warning(true, 5, s).is_none(), "{s}");
        }
        // Later uploads on a connection reach block 5 at 1.3-2.04 s when healthy.
        assert!(first_upload_warning(false, 5, 2.04).is_none());
        assert!(first_upload_warning(true, 4, 2.5).is_none());
        assert!(first_upload_warning(true, 6, 2.5).is_none());
        assert_eq!(
            first_upload_warning(true, 5, 2.42).as_deref(),
            Some("WARNING first upload is slow (block 5 at 2.42 s; healthy 0.7–0.85 s)")
        );
    }

    #[test]
    fn delivery_record_carries_the_summary() {
        let dir = std::env::temp_dir().join(format!("tiga_timing_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let stream = dir.join("s.bin");
        let t = UploadTimings { control1_ack_ms: Some(40.0), control2_ack_ms: Some(55.0), block_ms: vec![150.0, 160.0] };
        record_delivery(&stream, "r-f0002", "0badf00d", 8.7, Some(&t.summary()));
        let d = fs::read_to_string(sidecar_path(&stream, ".delivered")).unwrap();
        assert_eq!(sidecar_value(&d, "frame_id"), Some("r-f0002"));
        assert_eq!(sidecar_value(&d, "elapsed_s"), Some("8.70"));
        assert_eq!(sidecar_value(&d, "block_median_ms"), Some("155"));
        assert_eq!(sidecar_value(&d, "block_max_ms"), Some("160"));
        assert_eq!(sidecar_value(&d, "control1_ack_ms"), Some("40"));
        assert_eq!(sidecar_value(&d, "control2_ack_ms"), Some("55"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn crc32_matches_zlib() {
        // zlib.crc32(b"123456789") == 0xcbf43926
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn utc_now_is_iso8601() {
        let s = utc_now();
        assert_eq!(s.len(), 20, "{s}");
        assert!(s.ends_with('Z') && s.as_bytes()[10] == b'T', "{s}");
        assert!(s.as_str() >= "2024-01-01T00:00:00Z", "{s}");
    }

    #[test]
    fn sidecar_lookup() {
        let text = "frame_id=renderer-x-f0003\ncrc32=DEADBEEF\nreason=a=b\n";
        assert_eq!(sidecar_value(text, "frame_id"), Some("renderer-x-f0003"));
        assert_eq!(sidecar_value(text, "reason"), Some("a=b"));
        assert_eq!(sidecar_value(text, "missing"), None);
    }

    #[test]
    fn frame_id_requires_matching_crc() {
        let dir = std::env::temp_dir().join(format!("tiga_sidecar_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let stream = dir.join("s.bin");
        assert_eq!(frame_id_for(&stream, "00000000"), "unknown(no-sidecar)");
        fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0001\ncrc32=0badf00d\n").unwrap();
        assert_eq!(frame_id_for(&stream, "0badf00d"), "r-f0001");
        assert_eq!(frame_id_for(&stream, "11111111"), "unknown(sidecar-is-r-f0001)");
        record_delivery(&stream, "r-f0001", "0badf00d", 7.5, None);
        let delivered = fs::read_to_string(sidecar_path(&stream, ".delivered")).unwrap();
        assert_eq!(sidecar_value(&delivered, "frame_id"), Some("r-f0001"));
        assert_eq!(sidecar_value(&delivered, "elapsed_s"), Some("7.50"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn identical_rerender_is_recorded_once() {
        let dir = std::env::temp_dir().join(format!("tiga_redisplay_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let stream = dir.join("s.bin");
        let mut last_id = "r-f0001".to_string();

        // Sidecar still names the uploaded frame: nothing to record.
        fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0001\ncrc32=0badf00d\n").unwrap();
        assert!(!note_already_displayed(&stream, "0badf00d", &mut last_id));

        // Same bytes, new frame ID: recorded once.
        fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0002\ncrc32=0badf00d\n").unwrap();
        assert!(note_already_displayed(&stream, "0badf00d", &mut last_id));
        assert!(!note_already_displayed(&stream, "0badf00d", &mut last_id));
        let delivered = fs::read_to_string(sidecar_path(&stream, ".delivered")).unwrap();
        assert_eq!(sidecar_value(&delivered, "frame_id"), Some("r-f0002"));

        // Sidecar already describes a different (not yet uploaded) stream: not recorded.
        fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0003\ncrc32=11111111\n").unwrap();
        assert!(!note_already_displayed(&stream, "0badf00d", &mut last_id));

        // Nothing uploaded yet: never recorded.
        assert!(!note_already_displayed(&stream, "", &mut String::new()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tracked_template_builds_expected_chunks() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("extracted")
            .join("session_01_bulk_stream.redgreen-backup.bin");
        let stream = fs::read(path).unwrap();
        assert_eq!(build_chunks(&stream, false).unwrap().len(), CAPTURED_BULK_WRITES);
        assert_eq!(build_chunks(&stream, true).unwrap().len(), CAPTURED_BULK_WRITES);
    }

    #[test]
    fn exit_codes_are_stable() {
        // The supervisors depend on these numbers.
        let codes: Vec<u8> = [
            Failure::Unreachable,
            Failure::Gatt,
            Failure::Interrupted,
            Failure::NoCompletion,
            Failure::Fatal,
        ]
        .iter()
        .map(|f| f.code())
        .collect();
        assert_eq!(codes, [10, 11, 20, 21, 30]);
        assert!(Failure::Interrupted.meaning().contains("battery for at least 30 s"));
        assert!(Failure::NoCompletion.meaning().contains("battery for at least 30 s"));
        assert!(!Failure::Interrupted.meaning().contains("power switch,"));
        assert!(Failure::Unreachable.meaning().contains("safe to retry"));
    }

    #[test]
    fn control2_is_the_boundary() {
        // Anything before CONTROL2 is retryable, however far it got.
        assert_eq!(classify_upload_failure(false, false), Failure::Unreachable);
        // From CONTROL2 on, a session may be open, even with zero bulk writes.
        assert_eq!(classify_upload_failure(true, false), Failure::Interrupted);
        assert_eq!(classify_upload_failure(true, true), Failure::NoCompletion);
    }

    #[test]
    fn upload_progress_reports_block_only_after_control2() {
        let path = Path::new("unused.bin");
        let mut p = UploadProgress {
            stream_path: path,
            frame_id: "r-f0007",
            total_blocks: 53,
            control2_attempted: false,
            last_acked_block: 0,
            all_blocks_acked: false,
        };
        let f = p.fail("C1 ACK timed out");
        assert_eq!((f.kind, f.last_acked_block), (Failure::Unreachable, None));
        assert_eq!(f.frame_id.as_deref(), Some("r-f0007"));

        p.control2_attempted = true;
        let f = p.fail("C2 ACK timed out");
        assert_eq!((f.kind, f.last_acked_block), (Failure::Interrupted, Some(0)));
        assert!(f.detail.contains("last ACKed block 0/53"), "{}", f.detail);

        p.last_acked_block = 17;
        let f = p.fail("notification stream ended");
        assert_eq!((f.kind, f.last_acked_block), (Failure::Interrupted, Some(17)));

        p.last_acked_block = 53;
        p.all_blocks_acked = true;
        assert_eq!(p.fail("no completion").kind, Failure::NoCompletion);
    }

    #[test]
    fn session_marker_and_exit_record() {
        let dir = std::env::temp_dir().join(format!("tiga_session_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let stream = dir.join("s.bin");

        assert!(read_session_marker(&stream).is_none());
        write_session_marker(&stream, "r-f0003", 12, 53);
        let m = read_session_marker(&stream).unwrap();
        assert_eq!(sidecar_value(&m, "frame_id"), Some("r-f0003"));
        assert_eq!(sidecar_value(&m, "last_acked_block"), Some("12"));
        clear_session_marker(&stream);
        assert!(read_session_marker(&stream).is_none());
        clear_session_marker(&stream); // absent is fine

        let f = Failed {
            kind: Failure::Interrupted,
            detail: "line one\nline two".into(),
            frame_id: Some("r-f0003".into()),
            last_acked_block: Some(12),
        };
        record_exit(&stream, &f);
        let e = fs::read_to_string(sidecar_path(&stream, ".exit")).unwrap();
        assert_eq!(sidecar_value(&e, "code"), Some("20"));
        assert_eq!(sidecar_value(&e, "detail"), Some("line one line two"));
        assert_eq!(sidecar_value(&e, "last_acked_block"), Some("12"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_numeric_argument_is_fatal() {
        let args: Vec<String> = ["--intra-block-ms", "fast"].iter().map(|s| s.to_string()).collect();
        let Err(f) = parse_opts(&args) else { panic!("accepted a bad number") };
        assert_eq!(f.kind, Failure::Fatal);
        let ok: Vec<String> = ["--stream", "x.bin", "--confirm-flow-watch"].iter().map(|s| s.to_string()).collect();
        let o = parse_opts(&ok).unwrap();
        assert!(o.live && !o.once && o.intra_block_ms == 2 && o.block_timeout_ms == 2_500);
    }

    #[test]
    fn captured_stream_control1_unchanged() {
        assert_eq!(control1_for(STREAM_LEN), CONTROL1);
    }

    fn synthetic_stream(len: usize) -> Vec<u8> {
        let tpl = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("extracted")
                .join("session_01_bulk_stream.redgreen-backup.bin"),
        )
        .unwrap();
        let mut s = tpl[..33].to_vec();
        s.resize(len - 4, 0);
        s.extend_from_slice(&[0xfc, 0xff, 0, 0]);
        let body = len - 8;
        s[4] = (body >> 16) as u8;
        s[5] = (body >> 8) as u8;
        s[6] = body as u8;
        s
    }

    #[test]
    fn single_frame_stream_needs_flag() {
        for len in SINGLE_FRAME_STREAM_LENS {
            let s = synthetic_stream(len);
            assert!(validate_stream(&s, false).is_err(), "{len} accepted without flag");
            assert!(validate_stream(&s, true).is_ok(), "{len} rejected with flag");
        }
        // Other sizes stay rejected even with the flag.
        assert!(validate_stream(&synthetic_stream(150_000), true).is_err());
    }

    /// Validates real tiga_dlx.py output when TIGA_SINGLE_FRAME_SAMPLE names a file
    /// (comma-separated for several); skipped otherwise.
    #[test]
    fn python_built_single_frame_sample() {
        let Ok(paths) = std::env::var("TIGA_SINGLE_FRAME_SAMPLE") else { return };
        for path in paths.split(',') {
            let s = fs::read(path).unwrap();
            let chunks = build_chunks(&s, true).unwrap();
            assert!(validate_stream(&s, false).is_err());
            println!(
                "{path}: {} bytes, {} writes, {} blocks, CONTROL1 len bytes {:02x?}",
                s.len(),
                chunks.len(),
                chunks.iter().filter(|c| c.ends_4k_block).count(),
                &control1_for(s.len())[18..21]
            );
        }
    }

    // ---- Device selection (issue #14) ----

    const GOOD: &str = "07:01:88:B3:66:1B";
    const BAD: &str = "1B:A3:A9:10:13:5D";

    /// A scan on a virtual clock. Each device appears at a given time; probing takes 1 s.
    struct FakeRadio {
        devices: Vec<(Duration, &'static str, &'static str)>, // appears_at, address, name
        qualified: Vec<&'static str>,
        unreachable: Vec<&'static str>,
        clock: Duration,
        scan_started: Duration,
        rescans: u32,
        probed: Vec<String>,
    }

    impl FakeRadio {
        fn new(devices: &[(u64, &'static str)], qualified: &[&'static str]) -> Self {
            FakeRadio {
                devices: devices.iter().map(|&(ms, a)| (Duration::from_millis(ms), a, "Zoom75 Tiga")).collect(),
                qualified: qualified.to_vec(),
                unreachable: Vec::new(),
                clock: Duration::ZERO,
                scan_started: Duration::ZERO,
                rescans: 0,
                probed: Vec::new(),
            }
        }
    }

    impl Radio for FakeRadio {
        type Dev = &'static str;
        type Link = ();

        async fn visible(&mut self) -> Result<Vec<Seen<&'static str>>, String> {
            Ok(self
                .devices
                .iter()
                .filter(|d| d.0 <= self.clock)
                .map(|&(_, address, name)| Seen { address: address.into(), name: name.into(), rssi: Some(-60), dev: address })
                .collect())
        }

        async fn probe(&mut self, dev: &&'static str) -> Probe<()> {
            self.probed.push(dev.to_string());
            self.clock += Duration::from_secs(1);
            if self.unreachable.contains(dev) {
                Probe::Unreachable("connect failed".into())
            } else if self.qualified.contains(dev) {
                Probe::Qualified(())
            } else {
                Probe::Unqualified("connected, but no media service".into())
            }
        }

        fn elapsed(&self) -> Duration {
            self.clock - self.scan_started
        }

        async fn pause(&mut self, d: Duration) {
            self.clock += d;
        }

        async fn rescan(&mut self) {
            self.rescans += 1;
            self.scan_started = self.clock;
        }
    }

    /// The names seen on hardware 2026-10-15: GOOD is the display, BAD the BT1 typing slot.
    fn real_names(devices: &[(u64, &'static str)], qualified: &[&'static str]) -> FakeRadio {
        let mut r = FakeRadio::new(&[], qualified);
        for &(ms, a) in devices {
            let name = if a == BAD { "Zoom75 Tiga BT1" } else { "Zoom75 Tiga" };
            r.devices.push((Duration::from_millis(ms), a, name));
        }
        r
    }

    fn select(radio: &mut FakeRadio, cached: Option<&str>) -> Result<String, Failed> {
        futures::executor::block_on(select_device(radio, cached)).map(|c| c.address)
    }

    #[test]
    fn wrong_device_first_is_skipped() {
        let mut r = FakeRadio::new(&[(0, BAD), (300, GOOD)], &[GOOD]);
        assert_eq!(select(&mut r, None).unwrap(), GOOD);
        assert_eq!(r.probed, [BAD, GOOD]);
        // Nothing is tried before the window closes, so every candidate gets logged.
        assert!(r.clock >= SCAN_WINDOW);
    }

    #[test]
    fn typing_slot_names() {
        for n in ["Zoom75 Tiga BT1", "Zoom75 Tiga BT2", "Zoom75 Tiga bt12"] {
            assert!(is_typing_slot_name(n), "{n}");
        }
        for n in ["Zoom75 Tiga", "Zoom75 Tiga BT", "Zoom75 Tiga BTx", "BT1", "Zoom75 Tiga 2.4G", ""] {
            assert!(!is_typing_slot_name(n), "{n}");
        }
    }

    #[test]
    fn exact_name_beats_typing_slot_seen_first() {
        // 23:06 on hardware: BT1 seen first cost an 11 s failed connect.
        let mut r = real_names(&[(0, BAD), (1000, GOOD)], &[GOOD]);
        assert_eq!(select(&mut r, None).unwrap(), GOOD);
        assert_eq!(r.probed, [GOOD]);
        // If the display name does not qualify, the typing slot is still tried, second.
        let mut r = real_names(&[(0, BAD), (1000, GOOD)], &[BAD]);
        assert_eq!(select(&mut r, None).unwrap(), BAD);
        assert_eq!(r.probed, [GOOD, BAD]);
    }

    #[test]
    fn other_zoom75_name_comes_between() {
        let odd = "AA:BB:CC:DD:EE:FF";
        let mut r = real_names(&[(0, BAD), (100, GOOD)], &[]);
        r.devices.push((Duration::from_millis(50), odd, "ZOOM75 TIGA-EU"));
        let _ = select(&mut r, None).unwrap_err();
        assert_eq!(r.probed, [GOOD, odd, BAD]);
    }

    #[test]
    fn only_a_typing_slot_is_still_probed() {
        let mut r = real_names(&[(0, BAD)], &[BAD]);
        assert_eq!(select(&mut r, None).unwrap(), BAD);
        assert_eq!(r.probed, [BAD]);
    }

    #[test]
    fn cache_beats_the_exact_name() {
        // Even a cached typing slot is tried before the display name.
        let mut r = real_names(&[(0, GOOD), (300, BAD)], &[BAD]);
        assert_eq!(select(&mut r, Some(BAD)).unwrap(), BAD);
        assert_eq!(r.probed, [BAD]);
        assert!(r.clock < SCAN_WINDOW);
    }

    #[test]
    fn only_the_wrong_device_gives_11() {
        let mut r = FakeRadio::new(&[(0, BAD)], &[GOOD]);
        let f = select(&mut r, None).unwrap_err();
        assert_eq!(f.kind, Failure::Gatt);
        assert!(f.detail.contains("tried 1 candidate(s) over 2 scan(s)"), "{}", f.detail);
        assert!(f.detail.contains(BAD), "{}", f.detail);
        assert_eq!(r.probed, [BAD]); // tried once, not again on later polls or the rescan
        assert_eq!(r.rescans, 1);
    }

    #[test]
    fn rescan_finds_a_device_that_advertised_late() {
        // The good keyboard only shows up after the first window has closed.
        let mut r = FakeRadio::new(&[(0, BAD), (8000, GOOD)], &[GOOD]);
        assert_eq!(select(&mut r, None).unwrap(), GOOD);
        assert_eq!(r.probed, [BAD, GOOD]);
        assert_eq!(r.rescans, 1);
    }

    #[test]
    fn neither_of_two_qualifies() {
        let other = "AA:BB:CC:DD:EE:FF";
        let mut r = FakeRadio::new(&[(0, BAD), (1000, other)], &[]);
        let f = select(&mut r, None).unwrap_err();
        assert_eq!(f.kind, Failure::Gatt);
        assert!(f.detail.contains("tried 2 candidate(s)"), "{}", f.detail);
        assert_eq!(r.probed, [BAD, other]);
    }

    #[test]
    fn cached_device_is_tried_first_without_waiting() {
        let mut r = FakeRadio::new(&[(0, BAD), (500, GOOD)], &[GOOD]);
        // Cached address in a different case still matches.
        assert_eq!(select(&mut r, Some(&GOOD.to_ascii_lowercase())).unwrap(), GOOD);
        assert_eq!(r.probed, [GOOD]);
        assert!(r.clock < SCAN_WINDOW, "waited for the whole window: {:?}", r.clock);
    }

    #[test]
    fn cached_device_gone_falls_back_to_full_scan() {
        let gone = "11:22:33:44:55:66";
        let mut r = FakeRadio::new(&[(0, BAD), (200, GOOD)], &[GOOD]);
        assert_eq!(select(&mut r, Some(gone)).unwrap(), GOOD);
        assert_eq!(r.probed, [BAD, GOOD]);
    }

    #[test]
    fn cached_device_no_longer_qualifying_falls_back() {
        let mut r = FakeRadio::new(&[(0, BAD), (200, GOOD)], &[GOOD]);
        assert_eq!(select(&mut r, Some(BAD)).unwrap(), GOOD);
        assert_eq!(r.probed, [BAD, GOOD]);
    }

    #[test]
    fn cached_device_counts_even_without_a_name() {
        let mut r = FakeRadio::new(&[], &[GOOD]);
        r.devices.push((Duration::ZERO, GOOD, ""));
        r.devices.push((Duration::ZERO, "99:99:99:99:99:99", "Some Mouse"));
        assert_eq!(select(&mut r, Some(GOOD)).unwrap(), GOOD);
        assert_eq!(r.probed, [GOOD]);
    }

    #[test]
    fn nothing_found_is_10() {
        let mut r = FakeRadio::new(&[], &[]);
        r.devices.push((Duration::ZERO, "99:99:99:99:99:99", "Some Mouse"));
        let f = select(&mut r, None).unwrap_err();
        assert_eq!(f.kind, Failure::Unreachable);
        assert!(f.detail.contains("not found"), "{}", f.detail);
        assert!(r.probed.is_empty());
        // Nothing to reject, so no rescan: the supervisor's retry covers this case.
        assert_eq!(r.rescans, 0);
        // But it kept looking for the longer limit, not just one window.
        assert!(r.clock >= NOTHING_SEEN_LIMIT, "{:?}", r.clock);
        assert!(f.detail.contains("15 s scan"), "{}", f.detail);
    }

    #[test]
    fn keyboard_appearing_late_is_found_in_the_first_scan() {
        // After a restart the keyboard may take a while to advertise again.
        let mut r = real_names(&[(9000, BAD), (9000, GOOD)], &[GOOD]);
        assert_eq!(select(&mut r, None).unwrap(), GOOD);
        assert_eq!(r.probed, [GOOD]);
        assert_eq!(r.rescans, 0);
        assert!(r.clock < Duration::from_secs(11), "{:?}", r.clock);
        // Same with the cache.
        let mut r = real_names(&[(12_000, GOOD)], &[GOOD]);
        assert_eq!(select(&mut r, Some(GOOD)).unwrap(), GOOD);
    }

    #[test]
    fn rejected_candidates_do_not_get_the_longer_wait() {
        // One window, one rescan window, done: the extension is for "nothing seen" only.
        let mut r = real_names(&[(0, BAD)], &[GOOD]);
        let _ = select(&mut r, None).unwrap_err();
        assert!(r.clock < Duration::from_secs(12), "{:?}", r.clock);
    }

    #[test]
    fn unreachable_candidate_makes_it_10_not_11() {
        let mut r = FakeRadio::new(&[(0, BAD), (0, GOOD)], &[GOOD]);
        r.unreachable.push(GOOD);
        let f = select(&mut r, None).unwrap_err();
        assert_eq!(f.kind, Failure::Unreachable);
        assert!(f.detail.contains("tried 2 candidate(s)"), "{}", f.detail);
    }

    #[test]
    fn media_link_requires_service_and_all_four() {
        use btleplug::api::{CharPropFlags, Characteristic, Service};
        use std::collections::BTreeSet;
        let chr = |uuid| Characteristic {
            uuid,
            service_uuid: SERVICE_MEDIA,
            properties: CharPropFlags::empty(),
            descriptors: BTreeSet::new(),
        };
        let service = |uuid, chars: &[Uuid]| Service {
            uuid,
            primary: true,
            characteristics: chars.iter().map(|&u| chr(u)).collect(),
        };
        let all = [CHAR_CONTROL, CHAR_NOTIFY, CHAR_BULK, CHAR_NOTIFY2];

        let ok: BTreeSet<_> = [service(SERVICE_MEDIA, &all)].into();
        assert!(media_link(&ok).is_ok());

        let none: BTreeSet<Service> = BTreeSet::new();
        let why = media_link(&none).unwrap_err();
        assert!(why.contains("no media service") && why.contains("exposes no services"), "{why}");

        // A rejected device names what it does expose.
        let gap = Uuid::from_u128(0x00001800_0000_1000_8000_00805f9b34fb);
        let name_chr = Uuid::from_u128(0x00002a00_0000_1000_8000_00805f9b34fb);
        let other: BTreeSet<_> = [service(gap, &[name_chr])].into();
        let why = media_link(&other).unwrap_err();
        assert!(why.contains("exposes 1 service(s): 00001800-0000-1000-8000-00805f9b34fb [00002a00-"), "{why}");

        // The four characteristics under some other service do not count.
        let elsewhere: BTreeSet<_> = [service(Uuid::from_u128(0x180a), &all)].into();
        assert!(media_link(&elsewhere).unwrap_err().contains("no media service"));

        let partial: BTreeSet<_> = [service(SERVICE_MEDIA, &all[1..])].into();
        assert!(media_link(&partial).unwrap_err().contains("missing 1f400001"));
    }

    #[test]
    fn device_cache_round_trip() {
        let dir = std::env::temp_dir().join(format!("tiga_device_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let stream = dir.join("s.bin");
        assert_eq!(read_cached_device(&stream), None);
        write_cached_device(&stream, GOOD, "Zoom75 Tiga");
        assert_eq!(read_cached_device(&stream).as_deref(), Some(GOOD));
        fs::write(sidecar_path(&stream, ".device"), "address=\n").unwrap();
        assert_eq!(read_cached_device(&stream), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn envelope_length_must_match() {
        let mut s = synthetic_stream(SINGLE_FRAME_STREAM_LENS[0]);
        s[6] ^= 1;
        assert!(validate_stream(&s, true).is_err());
    }

    #[test]
    fn single_frame_chunking() {
        let s = synthetic_stream(110_565);
        let chunks = build_chunks(&s, true).unwrap();
        let stream_len: usize = chunks.iter().map(|c| c.value.len() - 1).sum();
        assert_eq!(stream_len, 110_565);
        assert_eq!(chunks.iter().filter(|c| c.ends_4k_block).count(), 26);
        assert!(chunks.iter().all(|c| c.value.len() <= 182));
        let c1 = control1_for(stream_len);
        assert_eq!(&c1[18..21], &[0x01, 0xaf, 0xcc]); // file length 110,540
        assert_eq!(&c1[..18], &CONTROL1[..18]);
        assert_eq!(&c1[21..], &CONTROL1[21..]);
    }
}
