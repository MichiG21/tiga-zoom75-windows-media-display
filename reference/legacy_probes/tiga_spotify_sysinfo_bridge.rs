//! Zoom75 TIGA — Spotify SYSINFO runtime bridge.
//!
//! Reads a tiny text file produced by tiga_spotify_sysinfo_values_v1.py:
//!     <progress_tenths_percent> <elapsed_seconds> <playing>
//!
//! Runtime mapping:
//!   Mbps = Spotify progress percent, e.g. 54.3
//!   RPM  = elapsed track seconds
//!
//! This is deliberately a runtime-channel test, not the final UI.
//! It never sends FC/media commands, so it must not replace the custom-media slot.

use hidapi::{HidApi, HidDevice};
use std::fs;
use std::path::PathBuf;
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
    raw[5] = 0x10;
    raw[8] = 0xA5;
    raw[9] = 0xFF;
    raw[10] = 0x00;
    raw[11] = 0x0B;

    let cpu_b = cpu.to_be_bytes();
    let gpu_b = gpu.to_be_bytes();
    let unk_b = unknown.to_be_bytes();
    let rpm_b = rpm.to_be_bytes();
    let net_b = mbps_tenths.to_be_bytes();

    raw[12] = 0x00;
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
    raw[23] = 0xFF;

    raw[6] = 0;
    raw[7] = 0;
    let crc = crc16_ccitt_false(&raw);
    let c = crc.to_le_bytes();
    raw[6] = c[0];
    raw[7] = c[1];
    raw
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

fn parse_values(text: &str) -> Option<(u16, u16, bool)> {
    let mut it = text.split_whitespace();
    let progress: u16 = it.next()?.parse().ok()?;
    let elapsed: u16 = it.next()?.parse().ok()?;
    let playing: u8 = it.next()?.parse().ok()?;
    Some((progress.min(1000), elapsed, playing != 0))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let values_path = args
        .windows(2)
        .find(|w| w[0] == "--values")
        .map(|w| PathBuf::from(&w[1]))
        .unwrap_or_else(|| PathBuf::from(r"..\..\..\tiga_ble_re_tools\spotify_sysinfo_values.txt"));

    let live = args.iter().any(|a| a == "--send")
        && args.iter().any(|a| a == "--confirm-spotify-sysinfo-bridge");

    println!("Zoom75 TIGA — Spotify SYSINFO runtime bridge");
    println!("---------------------------------------------");
    println!("Values file : {}", values_path.display());
    println!("Mbps widget : Spotify progress %");
    println!("RPM widget  : elapsed track seconds");
    println!("Update      : 1 Hz");
    println!("FC/media    : NONE");
    println!();

    if !live {
        println!("DRY RUN ONLY.");
        println!("Live mode:");
        println!("  --send --confirm-spotify-sysinfo-bridge");
        return Ok(());
    }

    let api = HidApi::new()?;
    let device = open_tiga(&api)?;

    // For this bridge test we hold CPU/GPU at harmless recognizable values.
    // We only care about proving the two repurposed carriers are stable at 1 Hz.
    const CPU: u16 = 33;
    const GPU: u16 = 44;
    const UNKNOWN: u16 = 43;

    let mut last: Option<(u16, u16, bool)> = None;

    println!("Connected. Ctrl+C closes the process.");
    println!("Stay on the telemetry screen to watch the two carriers.");
    println!();

    loop {
        match fs::read_to_string(&values_path)
            .ok()
            .and_then(|s| parse_values(&s))
        {
            Some(values) if Some(values) != last => {
                let (progress_tenths, elapsed_seconds, playing) = values;

                let raw = sysinfo_raw(
                    CPU,
                    GPU,
                    UNKNOWN,
                    elapsed_seconds,
                    progress_tenths,
                );

                let mut hid = [0u8; 33];
                hid[1..].copy_from_slice(&raw);

                let written = device.write(&hid)?;
                if written != 33 {
                    return Err(format!("HID write returned {written}, expected 33").into());
                }

                println!(
                    "sent: Mbps={:5.1}%  RPM={}  {}",
                    progress_tenths as f32 / 10.0,
                    elapsed_seconds,
                    if playing { "PLAYING" } else { "PAUSED" }
                );

                last = Some(values);
            }
            Some(_) => {}
            None => {
                println!("waiting for valid values file...");
            }
        }

        sleep(Duration::from_millis(100));
    }
}
