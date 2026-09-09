use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::Manager;
use futures::StreamExt;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
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

fn xor8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, b| acc ^ b)
}

fn validate_stream(stream: &[u8]) -> Result<(), Box<dyn Error>> {
    if stream.len() != STREAM_LEN {
        return Err(format!("stream size {} != expected {}", stream.len(), STREAM_LEN).into());
    }
    if stream.get(0..8) != Some(&[0x88,0,0,0,0x03,0x5d,0xe1,0][..]) {
        return Err("outer 0x88 envelope mismatch".into());
    }
    if stream.get(8..25) != Some(&[0x08,0x05,0x01,0,0x09,0,0xff,0xff,0xff,0,0xff,0xff,0xff,0x02,0x02,0xff,0xff][..]) {
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

fn build_chunks(stream: &[u8]) -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
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
        chunks.push(value);
        off += n;
    }

    if chunks.len() != CAPTURED_BULK_WRITES {
        return Err(format!("chunk count {} != expected {}", chunks.len(), CAPTURED_BULK_WRITES).into());
    }
    Ok(chunks)
}

fn file_stamp(path: &Path) -> Result<(SystemTime, u64), Box<dyn Error>> {
    let m = fs::metadata(path)?;
    Ok((m.modified()?, m.len()))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let stream_path = args.windows(2)
        .find(|w| w[0] == "--stream")
        .map(|w| PathBuf::from(&w[1]))
        .unwrap_or_else(|| PathBuf::from(r"..\extracted\session_01_bulk_stream.bin"));

    let scan_secs: u64 = args.windows(2)
        .find(|w| w[0] == "--scan-seconds")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(5);

    let pacing_ms: u64 = args.windows(2)
        .find(|w| w[0] == "--pacing-ms")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(16);

    let once = args.iter().any(|a| a == "--once");
    let live = args.iter().any(|a| a == "--confirm-live-watch");

    println!("TIGA persistent stream uploader");
    println!("Stream : {}", stream_path.display());
    println!("Pacing : {} ms/write", pacing_ms);
    println!("Mode   : {}", if once { "one upload" } else { "watch + coalesce" });

    let initial = fs::read(&stream_path)?;
    validate_stream(&initial)?;
    println!("Initial stream validated: {} bytes", initial.len());

    if !live {
        println!();
        println!("DRY RUN ONLY — no Bluetooth connection was opened.");
        println!("Live mode requires --confirm-live-watch");
        return Ok(());
    }

    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    if adapters.is_empty() {
        return Err("No Bluetooth adapter found".into());
    }
    let adapter = &adapters[0];

    println!("Scanning once for {TARGET_NAME}...");
    adapter.start_scan(ScanFilter::default()).await?;
    sleep(Duration::from_secs(scan_secs)).await;

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
    println!("Connected once. Notifications active.");

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
        println!();
        println!("New stream detected -> uploading {} chunks", chunks.len());

        // Drain stale status traffic for a short moment.
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

        peripheral.write(&control, &CONTROL1, WriteType::WithoutResponse).await?;
        let d1 = sleep(Duration::from_millis(1500));
        tokio::pin!(d1);
        let (mut a, mut b) = (false, false);
        loop {
            tokio::select! {
                _ = &mut d1 => break,
                n = notifications.next() => {
                    let Some(n) = n else { return Err("notification stream ended".into()); };
                    if n.value.as_slice() == C1_ACK_A { a = true; }
                    if n.value.as_slice() == C1_ACK_B { b = true; }
                    if a && b { break; }
                }
            }
        }
        if !(a && b) {
            return Err("Control #1 ACK verification failed; aborting watch loop".into());
        }

        sleep(Duration::from_millis(60)).await;
        peripheral.write(&control, &CONTROL2, WriteType::WithoutResponse).await?;
        let d2 = sleep(Duration::from_millis(1500));
        tokio::pin!(d2);
        let (mut a2, mut b2) = (false, false);
        loop {
            tokio::select! {
                _ = &mut d2 => break,
                n = notifications.next() => {
                    let Some(n) = n else { return Err("notification stream ended".into()); };
                    if n.value.as_slice() == C2_ACK_A { a2 = true; }
                    if n.value.as_slice() == C2_ACK_B { b2 = true; }
                    if a2 && b2 { break; }
                }
            }
        }
        if !(a2 && b2) {
            return Err("Control #2 ACK verification failed; aborting watch loop".into());
        }

        sleep(Duration::from_millis(60)).await;
        let started = Instant::now();
        let mut progress_count = 0usize;
        let mut completion_seen = false;

        for (idx, chunk) in chunks.iter().enumerate() {
            peripheral.write(&bulk, chunk, WriteType::WithoutResponse).await?;

            let pace = sleep(Duration::from_millis(pacing_ms));
            tokio::pin!(pace);
            loop {
                tokio::select! {
                    _ = &mut pace => break,
                    n = notifications.next() => {
                        let Some(n) = n else { return Err("notification stream ended".into()); };
                        if n.value.as_slice() == PROGRESS { progress_count += 1; }
                        if n.value.as_slice() == COMPLETE { completion_seen = true; }
                    }
                }
            }

            if (idx + 1) % 256 == 0 || idx + 1 == chunks.len() {
                println!(
                    "  {:4}/{:4} {:5.1}%  {:.1}s  status={}",
                    idx + 1,
                    chunks.len(),
                    ((idx + 1) as f64 / chunks.len() as f64) * 100.0,
                    started.elapsed().as_secs_f64(),
                    progress_count
                );
            }
        }

        if !completion_seen {
            let final_wait = sleep(Duration::from_secs(4));
            tokio::pin!(final_wait);
            loop {
                tokio::select! {
                    _ = &mut final_wait => break,
                    n = notifications.next() => {
                        let Some(n) = n else { break; };
                        if n.value.as_slice() == PROGRESS { progress_count += 1; }
                        if n.value.as_slice() == COMPLETE {
                            completion_seen = true;
                            break;
                        }
                    }
                }
            }
        }

        if !completion_seen {
            return Err("No completion notification. Stop here and power-cycle before another media test.".into());
        }

        println!(
            "Upload complete in {:.2}s. Latest renderer updates were coalesced while this upload ran.",
            started.elapsed().as_secs_f64()
        );
        last_uploaded = Some(stream);

        if once {
            break;
        }

        // Renderer may have produced many newer frames while the full upload was running.
        // The next loop iteration reads only the newest complete file and skips all intermediates.
        sleep(Duration::from_millis(100)).await;
    }

    peripheral.disconnect().await?;
    println!("Disconnected cleanly.");
    Ok(())
}
