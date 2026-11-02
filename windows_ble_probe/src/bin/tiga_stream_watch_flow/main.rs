//! The flow-controlled BLE uploader: watches the stream file and sends each new frame to
//! the keyboard. This file holds the command line and the main loop.

mod device;
mod failure;
mod logging;
mod notifications;
mod protocol;
mod sidecars;
mod stream;
mod timing;
mod upload;

#[cfg(test)]
mod tests;

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::Manager;
use device::{read_cached_device, select_device, write_cached_device, BleRadio, Chosen, GattLink};
use failure::{stall_signature, Classify, Failed, Failure, Signature};
use logging::{log, utc_now};
use notifications::{flush_notification_repeats, listen_between_uploads};
use protocol::TARGET_NAME;
use sidecars::{
    frame_id_for, note_already_displayed, read_session_marker, record_delivery, record_exit, sidecar_path,
    sidecar_value, stop_requested, Gate, RendererGate,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};
use stream::{build_chunks, crc32, validate_stream};
use timing::UploadTimings;
use upload::upload_stream;

// The tests see everything through `use super::*`.
#[cfg(test)]
use {
    btleplug::api::WriteType, device::*, failure::*, notifications::*, protocol::*, sidecars::*, stream::*, timing::*,
    upload::*, uuid::Uuid,
};

/// How long to wait for one block's PROGRESS before abandoning the transfer. Healthy
/// blocks peak around 1 s. 10000 was tried 2026-10-15/16: it rescued no upload, and its
/// one slowing run stalled anyway, only later (PROJECT_HISTORY §7.1, third part).
const BLOCK_TIMEOUT_MS: u64 = 2_500;

struct Opts {
    stream_path: PathBuf,
    intra_block_ms: u64,
    block_timeout_ms: u64,
    once: bool,
    live: bool,
    allow_single_frame: bool,
    allow_no_sidecar: bool,
}

fn parse_opts(args: &[String]) -> Result<Opts, Failed> {
    fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a String> {
        args.windows(2).find(|w| w[0] == flag).map(|w| &w[1])
    }
    fn number(args: &[String], flag: &str, default: u64) -> Result<u64, Failed> {
        match value(args, flag) {
            None => Ok(default),
            Some(v) => v.parse().class(Failure::Fatal).map_err(|mut f| {
                f.detail = format!("{flag} {v}: {}", f.detail);
                f
            }),
        }
    }
    Ok(Opts {
        stream_path: value(args, "--stream")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"..\extracted\session_01_bulk_stream.bin")),
        intra_block_ms: number(args, "--intra-block-ms", 2)?,
        block_timeout_ms: number(args, "--block-timeout-ms", BLOCK_TIMEOUT_MS)?,
        once: args.iter().any(|a| a == "--once"),
        live: args.iter().any(|a| a == "--confirm-flow-watch"),
        allow_single_frame: args.iter().any(|a| a == "--allow-single-frame"),
        allow_no_sidecar: args.iter().any(|a| a == "--allow-no-sidecar"),
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    log!(
        "TIGA flow-controlled stream uploader starting (run_id={})",
        std::env::var("TIGA_RUN_ID").unwrap_or_else(|_| "-".into())
    );

    let opts = match parse_opts(&args) {
        Ok(o) => o,
        Err(f) => return exit_with(None, f),
    };
    match run(&opts).await {
        Ok(()) => {
            flush_notification_repeats();
            log!("EXIT code=0 (success)");
            ExitCode::SUCCESS
        }
        Err(f) => exit_with(Some(&opts.stream_path), f),
    }
}

fn exit_with(stream_path: Option<&Path>, f: Failed) -> ExitCode {
    flush_notification_repeats();
    let line = format!(
        "EXIT code={} ({}): {}{}",
        f.kind.code(),
        f.kind.meaning(),
        f.detail,
        f.frame_id.as_deref().map(|id| format!(" frame={id}")).unwrap_or_default()
    );
    log!("{line}");
    eprintln!("{} {line}", utc_now());
    if let Some(path) = stream_path {
        record_exit(path, &f);
    }
    ExitCode::from(f.kind.code())
}

