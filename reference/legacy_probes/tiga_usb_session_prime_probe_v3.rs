//! Zoom75 TIGA — USB random-access probe v3.
//!
//! Difference from v1:
//!   v1 jumped directly to a high FC page and the keyboard rejected it.
//!   v2 first sends the EXACT existing page 0 to prime/open a USB FC media
//!   session, then probes a high page.
//!
//! SAFETY:
//!   - dry-run by default
//!   - exact original page 0 is used
//!   - NO FC end/commit packet is ever sent
//!   - if the high-page probe is accepted, its original bytes are written back
//!   - after a live test, power-cycle before doing another media upload

use hidapi::{HidApi, HidDevice};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use zoom_tiga_protocol::{image_chunk_raw, raw_report_to_hid_write_buffer};

const VENDOR_ID: u16 = 0x1EA7;
const PRODUCT_ID: u16 = 0xCEDD;
const USAGE_PAGE: u16 = 0xFF60;
const USAGE: u16 = 0x0061;

const WIDTH: usize = 320;
const HEIGHT: usize = 172;
const ROW_BYTES: usize = WIDTH * 2;
const FRAME_BYTES: usize = WIDTH * HEIGHT * 2;

const STREAM_LEN: usize = 220_649;
const MEDIA_BODY_STREAM_START: usize = 33; // 8 outer + 17 preamble + 8 DLX prefix
const MEDIA_HEADER_LEN: usize = 452;

const TEST_X: usize = 158; // chosen so 8 pixels == one aligned 16-byte FC page
const TEST_Y: usize = 136;
const TEST_PIXELS: usize = 8;
const TEST_PIXEL: [u8; 2] = [0xF8, 0x1F]; // RGB565-BE magenta

const ACK_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RETRIES: usize = 4;
const RETRY_BACKOFF: Duration = Duration::from_millis(150);

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
            Ok(n) => {
                if is_media_ack(&response[..n]) {
                    return Ok(());
                }
            }
            Err(e) => return Err(format!("HID read failed: {e}")),
        }
    }
    Err("timed out waiting for FC/FE00 media ACK".into())
}

fn send_page(device: &HidDevice, page: u16, data: &[u8; 16], label: &str) -> Result<(), String> {
    let raw = image_chunk_raw(page, data);
    let hid = raw_report_to_hid_write_buffer(raw);

    for attempt in 1..=MAX_RETRIES {
        let written = device
            .write(&hid)
            .map_err(|e| format!("{label}: HID write failed: {e}"))?;

        if written != 33 {
            return Err(format!("{label}: wrote {written} bytes, expected 33"));
        }

        match wait_for_media_ack(device) {
            Ok(()) => {
                println!("{label}: FC/FE00 ACK");
                return Ok(());
            }
            Err(e) if attempt < MAX_RETRIES => {
                println!("{label}: {e}; retry {attempt}/{MAX_RETRIES}");
                sleep(RETRY_BACKOFF);
            }
            Err(e) => return Err(format!("{label}: {e} after {MAX_RETRIES} attempts")),
        }
    }
    unreachable!()
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let stream_path = args
        .windows(2)
        .find(|w| w[0] == "--stream")
        .map(|w| PathBuf::from(&w[1]))
        .ok_or("missing --stream <path>")?;

    let send = args.iter().any(|a| a == "--send");
    let confirm = args.iter().any(|a| a == "--confirm-session-prime-probe");

    let stream = fs::read(&stream_path)?;
    if stream.len() != STREAM_LEN {
        return Err(format!("stream size {} != expected {}", stream.len(), STREAM_LEN).into());
    }

    let frame1_offset = MEDIA_HEADER_LEN + TEST_Y * ROW_BYTES + TEST_X * 2;
    let frame2_offset = MEDIA_HEADER_LEN + FRAME_BYTES + TEST_Y * ROW_BYTES + TEST_X * 2;

    if frame1_offset % 16 != 0 || frame2_offset % 16 != 0 {
        return Err("test location is not aligned to FC pages".into());
    }

    let page1 = frame1_offset / 16;
    let page2 = frame2_offset / 16;

    let page0 = page_bytes(&stream, 0)?;
    let original1 = page_bytes(&stream, page1)?;
    let original2 = page_bytes(&stream, page2)?;

    let mut magenta = [0u8; 16];
    for i in 0..TEST_PIXELS {
        magenta[i * 2] = TEST_PIXEL[0];
        magenta[i * 2 + 1] = TEST_PIXEL[1];
    }

    println!("Zoom75 TIGA — USB SESSION-PRIME random-access probe v3");
    println!("-------------------------------------------------------");
    println!("Stream        : {}", stream_path.display());
    println!("Exact page 0  : {:02X?}", page0);
    println!("Frame-1 page  : {page1}");
    println!("Frame-2 page  : {page2}");
    println!("Original F1   : {:02X?}", original1);
    println!("Original F2   : {:02X?}", original2);
    println!("Probe bytes   : {:02X?}", magenta);
    println!();
    println!("No FC end/commit packet exists anywhere in this program.");
    println!("ACK timeout    : 3 seconds (matches our known-good USB uploader)");

    if !send || !confirm {
        println!();
        println!("DRY RUN ONLY.");
        println!("Live mode requires:");
        println!("  --send --confirm-session-prime-probe");
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
    println!("STEP 1 — prime USB FC media session using the exact CURRENT page 0.");
    send_page(&device, 0, &page0, "page-0 prime")?;

    sleep(Duration::from_millis(100));

    println!();
    println!("STEP 2 — jump directly to frame-1 page {page1}.");
    match send_page(&device, page1 as u16, &magenta, "frame-1 random page") {
        Ok(()) => {}
        Err(e) => {
            println!();
            println!("RESULT: HIGH PAGE REJECTED EVEN AFTER PAGE-0 PRIME.");
            println!("This strongly suggests USB FC expects a sequential transfer/session.");
            println!("Error: {e}");
            println!();
            println!("No commit was sent. Power-cycle the keyboard before the next media test.");
            return Ok(());
        }
    }

    sleep(Duration::from_millis(100));

    println!();
    println!("STEP 3 — jump to corresponding frame-2 page {page2}.");
    let frame2_ok = match send_page(&device, page2 as u16, &magenta, "frame-2 random page") {
        Ok(()) => true,
        Err(e) => {
            println!("Frame-2 page was rejected: {e}");
            false
        }
    };

    println!();
    println!("HIGH-PAGE WRITE WAS ACKED.");
    println!("Look at the LCD now. Did the magenta segment appear WITHOUT a commit?");
    println!("Press ENTER after looking; then original page bytes will be written back");
    println!("where the session still accepts them. NO COMMIT will be sent.");
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;

    println!("Attempting in-session restore...");
    let _ = send_page(&device, page1 as u16, &original1, "restore frame-1");
    if frame2_ok {
        let _ = send_page(&device, page2 as u16, &original2, "restore frame-2");
    }

    println!();
    println!("Done. NO COMMIT was sent.");
    println!("Power-cycle the keyboard before restarting the BLE media pipeline.");
    println!();
    println!("Report:");
    println!("  A) high page rejected after page-0 prime");
    println!("  B) high page ACKed, but LCD did not change");
    println!("  C) high page ACKed and magenta appeared live");
    Ok(())
}
