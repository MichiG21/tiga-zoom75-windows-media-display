//! Zoom75 TIGA — FC random-page probe + interactive screen navigation.
//!
//! Why this exists:
//! PocketWuque cannot simply select the existing Custom Upload slot without
//! starting another upload. This probe keeps the SAME USB handle/session open
//! and, after the already-proven random FC page writes, lets us send the
//! official screen-navigation commands (0x39) ourselves.
//!
//! Flow:
//!   1) page 0 prime (exact current bytes)
//!   2) random page in frame 1 -> magenta
//!   3) random page in frame 2 -> magenta
//!   4) NO COMMIT
//!   5) interactive U/D/E/R screen navigation
//!   6) on Q, restore the two original pages, still NO COMMIT
//!
//! This does not flash firmware and never sends the media end/commit packet.

use hidapi::{HidApi, HidDevice};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use zoom_tiga_protocol::{
    image_chunk_raw, raw_report_to_hid_write_buffer, screen_control_raw, ScreenMode,
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
const TEST_PIXEL: [u8; 2] = [0xF8, 0x1F]; // RGB565 BE magenta

const MEDIA_ACK_TIMEOUT: Duration = Duration::from_secs(3);
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
            return Err(format!("{label}: wrote {written}, expected 33"));
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

fn send_nav(device: &HidDevice, mode: ScreenMode, label: &str) -> Result<(), String> {
    let raw = screen_control_raw(mode);
    let hid = raw_report_to_hid_write_buffer(raw);
    let written = device
        .write(&hid)
        .map_err(|e| format!("{label}: HID write failed: {e}"))?;

    if written != 33 {
        return Err(format!("{label}: wrote {written}, expected 33"));
    }

    // Do not require a particular response. These are the already-captured
    // official 0x39 screen-navigation packets; for this probe the LCD reaction
    // is the source of truth.
    sleep(Duration::from_millis(120));
    println!("{label}: sent");
    Ok(())
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

    let live = args.iter().any(|a| a == "--send")
        && args
            .iter()
            .any(|a| a == "--confirm-interactive-nav-probe");

    let stream = fs::read(&stream_path)?;
    if stream.len() != STREAM_LEN {
        return Err(format!("stream size {} != expected {}", stream.len(), STREAM_LEN).into());
    }

    let f1_off = MEDIA_HEADER_LEN + TEST_Y * ROW_BYTES + TEST_X * 2;
    let f2_off = MEDIA_HEADER_LEN + FRAME_BYTES + TEST_Y * ROW_BYTES + TEST_X * 2;
    if f1_off % 16 != 0 || f2_off % 16 != 0 {
        return Err("test location is not FC-page aligned".into());
    }

    let page1 = f1_off / 16;
    let page2 = f2_off / 16;
    let page0 = page_bytes(&stream, 0)?;
    let original1 = page_bytes(&stream, page1)?;
    let original2 = page_bytes(&stream, page2)?;

    let mut magenta = [0u8; 16];
    for px in 0..TEST_PIXELS {
        magenta[px * 2] = TEST_PIXEL[0];
        magenta[px * 2 + 1] = TEST_PIXEL[1];
    }

    println!("Zoom75 TIGA — FC + interactive screen-nav probe");
    println!("-------------------------------------------------");
    println!("Stream       : {}", stream_path.display());
    println!("Frame 1 page : {page1}");
    println!("Frame 2 page : {page2}");
    println!("NO MEDIA COMMIT is sent by this program.");
    println!();

    if !live {
        println!("DRY RUN ONLY.");
        println!("Live mode:");
        println!("  --send --confirm-interactive-nav-probe");
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

    println!("Priming FC session + writing two magenta test pages...");
    send_page(&device, 0, &page0, "page-0 prime")?;
    sleep(Duration::from_millis(100));
    send_page(&device, page1 as u16, &magenta, "frame-1 random page")?;
    sleep(Duration::from_millis(100));
    send_page(&device, page2 as u16, &magenta, "frame-2 random page")?;

    println!();
    println!("Random pages ACKed. The factory tiger may now be visible.");
    println!("We will NOT use PocketWuque.");
    println!();
    println!("Interactive screen navigation:");
    println!("  u = screen UP");
    println!("  d = screen DOWN");
    println!("  e = ENTER");
    println!("  r = RETURN");
    println!("  q = restore original two pages and quit");
    println!();
    println!("Use ONE command at a time and watch the LCD after each.");
    println!("If the Spotify/custom screen appears, STOP navigating and inspect");
    println!("the middle of the progress bar for the magenta 8-pixel segment.");
    println!();

    loop {
        print!("nav> ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let cmd = line.trim().to_ascii_lowercase();

        match cmd.as_str() {
            "u" => send_nav(&device, ScreenMode::Up, "SCREEN UP")?,
            "d" => send_nav(&device, ScreenMode::Down, "SCREEN DOWN")?,
            "e" => send_nav(&device, ScreenMode::Enter, "SCREEN ENTER")?,
            "r" => send_nav(&device, ScreenMode::Return, "SCREEN RETURN")?,
            "q" => break,
            "" => continue,
            _ => println!("Use u, d, e, r, or q."),
        }
    }

    println!();
    println!("Restoring the exact original random pages. Still NO COMMIT...");
    // Re-prime because screen-navigation commands may have changed protocol state.
    let _ = send_page(&device, 0, &page0, "restore page-0 prime");
    sleep(Duration::from_millis(100));
    let _ = send_page(&device, page1 as u16, &original1, "restore frame-1 page");
    sleep(Duration::from_millis(100));
    let _ = send_page(&device, page2 as u16, &original2, "restore frame-2 page");

    println!();
    println!("Done. Power-cycle once before the next media upload.");
    println!("Tell me which nav commands changed the LCD and whether the");
    println!("Spotify/custom screen was reachable.");
    Ok(())
}
