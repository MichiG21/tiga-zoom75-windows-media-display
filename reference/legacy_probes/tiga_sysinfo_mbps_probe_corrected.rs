//! Zoom75 TIGA — corrected SYSINFO download/Mbps probe.
//!
//! We found the earlier mistake: the official 11-byte SYSINFO payload is NOT
//! five little-endian u16 values. It is structured as:
//!
//!   byte 0      : unknown flag
//!   bytes 1..2  : CPU       (big-endian)
//!   bytes 3..4  : GPU       (big-endian)
//!   bytes 5..6  : unknown   (big-endian)
//!   bytes 7..8  : RPM       (big-endian)
//!   bytes 9..10 : download  (big-endian, tenths of Mbps)
//!
//! Then a fixed 0xFF byte follows the declared 11-byte payload.
//!
//! This layout matches the user's observed CPU/GPU/RPM values exactly.
//! This probe varies ONLY bytes 9..10.
//!
//! NO FC/media commands are sent.

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

fn sysinfo_raw(cpu: u16, gpu: u16, unknown: u16, rpm: u16, mbps_tenths: u16) -> [u8; 32] {
    let mut raw = [0u8; 32];

    raw[0] = 0x1C;
    raw[1] = 0x02;
    raw[5] = 0x10; // 4 + 11 payload + 1 marker
    raw[8] = 0xA5;
    raw[9] = 0xFF;
    raw[10] = 0x00;
    raw[11] = 0x0B;

    let cpu_b = cpu.to_be_bytes();
    let gpu_b = gpu.to_be_bytes();
    let unk_b = unknown.to_be_bytes();
    let rpm_b = rpm.to_be_bytes();
    let net_b = mbps_tenths.to_be_bytes();

    raw[12] = 0x00; // unknown flag
    raw[13] = cpu_b[0];
    raw[14] = cpu_b[1];
    raw[15] = gpu_b[0];
    raw[16] = gpu_b[1];
    raw[17] = unk_b[0];
    raw[18] = unk_b[1];
    raw[19] = rpm_b[0];
    raw[20] = rpm_b[1];
    raw[21] = net_b[0];
    raw[22] = net_b[1];

    // Command-specific marker immediately after the 11-byte payload.
    raw[23] = 0xFF;

    raw[6] = 0;
    raw[7] = 0;
    let crc = crc16_ccitt_false(&raw);
    let crc_b = crc.to_le_bytes();
    raw[6] = crc_b[0];
    raw[7] = crc_b[1];

    raw
}

fn send_sysinfo(
    device: &HidDevice,
    cpu: u16,
    gpu: u16,
    unknown: u16,
    rpm: u16,
    mbps_tenths: u16,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let raw = sysinfo_raw(cpu, gpu, unknown, rpm, mbps_tenths);
    let mut hid = [0u8; 33];
    hid[1..].copy_from_slice(&raw);

    let written = device.write(&hid)?;
    if written != 33 {
        return Err(format!("{label}: wrote {written}, expected 33").into());
    }

    println!(
        "{label:<18} raw={}  expected display≈{:.1} Mbps",
        mbps_tenths,
        mbps_tenths as f32 / 10.0
    );

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
        && args.iter().any(|a| a == "--confirm-corrected-mbps-map");

    println!("Zoom75 TIGA — corrected SYSINFO Mbps probe");
    println!("-------------------------------------------");
    println!("Confirmed mapping:");
    println!("  CPU = payload bytes 1..2");
    println!("  GPU = payload bytes 3..4");
    println!("  RPM = payload bytes 7..8");
    println!("Testing:");
    println!("  Mbps = payload bytes 9..10, big-endian, tenths");
    println!("No FC/media commands.");
    println!();

    if !live {
        println!("DRY RUN ONLY.");
        println!("Live mode:");
        println!("  --send --confirm-corrected-mbps-map");
        return Ok(());
    }

    let api = HidApi::new()?;
    let device = open_tiga(&api)?;

    // Hold mapped values steady.
    const CPU: u16 = 33;
    const GPU: u16 = 44;
    const UNKNOWN: u16 = 43;
    const RPM: u16 = 111;

    println!("Watch ONLY the Mbps value.");
    println!("Expected sequence:");
    println!("  0.5 -> 1.0 -> 5.5 -> 12.3 -> 50.0 -> 99.9 Mbps");
    println!();

    for raw in [5u16, 10, 55, 123, 500, 999] {
        send_sysinfo(
            &device,
            CPU,
            GPU,
            UNKNOWN,
            RPM,
            raw,
            &format!("net raw={raw}"),
        )?;
        sleep(Duration::from_secs(2));
    }

    println!();
    println!("Returning to 0.0 Mbps...");
    send_sysinfo(&device, 43, 48, 43, 125, 0, "neutral")?;

    println!();
    println!("Done. Tell me exactly which Mbps values appeared.");
    Ok(())
}
