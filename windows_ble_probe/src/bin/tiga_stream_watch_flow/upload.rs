//! One upload: CONTROL1, CONTROL2, the bulk writes and the waits between them.

use crate::failure::{classify_upload_failure, Failed};
use crate::logging::log;
use crate::notifications::*;
use crate::protocol::*;
use crate::sidecars::{clear_session_marker, write_session_marker};
use crate::stream::{control1_for, Chunk};
use crate::timing::{first_upload_warning, ms, slow_block_line, UploadTimings};
use btleplug::api::WriteType;
use futures::StreamExt;
use std::{
    fmt::Display,
    path::Path,
    time::{Duration, Instant},
};
use tokio::time::sleep;

/// How far one upload got; turns any error into a classified failure.
pub(crate) struct UploadProgress<'a> {
    pub(crate) stream_path: &'a Path,
    pub(crate) frame_id: &'a str,
    pub(crate) total_blocks: usize,
    pub(crate) control2_attempted: bool,
    pub(crate) last_acked_block: usize,
    pub(crate) all_blocks_acked: bool,
}

impl UploadProgress<'_> {
    pub(crate) fn fail(&self, detail: impl Display) -> Failed {
        let kind = classify_upload_failure(self.control2_attempted, self.all_blocks_acked);
        Failed {
            kind,
            detail: if self.control2_attempted {
                format!("{detail} (last ACKed block {}/{})", self.last_acked_block, self.total_blocks)
            } else {
                detail.to_string()
            },
            frame_id: Some(self.frame_id.to_string()),
            last_acked_block: self.control2_attempted.then_some(self.last_acked_block),
            progress_timeout: false,
            signature: None,
        }
    }
}

/// The one thing an upload asks of the connected keyboard: a write. Faked in the tests,
/// which pin every write of an upload; the real one only forwards to btleplug.
pub(crate) trait GattWriter {
    async fn write(
        &self,
        characteristic: &btleplug::api::Characteristic,
        data: &[u8],
        write_type: WriteType,
    ) -> btleplug::Result<()>;
}

impl GattWriter for btleplug::platform::Peripheral {
    async fn write(
        &self,
        characteristic: &btleplug::api::Characteristic,
        data: &[u8],
        write_type: WriteType,
    ) -> btleplug::Result<()> {
        btleplug::api::Peripheral::write(self, characteristic, data, write_type).await
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn upload_stream<P, S>(
    peripheral: &P,
    control: &btleplug::api::Characteristic,
    bulk: &btleplug::api::Characteristic,
    notifications: &mut S,
    chunks: &[Chunk],
    intra_block_ms: u64,
    block_timeout_ms: u64,
    stream_path: &Path,
    frame_id: &str,
    first_on_connection: bool,
    timings: &mut UploadTimings,
) -> Result<(), Failed>
where
    P: GattWriter,
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    let mut p = UploadProgress {
        stream_path,
        frame_id,
        total_blocks: chunks.iter().filter(|c| c.ends_4k_block).count(),
        control2_attempted: false,
        last_acked_block: 0,
        all_blocks_acked: false,
    };

    // Drain stale notifications (logged, then discarded as before).
    let drain_started = Instant::now();
    let drain = sleep(Duration::from_millis(150));
    tokio::pin!(drain);
    loop {
        tokio::select! {
            _ = &mut drain => break,
            n = notifications.next() => {
                let Some(n) = n else { return Err(p.fail("notification stream ended")); };
                log_notification(&n, "draining before CONTROL1", drain_started);
            }
        }
    }

    let stream_len = chunks.iter().map(|c| c.value.len() - 1).sum::<usize>();
    let control1_sent = Instant::now();
    peripheral.write(control, &control1_for(stream_len), WriteType::WithoutResponse).await
        .map_err(|e| p.fail(e))?;
    wait_for_ack_pair(notifications, C1_ACK_A, C1_ACK_B, 1500, "waiting for CONTROL1 ACK").await
        .map_err(|e| p.fail(e))?;
    timings.control1_ack_ms = Some(ms(control1_sent.elapsed()));

    sleep(Duration::from_millis(60)).await;

    // ---- Point of no return: from here a failure leaves a partial media session. ----
    write_session_marker(p.stream_path, p.frame_id, 0, p.total_blocks);
    p.control2_attempted = true;
    let control2_sent = Instant::now();
    peripheral.write(control, &CONTROL2, WriteType::WithoutResponse).await
        .map_err(|e| p.fail(e))?;
    wait_for_ack_pair(notifications, C2_ACK_A, C2_ACK_B, 1500, "waiting for CONTROL2 ACK").await
        .map_err(|e| p.fail(e))?;
    timings.control2_ack_ms = Some(ms(control2_sent.elapsed()));

    sleep(Duration::from_millis(60)).await;

    let started = Instant::now();
    let mut previous_ack = started;
    let total_blocks = p.total_blocks;

    for (idx, chunk) in chunks.iter().enumerate() {
        peripheral.write(bulk, &chunk.value, WriteType::WithoutResponse).await
            .map_err(|e| p.fail(e))?;

        if intra_block_ms > 0 {
            sleep(Duration::from_millis(intra_block_ms)).await;
        }

        if chunk.ends_4k_block {
            let block_no = p.last_acked_block + 1;

            let phase = format!("waiting for PROGRESS of block {block_no}/{total_blocks}");
            let ok = wait_for_progress(notifications, block_timeout_ms, &phase).await
                .map_err(|e| p.fail(e))?;
            if !ok {
                let mut f = p.fail(format!(
                    "timed out waiting for PROGRESS after 4K block {block_no}/{total_blocks}"
                ));
                f.progress_timeout = true;
                return Err(f);
            }
            let acked = Instant::now();
            let latency_ms = ms(acked - previous_ack);
            previous_ack = acked;
            timings.block_ms.push(latency_ms);
            p.last_acked_block = block_no;
            write_session_marker(p.stream_path, p.frame_id, block_no, total_blocks);

            let elapsed_s = (acked - started).as_secs_f64();
            if let Some(line) = slow_block_line(block_no, total_blocks, latency_ms, elapsed_s) {
                log!("{line}");
            }
            if let Some(line) = first_upload_warning(first_on_connection, block_no, elapsed_s) {
                log!("{line}");
            }

            if block_no % 5 == 0 || block_no == total_blocks {
                println!(
                    "  block {:2}/{} ACKed  writes={:4}/{}  elapsed={:.2}s",
                    block_no,
                    total_blocks,
                    idx + 1,
                    chunks.len(),
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }

    p.all_blocks_acked = true;
    println!("All {total_blocks} progress blocks ACKed; waiting for final completion...");

    if !wait_for_completion(notifications, 8000).await.map_err(|e| p.fail(e))? {
        return Err(p.fail(format!("no completion after all {total_blocks} block ACKs")));
    }

    // The keyboard closed the session itself.
    clear_session_marker(p.stream_path);
    Ok(())
}
