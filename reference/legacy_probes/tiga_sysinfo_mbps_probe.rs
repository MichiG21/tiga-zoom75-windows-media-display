//! Zoom75 TIGA — SYSINFO Mbps field probe.
//!
//! Confirmed from the user's live display:
//!   u16[1] = CPU
//!   u16[2] = GPU
//!   u16[4] = RPM
//!
//! In every captured MeletrixID packet u16[0] was 0 while the LCD showed
//! Mbps = 0.0. This probe varies ONLY u16[0] to test whether it is network
//! throughput and to determine the display scaling.
//!
//! No FC/media commands are sent.

use hidapi::{HidApi, HidDevice};
use std::thread::sleep;
use std::time::Duration;

const VENDOR_ID: u16 = 0x1EA7;
const PRODUCT_ID: u16 = 0xCEDD;
const USAGE_PAGE: u16 = 0xFF60;
const USAGE: u16 = 0x0061;

fn crc16_ccitt_false(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if (crc & 0x8000) != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn sysinfo_raw(fields: [u16; 5]) -> [u8; 32] {
    let mut raw = [0u8; 32];

    raw[0] = 0x1C;
    raw[1] = 0x02;
    raw[5] = 0x10;
    raw[8] = 0xA5;
    raw[9] = 0xFF;
    raw[10] = 0x00;
    raw[11] = 0x0B;

    for (i, value) in fields.iter().enumerate() {
        let b = value.to_le_bytes();
        raw[12 + i * 2] = b[0];
        raw[13 + i * 2] = b[1];
    }

    raw[22] = 0x00;
    raw[23] = 0xFF;

    raw[6] = 0;
    raw[7] = 0;
    let crc = crc16_ccitt_false(&raw);
    let c = crc.to_le_bytes();
    raw[6] = c[0];
    raw[7] = c[1];

    raw
}

fn send(device: &HidDevice, fields: [u16; 5], label: &str) -> Result<(), Box<dyn std::error::Error>> {
    let raw = sysinfo_raw(fields);
    let mut hid = [0u8; 33];
    hid[1..].copy_from_slice(&raw);

    let written = device.write(&hid)?;
    if written != 33 {
        return Err(format!("{label}: wrote {written}, expected 33").into());
    }

    println!("{label:<24} raw fields={fields:?}");

    let mut buf = [0u8; 64];
    if let Ok(n) = device.read_timeout(&mut buf, 100) {
        if n > 0 {
            println!("  RX {:02X?}", &buf[..n]);
        }
    }
    Ok(())
}

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
        && args.iter().any(|a| a == "--confirm-mbps-map");

    println!("Zoom75 TIGA — SYSINFO Mbps field probe");
    println!("---------------------------------------");
    println!("Confirmed: u16[1]=CPU, u16[2]=GPU, u16[4]=RPM");
    println!("Testing : u16[0] only");
    println!("u16[3] stays fixed at captured value 43");
    println!("No FC/media commands.");
    println!();

    if !live {
        println!("DRY RUN ONLY.");
        println!("Live mode:");
        println!("  --send --confirm-mbps-map");
        return Ok(());
    }

    let api = HidApi::new()?;
    let device = open_tiga(&api)?;

    // Keep the already-mapped values obvious and stable.
    const CPU: u16 = 33;
    const GPU: u16 = 44;
    const UNKNOWN3: u16 = 43;
    const RPM: u16 = 111;

    println!("Watch the Mbps readout only.");
    println!("Raw u16[0] will step: 1 -> 5 -> 10 -> 25 -> 100 -> 250 -> 1000");
    println!("Each value is held for two seconds.");
    println!();

    for v in [1u16, 5, 10, 25, 100, 250, 1000] {
        send(&device, [v, CPU, GPU, UNKNOWN3, RPM], &format!("network raw={v}"))?;
        sleep(Duration::from_secs(2));
    }

    println!();
    println!("Returning network field to 0...");
    send(&device, [0, 43, 48, 43, 125], "captured-ish neutral")?;

    println!();
    println!("Done.");
    println!("Tell me the Mbps values you saw for raw:");
    println!("  1, 5, 10, 25, 100, 250, 1000");
    Ok(())
}
