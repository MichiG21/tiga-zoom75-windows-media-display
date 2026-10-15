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

fn record_delivery(stream_path: &Path, frame_id: &str, crc: &str, elapsed_s: f64) {
    write_sidecar(
        stream_path,
        ".delivered",
        &format!(
            "delivered_at={}\nframe_id={frame_id}\ncrc32={crc}\nelapsed_s={elapsed_s:.2}\nrun_id={}\n",
            utc_now(),
            run_id()
        ),
    );
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
            Failure::Interrupted => "transfer interrupted after CONTROL2; the keyboard may hold a partial media session; power-cycle it before retrying",
            Failure::NoCompletion => "all blocks sent but the keyboard never confirmed completion; power-cycle it before retrying",
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
    peripheral.write(control, &control1_for(stream_len), WriteType::WithoutResponse).await
        .map_err(|e| p.fail(e))?;
    wait_for_ack_pair(notifications, C1_ACK_A, C1_ACK_B, 1500).await
        .map_err(|e| p.fail(e))?;

    sleep(Duration::from_millis(60)).await;

    // ---- Point of no return: from here a failure leaves a partial media session. ----
    write_session_marker(p.stream_path, p.frame_id, 0, p.total_blocks);
    p.control2_attempted = true;
    peripheral.write(control, &CONTROL2, WriteType::WithoutResponse).await
        .map_err(|e| p.fail(e))?;
    wait_for_ack_pair(notifications, C2_ACK_A, C2_ACK_B, 1500).await
        .map_err(|e| p.fail(e))?;

    sleep(Duration::from_millis(60)).await;

    let started = Instant::now();
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
            p.last_acked_block = block_no;
            write_session_marker(p.stream_path, p.frame_id, block_no, total_blocks);

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
        block_timeout_ms: number(args, "--block-timeout-ms", 2500)?,
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
             power-cycle the keyboard, then run START_STACK.ps1 (or delete {})",
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

    log!("Scanning for {TARGET_NAME}...");
    adapter.start_scan(ScanFilter::default()).await.class(Failure::Unreachable)?;
    sleep(Duration::from_secs(5)).await;

    let mut target = None;
    for p in adapter.peripherals().await.class(Failure::Unreachable)? {
        let props = p.properties().await.class(Failure::Unreachable)?;
        let name = props.as_ref().and_then(|p| p.local_name.as_deref()).unwrap_or("");
        if name == TARGET_NAME || name.to_ascii_lowercase().contains("zoom75") {
            target = Some(p);
            break;
        }
    }
    let peripheral = target.ok_or("Zoom75 Tiga not found").class(Failure::Unreachable)?;
    log!("Found keyboard at {}", peripheral.address());

    if !peripheral.is_connected().await.class(Failure::Unreachable)? {
        peripheral.connect().await.class(Failure::Unreachable)?;
    }
    peripheral.discover_services().await.class(Failure::Gatt)?;

    let chars = peripheral.characteristics();
    let find = |uuid: Uuid, what: &str| {
        chars.iter().find(|c| c.uuid == uuid).cloned().ok_or(format!("{what} missing")).class(Failure::Gatt)
    };
    let control = find(CHAR_CONTROL, "control char")?;
    let notify1 = find(CHAR_NOTIFY, "notify 0002")?;
    let bulk = find(CHAR_BULK, "bulk 0003")?;
    let notify2 = find(CHAR_NOTIFY2, "notify 0004")?;

    let mut notifications = peripheral.notifications().await.class(Failure::Gatt)?;
    peripheral.subscribe(&notify1).await.class(Failure::Gatt)?;
    peripheral.subscribe(&notify2).await.class(Failure::Gatt)?;
    log!("Connected once. Notifications active.");

    let mut last_uploaded: Option<Vec<u8>> = None;

    loop {
        let stream = fs::read(stream_path).class(Failure::Fatal)?;
        validate_stream(&stream, allow_single_frame).class(Failure::Fatal)?;

        if last_uploaded.as_deref() == Some(stream.as_slice()) {
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
        if let Err(f) = upload_stream(
            &peripheral,
            &control,
            &bulk,
            &mut notifications,
            &chunks,
            intra_block_ms,
            block_timeout_ms,
            stream_path,
            &frame_id,
        )
        .await
        {
            log!(
                "UPLOAD FAILED frame={frame_id} crc32={crc} after {:.2}s: {}",
                started.elapsed().as_secs_f64(),
                f.detail
            );
            return Err(f);
        }

        let elapsed = started.elapsed().as_secs_f64();
        log!("UPLOAD COMPLETE frame={frame_id} crc32={crc} in {elapsed:.2}s");
        record_delivery(stream_path, &frame_id, &crc, elapsed);

        last_uploaded = Some(stream);

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
        record_delivery(&stream, "r-f0001", "0badf00d", 7.5);
        let delivered = fs::read_to_string(sidecar_path(&stream, ".delivered")).unwrap();
        assert_eq!(sidecar_value(&delivered, "frame_id"), Some("r-f0001"));
        assert_eq!(sidecar_value(&delivered, "elapsed_s"), Some("7.50"));
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
        assert!(Failure::Interrupted.meaning().contains("power-cycle"));
        assert!(Failure::NoCompletion.meaning().contains("power-cycle"));
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
        assert!(o.live && !o.once && o.intra_block_ms == 2 && o.block_timeout_ms == 2500);
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
