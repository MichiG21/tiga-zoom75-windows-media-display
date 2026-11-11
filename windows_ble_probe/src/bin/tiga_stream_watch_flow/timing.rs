//! Per-block timing of an upload, and the timer resolution the write pacing depends on.

use std::time::Duration;

// ---- Timer resolution (2026-11-11) -------------------------------------------------
//
// The bulk loop sleeps --intra-block-ms (2 ms) after each of the ~23 writes per 4K block,
// with tokio::time::sleep. On Windows that sleep only lasts about 2-3 ms if this process
// has asked for a fine timer. Since Windows 10 2004 another program raising the system
// resolution no longer counts. Without it the sleep lasts a whole 15.6 ms tick. Measured
// on Michi's PC, same binary, same afternoon:
//
//   started by the scheduled task       median block 157 ms, upload  8.89 s
//   started from a shell (no request)   median block 420 ms, upload 23.27 s
//   2 ms sleep, no request / requested  median 15.5 ms / 3.15 ms
//
// Why the scheduled task happened to get the fine timer is not known. Asking for it
// here makes the pace the same however the uploader is started. The request lasts
// until the process exits, which releases it.

/// Asks Windows for 1 ms timer resolution for this process. True if it was granted.
pub(crate) fn request_fine_timer() -> bool {
    #[cfg(windows)]
    {
        #[link(name = "winmm")]
        extern "system" {
            fn timeBeginPeriod(u_period: u32) -> u32;
        }
        // TIMERR_NOERROR is 0.
        unsafe { timeBeginPeriod(1) == 0 }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

// ---- Per-block timing (PROJECT_HISTORY §7.1) --------------------------------------
//
// Logging only: nothing here changes a write, a timeout or an exit code. A block's
// latency is the time from the previous PROGRESS (for block 1, the start of the bulk
// writes) to its own PROGRESS, so it includes that block's writes. Healthy blocks take
// ~0.15 s; in the slowing failures they took ~1.2-2.5 s each.

/// A block slower than this gets a `SLOW` line.
pub(crate) const SLOW_BLOCK_MS: f64 = 1000.0;
/// On the first upload of a connection, block 5 ACKed later than this (elapsed since the
/// bulk writes started) gets an early warning. Healthy first uploads: 0.71-0.85 s; all
/// three slowing runs: 2.35-2.97 s.
pub(crate) const FIRST_UPLOAD_BLOCK5_WARN_S: f64 = 1.5;
pub(crate) const EARLY_WARNING_BLOCK: usize = 5;

#[derive(Default)]
pub(crate) struct UploadTimings {
    pub(crate) control1_ack_ms: Option<f64>,
    pub(crate) control2_ack_ms: Option<f64>,
    /// Latency of each ACKed block, in block order (index 0 = block 1).
    pub(crate) block_ms: Vec<f64>,
}

pub(crate) struct TimingSummary {
    pub(crate) blocks: usize,
    pub(crate) median_ms: f64,
    pub(crate) p90_ms: f64,
    pub(crate) max_ms: f64,
    pub(crate) slowest_block: usize,
    pub(crate) control1_ack_ms: Option<f64>,
    pub(crate) control2_ack_ms: Option<f64>,
}

pub(crate) fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

pub(crate) fn fmt_ms(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "-".into())
}

/// Median (mean of the two middle values for an even count) and nearest-rank p90.
pub(crate) fn median_p90(values: &[f64]) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    let median = if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 };
    let rank = (n * 9).div_ceil(10).max(1);
    Some((median, v[rank - 1]))
}

impl UploadTimings {
    pub(crate) fn summary(&self) -> TimingSummary {
        let (median_ms, p90_ms) = median_p90(&self.block_ms).unwrap_or((0.0, 0.0));
        // The first block wins a tie, so a stall is attributed where it started.
        let (slowest_block, max_ms) = self
            .block_ms
            .iter()
            .enumerate()
            .fold((0, 0.0f64), |best, (i, &v)| if v > best.1 { (i + 1, v) } else { best });
        TimingSummary {
            blocks: self.block_ms.len(),
            median_ms,
            p90_ms,
            max_ms,
            slowest_block,
            control1_ack_ms: self.control1_ack_ms,
            control2_ack_ms: self.control2_ack_ms,
        }
    }
}

impl TimingSummary {
    pub(crate) fn log_line(&self, frame_id: &str) -> String {
        format!(
            "TIMING frame={frame_id} blocks_acked={} median={:.0}ms p90={:.0}ms max={:.0}ms slowest=block {} \
             control1_ack={}ms control2_ack={}ms",
            self.blocks,
            self.median_ms,
            self.p90_ms,
            self.max_ms,
            self.slowest_block,
            fmt_ms(self.control1_ack_ms),
            fmt_ms(self.control2_ack_ms)
        )
    }

    pub(crate) fn sidecar_lines(&self) -> String {
        format!(
            "block_median_ms={:.0}\nblock_p90_ms={:.0}\nblock_max_ms={:.0}\nslowest_block={}\n\
             control1_ack_ms={}\ncontrol2_ack_ms={}\n",
            self.median_ms,
            self.p90_ms,
            self.max_ms,
            self.slowest_block,
            fmt_ms(self.control1_ack_ms),
            fmt_ms(self.control2_ack_ms)
        )
    }
}

/// The `SLOW` line for one block, if it deserves one.
pub(crate) fn slow_block_line(block_no: usize, total_blocks: usize, latency_ms: f64, elapsed_s: f64) -> Option<String> {
    (latency_ms > SLOW_BLOCK_MS).then(|| {
        format!("SLOW block {block_no}/{total_blocks} took {latency_ms:.0} ms (elapsed={elapsed_s:.2}s)")
    })
}

/// The early warning for a first-on-connection upload whose block 5 came late.
pub(crate) fn first_upload_warning(first_on_connection: bool, block_no: usize, elapsed_s: f64) -> Option<String> {
    (first_on_connection && block_no == EARLY_WARNING_BLOCK && elapsed_s > FIRST_UPLOAD_BLOCK5_WARN_S).then(|| {
        format!("WARNING first upload is slow (block 5 at {elapsed_s:.2} s; healthy 0.7–0.85 s)")
    })
}
