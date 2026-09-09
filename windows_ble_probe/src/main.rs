use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::Manager;
use std::{error::Error, time::Duration};
use tokio::time::sleep;
use uuid::Uuid;

const TARGET_NAME: &str = "Zoom75 Tiga";
const SERVICE_UUID: Uuid = Uuid::from_u128(0x1f40eaf8_aab4_14a3_f1ba_f61f35cddbaa);
const CHAR_CONTROL: Uuid = Uuid::from_u128(0x1f400001_aab4_14a3_f1ba_f61f35cddbaa);
const CHAR_NOTIFY: Uuid = Uuid::from_u128(0x1f400002_aab4_14a3_f1ba_f61f35cddbaa);
const CHAR_BULK: Uuid = Uuid::from_u128(0x1f400003_aab4_14a3_f1ba_f61f35cddbaa);
const CHAR_NOTIFY2: Uuid = Uuid::from_u128(0x1f400004_aab4_14a3_f1ba_f61f35cddbaa);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let inspect = args.iter().any(|a| a == "--inspect");
    let scan_secs: u64 = args
        .windows(2)
        .find(|w| w[0] == "--seconds")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(10);

    println!("TIGA BLE probe — NO characteristic writes are implemented in this build.");
    println!("Target name : {TARGET_NAME}");
    println!("Service UUID: {SERVICE_UUID}");
    println!("Scan time   : {scan_secs}s");

    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    if adapters.is_empty() {
        return Err("No Bluetooth adapters found by btleplug".into());
    }

    println!("Bluetooth adapters found: {}", adapters.len());

    let mut found_target = None;
    for (idx, adapter) in adapters.iter().enumerate() {
        println!("\n[adapter {idx}] starting BLE scan...");
        adapter.start_scan(ScanFilter::default()).await?;
        sleep(Duration::from_secs(scan_secs)).await;

        let peripherals = adapter.peripherals().await?;
        println!("[adapter {idx}] peripherals visible: {}", peripherals.len());

        for p in peripherals {
            let props = p.properties().await?;
            let name = props
                .as_ref()
                .and_then(|x| x.local_name.as_deref())
                .unwrap_or("<unnamed>");
            if name.to_ascii_lowercase().contains("zoom75") || name == TARGET_NAME {
                println!("  MATCH: name={name:?} address={}", p.address());
                found_target = Some(p.clone());
            }
        }
    }

    let Some(peripheral) = found_target else {
        println!("\nNo Zoom75 Tiga found. Keep the keyboard powered and Bluetooth enabled, then retry.");
        return Ok(());
    };

    if !inspect {
        println!("\nSCAN SUCCESS. Re-run with --inspect to connect and enumerate GATT services.");
        println!("This scan mode performed no GATT writes.");
        return Ok(());
    }

    println!("\nConnecting for GATT inspection...");
    if !peripheral.is_connected().await? {
        peripheral.connect().await?;
    }
    peripheral.discover_services().await?;

    println!("Connected: {}", peripheral.is_connected().await?);
    println!("Reported MTU: {}", peripheral.mtu());

    let mut target_service_seen = false;
    let mut expected = [false; 4];

    for service in peripheral.services() {
        println!("\nSERVICE {}", service.uuid);
        if service.uuid == SERVICE_UUID {
            target_service_seen = true;
            println!("  >>> TARGET TIGA SERVICE <<<");
        }
        for ch in service.characteristics {
            println!("  CHAR {}  props={:?}", ch.uuid, ch.properties);
            if ch.uuid == CHAR_CONTROL { expected[0] = true; println!("       expected role: CONTROL WRITE"); }
            if ch.uuid == CHAR_NOTIFY { expected[1] = true; println!("       expected role: NOTIFY / STATUS"); }
            if ch.uuid == CHAR_BULK { expected[2] = true; println!("       expected role: BULK WRITE"); }
            if ch.uuid == CHAR_NOTIFY2 { expected[3] = true; println!("       expected role: SECOND NOTIFY"); }
        }
    }

    println!("\nVerification:");
    println!("  target service : {}", if target_service_seen { "FOUND" } else { "MISSING" });
    println!("  control 0001   : {}", if expected[0] { "FOUND" } else { "MISSING" });
    println!("  notify  0002   : {}", if expected[1] { "FOUND" } else { "MISSING" });
    println!("  bulk    0003   : {}", if expected[2] { "FOUND" } else { "MISSING" });
    println!("  notify  0004   : {}", if expected[3] { "FOUND" } else { "MISSING" });
    println!("\nNO media/control characteristic writes were sent.");

    peripheral.disconnect().await?;
    println!("Disconnected cleanly.");
    Ok(())
}