async fn run(opts: &Opts) -> Result<(), Failed> {
    let stream_path = &opts.stream_path;
    let (intra_block_ms, block_timeout_ms) = (opts.intra_block_ms, opts.block_timeout_ms);
    let (once, allow_single_frame) = (opts.once, opts.allow_single_frame);

    println!("Stream          : {}", stream_path.display());
    println!("Intra-block pace: {} ms/write", intra_block_ms);
    println!("Flow control    : WAIT for one PROGRESS after every 4096-byte DLX block");
    println!("Block timeout   : {} ms", block_timeout_ms);
    println!("Mode            : {}", if once { "one upload" } else { "watch + coalesce" });

    let initial = fs::read(stream_path)
        .class(Failure::Fatal)
        .map_err(|mut f| { f.detail = format!("{}: {}", stream_path.display(), f.detail); f })?;
    let initial_chunks = build_chunks(&initial, allow_single_frame).class(Failure::Fatal)?;
    println!(
        "Validated: {} bytes, {} writes, {} x 4096-byte progress blocks{}",
        initial.len(),
        initial_chunks.len(),
        initial_chunks.iter().filter(|c| c.ends_4k_block).count(),
        if allow_single_frame { "  [--allow-single-frame: EXPERIMENTAL]" } else { "" }
    );

    // A session left open by an earlier run (killed, crashed, or the PC shut down
    // mid-transfer) means the keyboard has not been power-cycled since.
    if let Some(marker) = read_session_marker(stream_path) {
        let frame = sidecar_value(&marker, "frame_id").unwrap_or("?").to_string();
        let block = sidecar_value(&marker, "last_acked_block").and_then(|b| b.parse().ok());
        let detail = format!(
            "a media session was left open by an earlier run (frame {frame}, last ACKed block {}); \
             disconnect the keyboard's battery for at least 30 s (there is no power switch; \
             unplugging USB doesn't count), then run START_STACK.ps1 (or delete {})",
            block.map(|b: usize| b.to_string()).unwrap_or_else(|| "?".into()),
            sidecar_path(stream_path, ".session").display()
        );
        if opts.live {
            return Err(Failed {
                kind: Failure::Interrupted,
                detail,
                frame_id: Some(frame),
                last_acked_block: block,
                progress_timeout: false,
                // A session left by an earlier run: nothing is known about its pace.
                signature: Some(Signature::Unclassified),
            });
        }
        println!("WARNING: {detail}");
    }

    if !opts.allow_no_sidecar && !sidecar_path(stream_path, ".frame").exists() {
        println!(
            "Note: no {} yet. A live run holds this stream until the renderer writes a frame \
             (--allow-no-sidecar uploads it anyway).",
            sidecar_path(stream_path, ".frame").display()
        );
    }

    if !opts.live {
        println!();
        println!("DRY RUN ONLY — no Bluetooth connection was opened.");
        println!("Live mode requires --confirm-flow-watch");
        return Ok(());
    }

    if stop_requested(stream_path) {
        log!(
            "STOP requested before connecting ({} exists); not connecting.",
            sidecar_path(stream_path, ".stop").display()
        );
        return Ok(());
    }

    let manager = Manager::new().await.class(Failure::Unreachable)?;
    let adapters = manager.adapters().await.class(Failure::Unreachable)?;
    let adapter = adapters.first().ok_or("No Bluetooth adapter found").class(Failure::Unreachable)?;
    log!(
        "Using adapter 1/{}: {}",
        adapters.len(),
        adapter.adapter_info().await.unwrap_or_else(|e| format!("(info unavailable: {e})"))
    );

    let cached = read_cached_device(stream_path);
    match &cached {
        Some(a) => log!("Scanning for {TARGET_NAME} (trying {a} first: it completed the last upload)..."),
        None => log!("Scanning for {TARGET_NAME}..."),
    }
    adapter.start_scan(ScanFilter::default()).await.class(Failure::Unreachable)?;
    let mut radio = BleRadio { adapter, started: Instant::now() };
    // select_device stops the scan once a device qualifies (UPLOAD_FAILURES_2026-10-18,
    // proposal B). Until 2026-10-18 it ran through every upload. Nothing after this
    // point scans: any failure ends the process, and the next run starts a fresh scan.
    let Chosen { address, name, dev: peripheral, link } = select_device(&mut radio, cached.as_deref()).await?;
    let GattLink { control, notify1, bulk, notify2 } = link;
    log!("Found keyboard at {address}");

    let mut notifications = peripheral.notifications().await.class(Failure::Gatt)?;
    peripheral.subscribe(&notify1).await.class(Failure::Gatt)?;
    peripheral.subscribe(&notify2).await.class(Failure::Gatt)?;
    log!("Connected once. Notifications active.");

    let mut last_uploaded: Option<Vec<u8>> = None;
    let mut last_crc = String::new();
    let mut last_frame_id = String::new();
    let mut device_recorded = false;
    let mut first_on_connection = true;
    let mut stream_ended = false;
    let mut gate = RendererGate::new(opts.allow_no_sidecar);

    loop {
        // Only here, between uploads: never in the middle of one.
        if stop_requested(stream_path) {
            flush_notification_repeats();
            log!("STOP requested; no upload in progress, disconnecting.");
            break;
        }

        let stream = fs::read(stream_path).class(Failure::Fatal)?;
        validate_stream(&stream, allow_single_frame).class(Failure::Fatal)?;

        if last_uploaded.as_deref() == Some(stream.as_slice()) {
            note_already_displayed(stream_path, &last_crc, &mut last_frame_id);
            if once { break; }
            listen_between_uploads(&mut notifications, Duration::from_millis(250), &mut stream_ended).await;
            continue;
        }

        let crc = format!("{:08x}", crc32(&stream));
        if let Gate::Hold(line) = gate.check(stream_path, &crc) {
            if let Some(line) = line {
                flush_notification_repeats();
                log!("{line}");
            }
            listen_between_uploads(&mut notifications, Duration::from_millis(250), &mut stream_ended).await;
            continue;
        }

        let chunks = build_chunks(&stream, allow_single_frame).class(Failure::Fatal)?;
        let frame_id = frame_id_for(stream_path, &crc);
        println!();
        flush_notification_repeats();
        log!("UPLOAD START frame={frame_id} crc32={crc}");

        let started = Instant::now();
        let mut timings = UploadTimings::default();
        let result = upload_stream(
            &peripheral,
            &control,
            &bulk,
            &mut notifications,
            &chunks,
            intra_block_ms,
            block_timeout_ms,
            stream_path,
            &frame_id,
            first_on_connection,
            &mut timings,
        )
        .await;
        let elapsed = started.elapsed().as_secs_f64();
        first_on_connection = false;
        let summary = timings.summary();
        if let Err(mut f) = result {
            flush_notification_repeats();
            log!("UPLOAD FAILED frame={frame_id} crc32={crc} after {elapsed:.2}s: {}", f.detail);
            log!("{}", summary.log_line(&frame_id));
            if f.kind == Failure::Interrupted {
                let sig = stall_signature(&timings.block_ms, f.progress_timeout);
                log!("SIGNATURE {} frame={frame_id}", sig.as_str());
                f.signature = Some(sig);
            }
            return Err(f);
        }

        flush_notification_repeats();
        log!("UPLOAD COMPLETE frame={frame_id} crc32={crc} in {elapsed:.2}s");
        log!("{}", summary.log_line(&frame_id));
        record_delivery(stream_path, &frame_id, &crc, elapsed, Some(&summary));
        if !device_recorded {
            write_cached_device(stream_path, &address, &name);
            device_recorded = true;
        }

        last_uploaded = Some(stream);
        last_crc = crc;
        last_frame_id = frame_id;

        if once {
            break;
        }

        listen_between_uploads(&mut notifications, Duration::from_millis(100), &mut stream_ended).await;
    }

    peripheral.disconnect().await.class(Failure::Unreachable)?;
    log!("Disconnected cleanly.");
    Ok(())
}
