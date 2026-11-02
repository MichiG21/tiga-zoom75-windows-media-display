//! Failure classes, exit codes and the stall signature.

use crate::timing::{median_p90, SLOW_BLOCK_MS};
use std::fmt::Display;

// ---- Failure classes and exit codes (issue #4) ------------------------------------
//
// The boundary is CONTROL2: it puts the stock firmware into its upload state (the roar
// animation). Before it, the keyboard is untouched and a retry is safe. From the moment
// CONTROL2 is *attempted* (a WriteWithoutResponse that errors may still have gone out),
// the keyboard may hold a partial media session and must be power-cycled first.
//
// The session marker `<stream>.session` is written just before CONTROL2, updated after
// every ACKed block and removed on COMPLETE. It survives a kill or crash, so the
// supervisor can tell an open session apart from a clean failure even when the exit
// code is meaningless (taskkill gives 1).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// No adapter, keyboard not found, connect failed, or the link failed before CONTROL2.
    Unreachable,
    /// Connected, but service discovery / characteristics / subscriptions failed.
    Gatt,
    /// CONTROL2 was sent and the transfer did not finish.
    Interrupted,
    /// Every block was ACKed but COMPLETE never arrived.
    NoCompletion,
    /// Bad arguments, missing or invalid stream, internal error.
    Fatal,
}

impl Failure {
    pub(crate) fn code(self) -> u8 {
        match self {
            Failure::Unreachable => 10,
            Failure::Gatt => 11,
            Failure::Interrupted => 20,
            Failure::NoCompletion => 21,
            Failure::Fatal => 30,
        }
    }

    pub(crate) fn meaning(self) -> &'static str {
        match self {
            Failure::Unreachable => "keyboard not reachable (no adapter, not found, connect failed, or link lost before the media session opened); safe to retry",
            Failure::Gatt => "connected, but the keyboard's GATT service was missing or unusable; safe to retry",
            Failure::Interrupted => "transfer interrupted after CONTROL2; the keyboard may hold a partial media session; disconnect the keyboard's battery for at least 30 s before retrying (there is no power switch; unplugging USB doesn't count)",
            Failure::NoCompletion => "all blocks sent but the keyboard never confirmed completion; disconnect the keyboard's battery for at least 30 s before retrying (there is no power switch; unplugging USB doesn't count)",
            Failure::Fatal => "bad arguments, missing or invalid stream, or internal error; not retried",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Failed {
    pub(crate) kind: Failure,
    pub(crate) detail: String,
    pub(crate) frame_id: Option<String>,
    pub(crate) last_acked_block: Option<usize>,
    /// The keyboard stopped sending PROGRESS within the block timeout (rather than the
    /// link failing or a write erroring).
    pub(crate) progress_timeout: bool,
    /// How the transfer stalled; set only for code 20.
    pub(crate) signature: Option<Signature>,
}

impl Failed {
    pub(crate) fn new(kind: Failure, detail: impl Display) -> Self {
        Failed {
            kind,
            detail: detail.to_string(),
            frame_id: None,
            last_acked_block: None,
            progress_timeout: false,
            signature: None,
        }
    }
}

// ---- Failure signature (PROJECT_HISTORY §7.1) --------------------------------------
//
// Recorded so the supervisor can decide whether a code 20 may be retried
// automatically (docs/design/RETRY_RECOVERY.md). From §7.1:
//
//   sudden  - a normal pace, then the keyboard stops sending PROGRESS. One slow block
//             just before the stall still counts (2026-10-18 14:09).
//   slowing - sustained slowness before the stall: blocks at ~1.2-2.5 s each
//             (2026-10-15 22:44 and 22:46, 2026-10-16 18:49).
//
// A stall before block 5 is sudden whatever its pace: too few blocks to call sustained.
// Healthy uploads often start slowly (block 1 up to 1540 ms on 2026-11-02, and the slowest
// block is among blocks 1-4 in 195 of 213 uploads). Real slowdowns begin at varying blocks
// (2, 5, 5, 5, 6 so far), and the fewest ACKed before one stalled is 6. Before this rule, one
// block of 987 ms was a "slowing" halt (2026-11-02 18:33). The replay of every logged failure
// is in PROJECT_HISTORY §7.1. Both kinds now get automatic retries; the label sets the wait.
//
// Anything that isn't clearly sudden is reported as `unclassified`, and the supervisor
// halts on it exactly as it did before the retry: a link failure or write error rather
// than a PROGRESS timeout, or no ACKed block at all, so there is no pace to judge.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signature {
    Sudden,
    Slowing,
    Unclassified,
}

impl Signature {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Signature::Sudden => "sudden",
            Signature::Slowing => "slowing",
            Signature::Unclassified => "unclassified",
        }
    }
}

/// Median block latency above this means sustained slowness. Healthy medians are
/// 150-188 ms, slowing runs ~1.2-2.5 s per block.
pub(crate) const SLOWING_MEDIAN_MS: f64 = 500.0;
/// Two or more blocks over SLOW_BLOCK_MS also mean sustained slowness; one does not.
pub(crate) const SLOWING_SLOW_BLOCKS: usize = 2;
/// With fewer ACKed blocks than this, the pace isn't judged: the stall counts as sudden.
pub(crate) const SLOWING_MIN_BLOCKS: usize = 5;

pub(crate) fn stall_signature(block_ms: &[f64], progress_timeout: bool) -> Signature {
    if !progress_timeout || block_ms.is_empty() {
        return Signature::Unclassified;
    }
    if block_ms.len() < SLOWING_MIN_BLOCKS {
        return Signature::Sudden;
    }
    let (median, _) = median_p90(block_ms).unwrap_or((0.0, 0.0));
    let slow_blocks = block_ms.iter().filter(|&&v| v > SLOW_BLOCK_MS).count();
    if median > SLOWING_MEDIAN_MS || slow_blocks >= SLOWING_SLOW_BLOCKS {
        Signature::Slowing
    } else {
        Signature::Sudden
    }
}

/// `result.class(Failure::Gatt)?` turns any error into a classified failure.
pub(crate) trait Classify<T> {
    fn class(self, kind: Failure) -> Result<T, Failed>;
}

impl<T, E: Display> Classify<T> for Result<T, E> {
    fn class(self, kind: Failure) -> Result<T, Failed> {
        self.map_err(|e| Failed::new(kind, e))
    }
}

/// Which class a failure inside an upload belongs to, given how far it got.
pub(crate) fn classify_upload_failure(control2_attempted: bool, all_blocks_acked: bool) -> Failure {
    match (control2_attempted, all_blocks_acked) {
        (false, _) => Failure::Unreachable,
        (true, false) => Failure::Interrupted,
        (true, true) => Failure::NoCompletion,
    }
}
