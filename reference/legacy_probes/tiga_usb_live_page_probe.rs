//! Zoom75 TIGA — tiny USB random-access live-page probe.
//!
//! Purpose:
//!   Test whether the already-installed custom media can be changed in-place by
//!   sending only TWO addressed 16-byte FC pages over the proven USB HID path.
//!
//! Safety design:
//!   - DRY RUN by default.
//!   - No FC end/commit packet is sent.
//!   - Reads the original page bytes from the current valid BLE stream.
//!   - Live mode temporarily replaces an 8-pixel segment in BOTH identical
//!     frames with magenta, waits for Enter, then writes the original bytes back.
//!
//! If the LCD changes immediately while the custom Spotify screen is visible,
//! we have the random-access primitive needed for a genuinely live progress bar.

use hidapi::{HidApi, HidDevice};
use std::fs;
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

// Captured BLE stream layout:
//   8 B outer envelope
//  17 B upload preamble
//   8 B DLX prefix
// 452 B media header
// frame 1
// frame 2
//   4 B trailer
const DLX_FILE_START: usize = 25;
const DLX_PREFIX_LEN: usize = 8;
const MEDIA_HEADER_START: usize = DLX_FILE_START + DLX_PREFIX_LEN; // stream offset 33
const MEDIA_HEADER_LEN: usize = 452;

// Choose a location inside the current progress bar.
// Header offset mod 16 is 4, therefore x = 158 aligns the pixel data exactly
// to a 16-byte USB media page.
const TEST_X: usize = 158;
const TEST_Y: usize = 136;
const TEST_PIXELS: usize = 8;

// RGB565 big-endian magenta.
const TEST_PIXEL: [u8; 2] = [0xF8, 0x1F];

const MEDIA_ACK_TIMEOUT: Duration = Duration::from_millis(350);
const MAX_RETRIES: usize = 4;
const RETRY_BACKOFF: Duration = Duration::from_millis(30);

fn is_media_ack(buf: &[u8]) -> bool {
    buf.len() >= 15
        && buf[0] == 0x1C
        && buf[8] == 0xA5
        && buf[9] == 0xFC
        && buf[11] == 0x02
        && buf[12] == 0xFE
        && buf[13] == 0x00
}

fn wait_for_media_ack(device: &HidDevice) -> Result<(), String> {
    let deadline = Instant::now() + MEDIA_ACK_TIMEOUT;
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
            return Err(format!("{label}: HID write returned {written}, expected 33"));
        }

        match wait_for_media_ack(device) {
            Ok(()) => {
                if attempt > 1 {
                    println!("{label}: ACK recovered on attempt {attempt}");
                }
                return Ok(());
            }
            Err(e) if attempt < MAX_RETRIES => {
                println!("{label}: {e}; retrying...");
                sleep(RETRY_BACKOFF);
            }
            Err(e) => return Err(format!("{label}: {e} after {MAX_RETRIES} attempts")),
        }
    }

    unreachable!()
}

