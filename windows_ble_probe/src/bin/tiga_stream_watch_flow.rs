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

fn validate_stream(stream: &[u8]) -> Result<(), Box<dyn Error>> {
    if stream.len() != STREAM_LEN {
        return Err(format!("stream size {} != expected {}", stream.len(), STREAM_LEN).into());
    }
    if stream.get(0..8) != Some(&[0x88,0,0,0,0x03,0x5d,0xe1,0][..]) {
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

fn build_chunks(stream: &[u8]) -> Result<Vec<Chunk>, Box<dyn Error>> {
    validate_stream(stream)?;
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

    if chunks.len() != CAPTURED_BULK_WRITES {
        return Err(format!("chunk count {} != expected {}", chunks.len(), CAPTURED_BULK_WRITES).into());
    }

    let blocks = chunks.iter().filter(|c| c.ends_4k_block).count();
    if blocks != 53 {
        return Err(format!("detected {blocks} full 4096-byte blocks; expected 53").into());
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

    peripheral.write(control, &CONTROL1, WriteType::WithoutResponse).await?;
    wait_for_ack_pair(notifications, C1_ACK_A, C1_ACK_B, 1500).await?;

    sleep(Duration::from_millis(60)).await;
    peripheral.write(control, &CONTROL2, WriteType::WithoutResponse).await?;
    wait_for_ack_pair(notifications, C2_ACK_A, C2_ACK_B, 1500).await?;

    sleep(Duration::from_millis(60)).await;

    let started = Instant::now();
    let mut block_no = 0usize;

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
                    "Timed out waiting for PROGRESS after 4K block {block_no}/53 \
                     (last ACKed block {}). Stop and power-cycle before another media test.",
                    block_no - 1
                ).into());
            }

            if block_no % 5 == 0 || block_no == 53 {
                println!(
                    "  block {:2}/53 ACKed  writes={:4}/{}  elapsed={:.2}s",
                    block_no,
                    idx + 1,
                    chunks.len(),
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }

    println!("All 53 progress blocks ACKed; waiting for final completion...");

    if !wait_for_completion(notifications, 8000).await? {
        return Err(
            "No completion after all 53 block ACKs. Stop and power-cycle before another media test."
                .into()
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

    log!(
        "TIGA flow-controlled stream uploader starting (run_id={})",
        std::env::var("TIGA_RUN_ID").unwrap_or_else(|_| "-".into())
    );
    println!("Stream          : {}", stream_path.display());
    println!("Intra-block pace: {} ms/write", intra_block_ms);
    println!("Flow control    : WAIT for one PROGRESS after every 4096-byte DLX block");
    println!("Mode            : {}", if once { "one upload" } else { "watch + coalesce" });

    let initial = fs::read(&stream_path)?;
    let initial_chunks = build_chunks(&initial)?;
    println!(
        "Validated: {} bytes, {} writes, 53 x 4096-byte progress blocks",
        initial.len(),
        initial_chunks.len()
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

    loop {
        let stream = fs::read(&stream_path)?;
        validate_stream(&stream)?;

        if last_uploaded.as_deref() == Some(stream.as_slice()) {
            if once { break; }
            sleep(Duration::from_millis(250)).await;
            continue;
        }

        let chunks = build_chunks(&stream)?;
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
    fn tracked_template_builds_expected_chunks() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("extracted")
            .join("session_01_bulk_stream.redgreen-backup.bin");
        let stream = fs::read(path).unwrap();
        assert_eq!(build_chunks(&stream).unwrap().len(), CAPTURED_BULK_WRITES);
    }
}
