//! Zoom75 TIGA — telemetry theme/layout explorer.
//!
//! Replays the three exact official Theme (0xFD) packets captured from the
//! Meletrix web console. The only changing byte between them is theme_id 1/2/3.
//! There are NO FC/media writes and NO firmware operations.
//!
//! Goal:
//!   While the telemetry page is visible and the Spotify SYSINFO bridge is
//!   running, determine whether theme_id changes only colors/decorations or
//!   actually changes the telemetry layout/widgets.
//!
//! Each theme is held for 5 seconds. The final packet returns to theme 1,
//! matching the first captured preset.

use hidapi::{HidApi, HidDevice};
use std::thread::sleep;
use std::time::Duration;

const VENDOR_ID: u16 = 0x1EA7;
const PRODUCT_ID: u16 = 0xCEDD;
const USAGE_PAGE: u16 = 0xFF60;
const USAGE: u16 = 0x0061;

const THEME1: [u8; 32] = [
    0x1c,0x02,0x00,0x00,0x00,0x0b,0xc1,0x65,0xa5,0xfd,0x00,0x06,
    0x00,0xab,0xf9,0x00,0x00,0x01,0x57,0x00,0x00,0x00,0x00,0x00,
    0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
];

const THEME2: [u8; 32] = [
    0x1c,0x02,0x00,0x00,0x00,0x0b,0x85,0xfd,0xa5,0xfd,0x00,0x06,
    0x00,0xab,0xf9,0x00,0x00,0x02,0x56,0x00,0x00,0x00,0x00,0x00,
    0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
];

const THEME3: [u8; 32] = [
    0x1c,0x02,0x00,0x00,0x00,0x0b,0xda,0xde,0xa5,0xfd,0x00,0x06,
    0x00,0xab,0xf9,0x00,0x00,0x03,0x55,0x00,0x00,0x00,0x00,0x00,
    0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
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

fn send(device: &HidDevice, raw: &[u8; 32], name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut hid = [0u8; 33];
    hid[1..].copy_from_slice(raw);
    let written = device.write(&hid)?;
    if written != 33 {
        return Err(format!("{name}: wrote {written}, expected 33").into());
    }

    println!("{name} sent. Watch the telemetry page for 5 seconds...");
    let mut buf = [0u8; 64];
    if let Ok(n) = device.read_timeout(&mut buf, 150) {
        if n > 0 {
            println!("  RX {:02X?}", &buf[..n]);
        }
    }
    sleep(Duration::from_millis(4850));
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let live = args.iter().any(|a| a == "--send")
        && args.iter().any(|a| a == "--confirm-theme-explorer");

    println!("Zoom75 TIGA — telemetry theme/layout explorer");
    println!("----------------------------------------------");
    println!("Packets: exact captured official Theme 1 / 2 / 3");
    println!("FC/media writes: NONE");
    println!("Firmware writes: NONE");
    println!("Hold time: 5 seconds per theme");
    println!();

    if !live {
        println!("DRY RUN ONLY.");
        println!("Live mode:");
        println!("  --send --confirm-theme-explorer");
        return Ok(());
    }

    let api = HidApi::new()?;
    let device = open_tiga(&api)?;

    println!("Keep the telemetry page visible.");
    println!("Your Spotify SYSINFO bridge may keep running.");
    println!();

    send(&device, &THEME1, "THEME 1")?;
    send(&device, &THEME2, "THEME 2")?;
    send(&device, &THEME3, "THEME 3")?;

    println!();
    println!("Returning to captured THEME 1...");
    send(&device, &THEME1, "THEME 1 restore")?;

    println!();
    println!("Done.");
    println!("Tell me for Theme 1 / 2 / 3:");
    println!("  - layout/widget positions changed? yes/no");
    println!("  - labels changed? yes/no");
    println!("  - only colors/background/decorations changed? yes/no");
    Ok(())
}
