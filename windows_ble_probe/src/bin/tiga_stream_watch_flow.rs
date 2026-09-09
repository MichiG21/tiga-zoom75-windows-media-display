use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::Manager;
use futures::StreamExt;
use std::{
    error::Error,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
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

    println!("TIGA flow-controlled stream uploader");
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

    println!("Scanning for {TARGET_NAME}...");
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
        println!("New stream detected -> flow-controlled upload");

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

        peripheral.write(&control, &CONTROL1, WriteType::WithoutResponse).await?;
        wait_for_ack_pair(&mut notifications, C1_ACK_A, C1_ACK_B, 1500).await?;

        sleep(Duration::from_millis(60)).await;
        peripheral.write(&control, &CONTROL2, WriteType::WithoutResponse).await?;
        wait_for_ack_pair(&mut notifications, C2_ACK_A, C2_ACK_B, 1500).await?;

        sleep(Duration::from_millis(60)).await;

        let started = Instant::now();
        let mut block_no = 0usize;

        for (idx, chunk) in chunks.iter().enumerate() {
            peripheral.write(&bulk, &chunk.value, WriteType::WithoutResponse).await?;

            if intra_block_ms > 0 {
                sleep(Duration::from_millis(intra_block_ms)).await;
            }

            if chunk.ends_4k_block {
                block_no += 1;

                let ok = wait_for_progress(&mut notifications, block_timeout_ms).await?;
                if !ok {
                    return Err(format!(
                        "Timed out waiting for PROGRESS after 4K block {block_no}/53. \
                         Stop and power-cycle before another media test."
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

        if !wait_for_completion(&mut notifications, 8000).await? {
            return Err(
                "No completion after all 53 block ACKs. Stop and power-cycle before another media test."
                    .into()
            );
        }

        println!(
            "FLOW-CONTROLLED UPLOAD COMPLETE in {:.2}s",
            started.elapsed().as_secs_f64()
        );

        last_uploaded = Some(stream);

        if once {
            break;
        }

        sleep(Duration::from_millis(100)).await;
    }

    peripheral.disconnect().await?;
    println!("Disconnected cleanly.");
    Ok(())
}
