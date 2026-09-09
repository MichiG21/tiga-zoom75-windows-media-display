//! Zoom75 TIGA — USB PARTIAL COMMIT probe.
//!
//! Goal:
//!   Determine whether the USB FC media path can update only a few addressed
//!   pages of an already-valid custom image, then make those changes active
//!   with FC 00 FF FF, WITHOUT retransmitting the whole media file.
//!
//! Test transaction:
//!   1) page 0: exact original/current bytes (session prime)
//!   2) frame-1 page at x=158..165,y=136: magenta
//!   3) frame-2 corresponding page: magenta
//!   4) FC 00 FF FF commit
//!
//! It then waits for ENTER and attempts a second tiny transaction that restores
//! the two exact original pages and commits again.
//!
//! SAFETY:
//!   - dry run unless BOTH live flags are supplied
//!   - only 3 addressed pages + commit per transaction
//!   - exact current page 0 is preserved
//!   - no firmware flashing/reset
//!   - if the media slot becomes invalid, restore with the already-proven BLE
//!     full upload after a power-cycle.

use hidapi::{HidApi, HidDevice};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use zoom_tiga_protocol::{
    image_chunk_raw, image_end_raw, raw_report_to_hid_write_buffer, RawReport,
};

const VENDOR_ID: u16 = 0x1EA7;
const PRODUCT_ID: u16 = 0xCEDD;
const USAGE_PAGE: u16 = 0xFF60;
const USAGE: u16 = 0x0061;

const WIDTH: usize = 320;
const HEIGHT: usize = 172;
const ROW_BYTES: usize = WIDTH * 2;
const FRAME_BYTES: usize = WIDTH * HEIGHT * 2;

const STREAM_LEN: usize = 220_649;
const MEDIA_BODY_STREAM_START: usize = 33;
const MEDIA_HEADER_LEN: usize = 452;

const TEST_X: usize = 158;
const TEST_Y: usize = 136;
const TEST_PIXELS: usize = 8;
const MAGENTA_565_BE: [u8; 2] = [0xF8, 0x1F];

const ACK_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RETRIES: usize = 4;
const RETRY_BACKOFF: Duration = Duration::from_millis(120);
const BETWEEN_WRITES: Duration = Duration::from_millis(75);

fn is_media_ack(buf: &[u8]) -> bool {
    buf.len() >= 14
        && buf[0] == 0x1C
        && buf[8] == 0xA5
        && buf[9] == 0xFC
        && buf[11] == 0x02
        && buf[12] == 0xFE
        && buf[13] == 0x00
}

fn wait_for_media_ack(device: &HidDevice) -> Result<(), String> {
    let deadline = Instant::now() + ACK_TIMEOUT;
    let mut response = [0u8; 64];

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout_ms = remaining.as_millis().clamp(1, 100) as i32;

        match device.read_timeout(&mut response, timeout_ms) {
            Ok(0) => {}
            Ok(n) if is_media_ack(&response[..n]) => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(format!("HID read failed: {e}")),
        }
    }

    Err("timed out waiting for FC/FE00 media ACK".into())
}

fn send_raw(device: &HidDevice, raw: RawReport, label: &str) -> Result<(), String> {
    let hid = raw_report_to_hid_write_buffer(raw);

    for attempt in 1..=MAX_RETRIES {
        let written = device
            .write(&hid)
            .map_err(|e| format!("{label}: HID write failed: {e}"))?;

        if written != 33 {
            return Err(format!("{label}: HID write returned {written}, expected 33"));
        }

        match wait_for_media_ack(device) {
            Ok(()) => {
                println!("{label}: FC/FE00 ACK");
                return Ok(());
            }
            Err(e) if attempt < MAX_RETRIES => {
                println!("{label}: {e}; retrying {attempt}/{MAX_RETRIES}");
                sleep(RETRY_BACKOFF);
            }
            Err(e) => return Err(format!("{label}: {e} after {MAX_RETRIES} attempts")),
        }
    }

    unreachable!()
}

fn send_page(device: &HidDevice, page: u16, data: &[u8; 16], label: &str) -> Result<(), String> {
    send_raw(device, image_chunk_raw(page, data), label)
}

fn send_commit(device: &HidDevice, label: &str) -> Result<(), String> {
    send_raw(device, image_end_raw(), label)
}

fn page_bytes(stream: &[u8], page: usize) -> Result<[u8; 16], String> {
    let start = MEDIA_BODY_STREAM_START + page * 16;
    let end = start + 16;
    let src = stream
        .get(start..end)
        .ok_or_else(|| format!("page {page} outside media body"))?;

    let mut out = [0u8; 16];
    out.copy_from_slice(src);
    Ok(out)
}

