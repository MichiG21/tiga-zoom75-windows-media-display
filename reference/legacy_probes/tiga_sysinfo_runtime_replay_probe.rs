//! Zoom75 TIGA — official SYSINFO runtime replay probe.
//!
//! This probe replays six exact 32-byte command 0xFF reports captured from
//! MeletrixID while it was running. In the USBPcap capture those reports were
//! emitted about every 2 seconds. There is NO FC media command anywhere here.
//!
//! Goal:
//!   Confirm that command 0xFF is a safe runtime/telemetry channel that does
//!   not kick the custom animation slot back to the factory tiger.
//!
//! Test:
//!   1) Put the Spotify custom upload on-screen.
//!   2) Run this probe.
//!   3) Watch whether Spotify stays visible for the whole ~12 second replay.
//!
//! The six packets are byte-for-byte from the user's own "MeletrixID running"
//! capture, including their original CRCs.

use hidapi::{HidApi, HidDevice};
use std::thread::sleep;
use std::time::Duration;

const VENDOR_ID: u16 = 0x1EA7;
const PRODUCT_ID: u16 = 0xCEDD;
const USAGE_PAGE: u16 = 0xFF60;
const USAGE: u16 = 0x0061;

const REPORTS: [[u8; 32]; 6] = [
    [
        0x1c,0x02,0x00,0x00,0x00,0x10,0x69,0xbe,0xa5,0xff,0x00,0x0b,0x00,0x00,0x2b,0x00,
        0x30,0x00,0x2b,0x00,0x7d,0x00,0x00,0xff,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00
    ],
    [
        0x1c,0x02,0x00,0x00,0x00,0x10,0xdd,0x89,0xa5,0xff,0x00,0x0b,0x00,0x00,0x2d,0x00,
        0x30,0x00,0x2b,0x00,0x6d,0x00,0x00,0xff,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00
    ],
    [
        0x1c,0x02,0x00,0x00,0x00,0x10,0x52,0x9b,0xa5,0xff,0x00,0x0b,0x00,0x00,0x34,0x00,
        0x30,0x00,0x2b,0x00,0x3b,0x00,0x00,0xff,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00
    ],
    [
        0x1c,0x02,0x00,0x00,0x00,0x10,0xd0,0x8b,0xa5,0xff,0x00,0x0b,0x00,0x00,0x2b,0x00,
        0x30,0x00,0x2b,0x00,0x85,0x00,0x00,0xff,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00
    ],
    [
        0x1c,0x02,0x00,0x00,0x00,0x10,0x8e,0x37,0xa5,0xff,0x00,0x0b,0x00,0x00,0x2f,0x00,
        0x30,0x00,0x2b,0x00,0xb8,0x00,0x00,0xff,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00
    ],
    [
        0x1c,0x02,0x00,0x00,0x00,0x10,0x5d,0x94,0xa5,0xff,0x00,0x0b,0x00,0x00,0x24,0x00,
        0x30,0x00,0x2b,0x00,0x8e,0x00,0x00,0xff,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00
    ],
];

fn open_tiga(api: &HidApi) -> Result<HidDevice, Box<dyn std::error::Error>> {
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == VENDOR_ID
                && d.product_id() == PRODUCT_ID
                && d.usage_page() == USAGE_PAGE
                && d.usage() == USAGE
        })
        .ok_or("Zoom75 TIGA HID interface FF60:0061 not found")?;
    Ok(info.open_device(api)?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let live = args.iter().any(|a| a == "--send")
        && args.iter().any(|a| a == "--confirm-sysinfo-replay");

    println!("Zoom75 TIGA — official SYSINFO runtime replay probe");
    println!("----------------------------------------------------");
    println!("Command        : 0xFF");
    println!("Reports        : 6 exact captured MeletrixID packets");
    println!("Cadence        : 2 seconds");
    println!("FC/media writes: NONE");
    println!("Expected time  : ~12 seconds");
    println!();

    if !live {
        println!("DRY RUN ONLY — no HID device was opened.");
        println!("Live mode requires:");
        println!("  --send --confirm-sysinfo-replay");
        return Ok(());
    }

    let api = HidApi::new()?;
    let device = open_tiga(&api)?;

    println!("Connected.");
    println!("Keep the Spotify custom animation visible and watch the LCD.");
    println!();

    for (i, raw) in REPORTS.iter().enumerate() {
        let mut hid = [0u8; 33];
        hid[1..].copy_from_slice(raw);

        let written = device.write(&hid)?;
        if written != 33 {
            return Err(format!("write {} returned {}, expected 33", i + 1, written).into());
        }

        println!(
            "sent SYSINFO {}/6  payload={:02X?}",
            i + 1,
            &raw[12..23]
        );

        // Read briefly only for observation; do not require a particular ACK.
        let mut buf = [0u8; 64];
        match device.read_timeout(&mut buf, 250) {
            Ok(n) if n > 0 => println!("  RX {:02X?}", &buf[..n]),
            Ok(_) => println!("  RX <none in 250 ms>"),
            Err(e) => println!("  RX read warning: {e}"),
        }

        if i + 1 < REPORTS.len() {
            sleep(Duration::from_millis(1750));
        }
    }

    println!();
    println!("Replay finished. No FC/media session was opened.");
    println!("Report:");
    println!("  A) Spotify stayed visible unchanged");
    println!("  B) Spotify stayed visible but something on it changed");
    println!("  C) LCD switched away / tiger appeared / glitched");
    Ok(())
}
