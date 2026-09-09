//! Zoom75 TIGA — SYSINFO field-mapping probe.
//!
//! We already mapped from the user's own MeletrixID capture:
//!   payload u16 #1 -> CPU display
//!   payload u16 #4 -> RPM display
//!
//! The two constant captured fields were:
//!   payload u16 #2 = 48
//!   payload u16 #3 = 43
//!
//! This probe varies #2 and #3 independently so we can identify GPU vs Mbps.
//! It never sends FC/media commands and does not touch the custom-media slot.

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
    raw[5] = 0x10; // 4 + 11-byte payload + 1
    raw[8] = 0xA5;
    raw[9] = 0xFF;
    raw[10] = 0x00;
    raw[11] = 0x0B;

    // 5 little-endian u16 values = 10 bytes
    for (i, value) in fields.iter().enumerate() {
        let b = value.to_le_bytes();
        raw[12 + i * 2] = b[0];
        raw[13 + i * 2] = b[1];
    }

    // 11th payload byte; zero in the captured packets.
    raw[22] = 0x00;

    // Command 0xFF is the special case we observed: fixed 0xFF after payload.
    raw[23] = 0xFF;

    raw[6] = 0;
    raw[7] = 0;
    let crc = crc16_ccitt_false(&raw);
    let c = crc.to_le_bytes();
    raw[6] = c[0];
    raw[7] = c[1];

    raw
}

fn send_sysinfo(device: &HidDevice, fields: [u16; 5], label: &str) -> Result<(), Box<dyn std::error::Error>> {
    let raw = sysinfo_raw(fields);
    let mut hid = [0u8; 33];
    hid[1..].copy_from_slice(&raw);

    let written = device.write(&hid)?;
    if written != 33 {
        return Err(format!("{label}: wrote {written}, expected 33").into());
    }

    println!("{label:<28} fields={fields:?}  crc={:02X}{:02X}", raw[7], raw[6]);

    // Short observational read only. We don't require a specific ACK.
    let mut buf = [0u8; 64];
    if let Ok(n) = device.read_timeout(&mut buf, 120) {
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
        && args.iter().any(|a| a == "--confirm-field-map");

    println!("Zoom75 TIGA — SYSINFO field-mapping probe");
    println!("------------------------------------------");
    println!("Known from capture:");
    println!("  field #1 = CPU");
    println!("  field #4 = RPM");
    println!();
    println!("This probe changes ONLY fields #2 and #3.");
    println!("No FC/media commands are sent.");
    println!();

    if !live {
        println!("DRY RUN ONLY.");
        println!("Live mode:");
        println!("  --send --confirm-field-map");
        return Ok(());
    }

    let api = HidApi::new()?;
    let device = open_tiga(&api)?;

    // Keep already-mapped CPU/RPM fixed at easy-to-recognize values.
    // Field layout: [reserved?, CPU, unknown-A, unknown-B, RPM]
    const CPU: u16 = 33;
    const RPM: u16 = 111;

    println!("PHASE A — vary field #2 only.");
    println!("Watch GPU and Mbps. One of them should step 10 -> 25 -> 40 -> 55.");
    println!();

    for v in [10u16, 25, 40, 55] {
        send_sysinfo(&device, [0, CPU, v, 43, RPM], &format!("A: field2={v}"))?;
        sleep(Duration::from_secs(2));
    }

    println!();
    println!("PHASE B — vary field #3 only.");
    println!("Watch GPU and Mbps. One of them should step 7 -> 17 -> 27 -> 37.");
    println!();

    for v in [7u16, 17, 27, 37] {
        send_sysinfo(&device, [0, CPU, 48, v, RPM], &format!("B: field3={v}"))?;
        sleep(Duration::from_secs(2));
    }

    println!();
    println!("PHASE C — return to captured-ish neutral values.");
    send_sysinfo(&device, [0, 43, 48, 43, 125], "neutral")?;

    println!();
    println!("Done.");
    println!("Tell me:");
    println!("  field #2 changed: GPU / Mbps / neither");
    println!("  field #3 changed: GPU / Mbps / neither");
    Ok(())
}