fn transaction(
    device: &HidDevice,
    page0: &[u8; 16],
    page1_index: u16,
    page1: &[u8; 16],
    page2_index: u16,
    page2: &[u8; 16],
    prefix: &str,
) -> Result<Duration, String> {
    let start = Instant::now();

    send_page(device, 0, page0, &format!("{prefix} page-0 prime"))?;
    sleep(BETWEEN_WRITES);

    send_page(
        device,
        page1_index,
        page1,
        &format!("{prefix} frame-1 page"),
    )?;
    sleep(BETWEEN_WRITES);

    send_page(
        device,
        page2_index,
        page2,
        &format!("{prefix} frame-2 page"),
    )?;
    sleep(BETWEEN_WRITES);

    send_commit(device, &format!("{prefix} COMMIT"))?;

    Ok(start.elapsed())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let stream_path = args
        .windows(2)
        .find(|w| w[0] == "--stream")
        .map(|w| PathBuf::from(&w[1]))
        .ok_or("missing --stream <path>")?;

    let send = args.iter().any(|a| a == "--send");
    let confirm = args.iter().any(|a| a == "--confirm-partial-commit-probe");

    let stream = fs::read(&stream_path)?;
    if stream.len() != STREAM_LEN {
        return Err(format!("stream size {} != expected {}", stream.len(), STREAM_LEN).into());
    }

    let frame1_offset = MEDIA_HEADER_LEN + TEST_Y * ROW_BYTES + TEST_X * 2;
    let frame2_offset = MEDIA_HEADER_LEN + FRAME_BYTES + TEST_Y * ROW_BYTES + TEST_X * 2;

    if frame1_offset % 16 != 0 || frame2_offset % 16 != 0 {
        return Err("test location does not align to 16-byte FC pages".into());
    }

    let page1 = frame1_offset / 16;
    let page2 = frame2_offset / 16;

    let page0 = page_bytes(&stream, 0)?;
    let original1 = page_bytes(&stream, page1)?;
    let original2 = page_bytes(&stream, page2)?;

    let mut magenta = [0u8; 16];
    for px in 0..TEST_PIXELS {
        magenta[px * 2] = MAGENTA_565_BE[0];
        magenta[px * 2 + 1] = MAGENTA_565_BE[1];
    }

    println!("Zoom75 TIGA — USB PARTIAL COMMIT probe");
    println!("---------------------------------------");
    println!("Stream        : {}", stream_path.display());
    println!("Test location : x={TEST_X}..{}, y={TEST_Y}", TEST_X + TEST_PIXELS - 1);
    println!("Page 0        : {:02X?}", page0);
    println!("Frame-1 page  : {page1}  original={:02X?}", original1);
    println!("Frame-2 page  : {page2}  original={:02X?}", original2);
    println!("Probe page    : {:02X?}", magenta);
    println!();
    println!("Transaction is ONLY: page0 + 2 random pages + FC 00 FF FF.");
    println!("No full image transfer.");

    if !send || !confirm {
        println!();
        println!("DRY RUN ONLY — no HID device was opened.");
        println!("Live mode requires BOTH:");
        println!("  --send --confirm-partial-commit-probe");
        return Ok(());
    }

    let api = HidApi::new()?;
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == VENDOR_ID
                && d.product_id() == PRODUCT_ID
                && d.usage_page() == USAGE_PAGE
                && d.usage() == USAGE
        })
        .ok_or("Zoom75 TIGA HID interface FF60:0061 not found")?;

    let device = info.open_device(&api)?;

    println!();
    println!("TEST transaction starting.");
    println!("Expect the factory tiger to appear when page 0 opens the FC session.");
    let elapsed = transaction(
        &device,
        &page0,
        page1 as u16,
        &magenta,
        page2 as u16,
        &magenta,
        "TEST",
    )?;

    println!();
    println!("TEST commit ACKed in {:.3}s.", elapsed.as_secs_f64());
    println!("LOOK AT THE LCD NOW:");
    println!("  - Did the Spotify media return automatically?");
    println!("  - If yes, is the small magenta bar segment visible?");
    println!("  - Approximately how long was the tiger visible?");
    println!();
    println!("Press ENTER to attempt a tiny RESTORE transaction.");
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;

    let restore_elapsed = transaction(
        &device,
        &page0,
        page1 as u16,
        &original1,
        page2 as u16,
        &original2,
        "RESTORE",
    )?;

    println!();
    println!(
        "RESTORE commit ACKed in {:.3}s.",
        restore_elapsed.as_secs_f64()
    );
    println!("Power-cycle once before returning to the BLE media pipeline.");
    println!();
    println!("Report these four things:");
    println!("  1) TEST commit ACKed? yes/no");
    println!("  2) Spotify returned after commit? yes/no");
    println!("  3) Magenta segment visible? yes/no");
    println!("  4) Tiger visible for roughly how long?");
    Ok(())
}