fn stream_page_bytes(stream: &[u8], page: usize) -> Result<[u8; 16], String> {
    // USB FC page 0 in the previously tested crossover uploader corresponded
    // to media-header byte 0, i.e. BLE stream offset 33.
    let start = MEDIA_HEADER_START + page * 16;
    let end = start + 16;
    let src = stream
        .get(start..end)
        .ok_or_else(|| format!("page {page} is outside the captured media body"))?;
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
        .unwrap_or_else(|| {
            PathBuf::from(
                r"..\..\..\tiga_ble_re_tools\extracted\session_01_bulk_stream.bin",
            )
        });

    let send = args.iter().any(|a| a == "--send");
    let confirm = args.iter().any(|a| a == "--confirm-live-page-probe");

    let stream = fs::read(&stream_path)?;
    if stream.len() != STREAM_LEN {
        return Err(format!(
            "stream size {} != expected {} ({})",
            stream.len(),
            STREAM_LEN,
            stream_path.display()
        )
        .into());
    }

    // Media-body byte offsets, excluding the DLX 8-byte prefix.
    let frame1_pixel_offset =
        MEDIA_HEADER_LEN + TEST_Y * ROW_BYTES + TEST_X * 2;
    let frame2_pixel_offset =
        MEDIA_HEADER_LEN + FRAME_BYTES + TEST_Y * ROW_BYTES + TEST_X * 2;

    if frame1_pixel_offset % 16 != 0 || frame2_pixel_offset % 16 != 0 {
        return Err("chosen test pixel is not FC-page aligned".into());
    }

    let page1 = frame1_pixel_offset / 16;
    let page2 = frame2_pixel_offset / 16;

    let original1 = stream_page_bytes(&stream, page1)?;
    let original2 = stream_page_bytes(&stream, page2)?;

    let mut test_page = [0u8; 16];
    for px in 0..TEST_PIXELS {
        test_page[px * 2] = TEST_PIXEL[0];
        test_page[px * 2 + 1] = TEST_PIXEL[1];
    }

    println!("Zoom75 TIGA — USB live-page probe");
    println!("---------------------------------");
    println!("Source stream : {}", stream_path.display());
    println!("Test location : x={TEST_X}..{}, y={TEST_Y}", TEST_X + TEST_PIXELS - 1);
    println!("Test color    : RGB565-BE magenta F8 1F");
    println!("Frame 1 page  : {page1}");
    println!("Frame 2 page  : {page2}");
    println!("Original F1   : {:02X?}", original1);
    println!("Original F2   : {:02X?}", original2);
    println!("Probe bytes   : {:02X?}", test_page);
    println!();
    println!("IMPORTANT: no FC end/commit packet will be sent.");

    if !send || !confirm {
        println!();
        println!("DRY RUN ONLY — no HID device was opened.");
        println!("Live mode requires BOTH:");
        println!("  --send --confirm-live-page-probe");
        return Ok(());
    }

    println!();
    println!("LIVE TEST:");
    println!("  Keep the Spotify custom screen visible.");
    println!("  PocketWuque/MeletrixID should be closed.");
    println!("  The USB cable must be connected.");
    println!();

    let api = HidApi::new()?;
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == VENDOR_ID
                && d.product_id() == PRODUCT_ID
                && d.usage_page() == USAGE_PAGE
                && d.usage() == USAGE
        })
        .ok_or("Zoom75 TIGA HID interface 1EA7:CEDD / FF60:0061 not found")?;

    println!(
        "Found {:04X}:{:04X}, usage_page=0x{:04X}, usage=0x{:04X}",
        info.vendor_id(),
        info.product_id(),
        info.usage_page(),
        info.usage()
    );

    let device = info.open_device(&api)?;

    println!("Writing ONLY the two test pages...");
    send_page(&device, page1 as u16, &test_page, "frame-1 test page")?;
    sleep(Duration::from_millis(75));
    send_page(&device, page2 as u16, &test_page, "frame-2 test page")?;

    println!();
    println!("TEST PAGES ACKED.");
    println!("Look at the progress bar NOW.");
    println!("If random-access writes affect the live media, you should see an");
    println!("8-pixel MAGENTA segment around the middle of the bar.");
    println!();
    println!("Press ENTER to restore the exact original page bytes.");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;

    println!("Restoring original bytes...");
    send_page(&device, page1 as u16, &original1, "frame-1 restore")?;
    sleep(Duration::from_millis(75));
    send_page(&device, page2 as u16, &original2, "frame-2 restore")?;

    println!("Restore pages ACKed. No end/commit packet was sent.");
    println!();
    println!("Tell me exactly one of these:");
    println!("  A) magenta appeared immediately");
    println!("  B) nothing changed");
    println!("  C) screen glitched/restarted/changed page");
    Ok(())
}
