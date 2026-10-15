use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::Manager;
use futures::StreamExt;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
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

fn record_delivery(stream_path: &Path, frame_id: &str, crc: &str, elapsed_s: f64) {
    let path = sidecar_path(stream_path, ".delivered");
    let tmp = sidecar_path(stream_path, ".delivered.tmp");
    let text = format!(
        "delivered_at={}\nframe_id={frame_id}\ncrc32={crc}\nelapsed_s={elapsed_s:.2}\nrun_id={}\n",
        utc_now(),
        std::env::var("TIGA_RUN_ID").unwrap_or_default()
    );
    if let Err(e) = fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, &path)) {
        log!("warning: could not write {}: {e}", path.display());
    }
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
    record_delivery(stream_path, &id, last_crc, 0.0);
    *last_frame_id = id;
    true
}

async fn upload_stream<S>(
    peripheral: &btleplug::platform::Peripheral,
    control: &btleplug::api::Characteristic,
    bulk: &btleplug::api::Characteristic,
    notifications: &mut S,
    chunks: &[Chunk],
    intra_block_ms: u64,
    block_timeout_ms: u64,
) -> Result<(), Box<dyn Error>>
where
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    // Drain stale notifications.
    let drain = sleep(Duration::from_millis(150));
    tokio::pin!(drain);
    loop {
        tokio::select! {
            _ = &mut drain => break,
            n = notifications.next() => {
                if n.is_none() { return Err("notification stream ended".into()); }
            }
        }
    }

    let stream_len = chunks.iter().map(|c| c.value.len() - 1).sum::<usize>();
    peripheral.write(control, &control1_for(stream_len), WriteType::WithoutResponse).await?;
    wait_for_ack_pair(notifications, C1_ACK_A, C1_ACK_B, 1500).await?;

    sleep(Duration::from_millis(60)).await;
    peripheral.write(control, &CONTROL2, WriteType::WithoutResponse).await?;
    wait_for_ack_pair(notifications, C2_ACK_A, C2_ACK_B, 1500).await?;

    sleep(Duration::from_millis(60)).await;

    let started = Instant::now();
    let mut block_no = 0usize;
    let total_blocks = chunks.iter().filter(|c| c.ends_4k_block).count();

    for (idx, chunk) in chunks.iter().enumerate() {
        peripheral.write(bulk, &chunk.value, WriteType::WithoutResponse).await?;

        if intra_block_ms > 0 {
            sleep(Duration::from_millis(intra_block_ms)).await;
        }

        if chunk.ends_4k_block {
            block_no += 1;

            let ok = wait_for_progress(notifications, block_timeout_ms).await?;
            if !ok {
                return Err(format!(
                    "Timed out waiting for PROGRESS after 4K block {block_no}/{total_blocks} \
                     (last ACKed block {}). Stop and power-cycle before another media test.",
                    block_no - 1
                ).into());
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

    println!("All {total_blocks} progress blocks ACKed; waiting for final completion...");

    if !wait_for_completion(notifications, 8000).await? {
        return Err(
            format!(
                "No completion after all {total_blocks} block ACKs. Stop and power-cycle before another media test."
            ).into()
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let stream_path = args.windows(2)
        .find(|w| w[0] == "--stream")
        .map(|w| PathBuf::from(&w[1]))
        .unwrap_or_else(|| PathBuf::from(r"..\extracted\session_01_bulk_stream.bin"));

    let intra_block_ms: u64 = args.windows(2)
        .find(|w| w[0] == "--intra-block-ms")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(2);

    let block_timeout_ms: u64 = args.windows(2)
        .find(|w| w[0] == "--block-timeout-ms")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(2500);

    let once = args.iter().any(|a| a == "--once");
    let live = args.iter().any(|a| a == "--confirm-flow-watch");
    let allow_single_frame = args.iter().any(|a| a == "--allow-single-frame");

    log!(
        "TIGA flow-controlled stream uploader starting (run_id={})",
        std::env::var("TIGA_RUN_ID").unwrap_or_else(|_| "-".into())
    );
    println!("Stream          : {}", stream_path.display());
    println!("Intra-block pace: {} ms/write", intra_block_ms);
    println!("Flow control    : WAIT for one PROGRESS after every 4096-byte DLX block");
    println!("Mode            : {}", if once { "one upload" } else { "watch + coalesce" });

    let initial = fs::read(&stream_path)?;
    let initial_chunks = build_chunks(&initial, allow_single_frame)?;
    println!(
        "Validated: {} bytes, {} writes, {} x 4096-byte progress blocks{}",
        initial.len(),
        initial_chunks.len(),
        initial_chunks.iter().filter(|c| c.ends_4k_block).count(),
        if allow_single_frame { "  [--allow-single-frame: EXPERIMENTAL]" } else { "" }
    );

    if !live {
        println!();
        println!("DRY RUN ONLY — no Bluetooth connection was opened.");
        println!("Live mode requires --confirm-flow-watch");
        return Ok(());
    }

    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let adapter = adapters.first().ok_or("No Bluetooth adapter found")?;
    log!(
        "Using adapter 1/{}: {}",
        adapters.len(),
        adapter.adapter_info().await.unwrap_or_else(|e| format!("(info unavailable: {e})"))
    );

    log!("Scanning for {TARGET_NAME}...");
    adapter.start_scan(ScanFilter::default()).await?;
    sleep(Duration::from_secs(5)).await;

    let mut target = None;
    for p in adapter.peripherals().await? {
        let props = p.properties().await?;
        let name = props.as_ref().and_then(|p| p.local_name.as_deref()).unwrap_or("");
        if name == TARGET_NAME || name.to_ascii_lowercase().contains("zoom75") {
            target = Some(p);
            break;
        }
    }
    let peripheral = target.ok_or("Zoom75 Tiga not found")?;
    log!("Found keyboard at {}", peripheral.address());

    if !peripheral.is_connected().await? {
        peripheral.connect().await?;
    }
    peripheral.discover_services().await?;

    let chars = peripheral.characteristics();
    let control = chars.iter().find(|c| c.uuid == CHAR_CONTROL).cloned().ok_or("control char missing")?;
    let notify1 = chars.iter().find(|c| c.uuid == CHAR_NOTIFY).cloned().ok_or("notify 0002 missing")?;
    let bulk = chars.iter().find(|c| c.uuid == CHAR_BULK).cloned().ok_or("bulk 0003 missing")?;
    let notify2 = chars.iter().find(|c| c.uuid == CHAR_NOTIFY2).cloned().ok_or("notify 0004 missing")?;

    let mut notifications = peripheral.notifications().await?;
    peripheral.subscribe(&notify1).await?;
    peripheral.subscribe(&notify2).await?;
    log!("Connected once. Notifications active.");

    let mut last_uploaded: Option<Vec<u8>> = None;
    let mut last_crc = String::new();
    let mut last_frame_id = String::new();

    loop {
        let stream = fs::read(&stream_path)?;
        validate_stream(&stream, allow_single_frame)?;

        if last_uploaded.as_deref() == Some(stream.as_slice()) {
            note_already_displayed(&stream_path, &last_crc, &mut last_frame_id);
            if once { break; }
            sleep(Duration::from_millis(250)).await;
            continue;
        }

        let chunks = build_chunks(&stream, allow_single_frame)?;
        let crc = format!("{:08x}", crc32(&stream));
        let frame_id = frame_id_for(&stream_path, &crc);
        println!();
        log!("UPLOAD START frame={frame_id} crc32={crc}");

        let started = Instant::now();
        if let Err(e) = upload_stream(
            &peripheral,
            &control,
            &bulk,
            &mut notifications,
            &chunks,
            intra_block_ms,
            block_timeout_ms,
        )
        .await
        {
            log!(
                "UPLOAD FAILED frame={frame_id} crc32={crc} after {:.2}s: {e}",
                started.elapsed().as_secs_f64()
            );
            return Err(e);
        }

        let elapsed = started.elapsed().as_secs_f64();
        log!("UPLOAD COMPLETE frame={frame_id} crc32={crc} in {elapsed:.2}s");
        record_delivery(&stream_path, &frame_id, &crc, elapsed);

        last_uploaded = Some(stream);
        last_crc = crc;
        last_frame_id = frame_id;

        if once {
            break;
        }

        sleep(Duration::from_millis(100)).await;
    }

    peripheral.disconnect().await?;
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
