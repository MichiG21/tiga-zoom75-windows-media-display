//! Reading the keyboard's notifications: what gets logged, and the waits for its answers.

use crate::logging::{log, utc_now};
use crate::protocol::*;
use crate::timing::ms;
use futures::StreamExt;
use std::{
    error::Error,
    time::{Duration, Instant},
};
use tokio::time::sleep;
use uuid::Uuid;

// ---- Notification log (docs/investigations/UPLOAD_FAILURES_2026-10-18.md, proposal A) --
//
// Logging only: nothing here changes what is written or when. Every notification other
// than PROGRESS and COMPLETE is logged with its characteristic, its value in hex, what
// the uploader was waiting for when it arrived, and how long that wait had lasted.
// Before this, anything else the keyboard sent (an error, a different status) was
// discarded unseen, including at the moment of a stall. Between uploads the stream is
// read too, so a notification is logged when it arrives rather than not at all.

/// Short name for the notifications the protocol expects (other than PROGRESS/COMPLETE).
pub(crate) fn known_notification(value: &[u8]) -> Option<&'static str> {
    match value {
        v if v == C1_ACK_A => Some("CONTROL1 ACK A"),
        v if v == C1_ACK_B => Some("CONTROL1 ACK B"),
        v if v == C2_ACK_A => Some("CONTROL2 ACK A"),
        v if v == C2_ACK_B => Some("CONTROL2 ACK B"),
        _ => None,
    }
}

pub(crate) fn should_log_notification(value: &[u8]) -> bool {
    value != PROGRESS && value != COMPLETE
}

pub(crate) fn notification_line(uuid: &Uuid, value: &[u8], phase: &str, waited: Duration) -> String {
    let hex: Vec<String> = value.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "NOTIFY char={} len={} value={}{} while={phase} after={:.0}ms",
        &uuid.to_string()[..8],
        value.len(),
        hex.join(" "),
        known_notification(value).map(|k| format!(" ({k})")).unwrap_or_default(),
        ms(waited)
    )
}

// ---- Repeats -----------------------------------------------------------------------
//
// The keyboard sends the same 28-byte status message on 1f400002 every 1-3 s
// (`88 00 00 00 00 00 14 2a 03 0d …`, docs/PROTOCOL_NOTES.md). Logged in full each time
// it was ~300 KB/h of ble.log and pushed older upload history out of the 3 x 5 MB
// rotation. So a notification identical to the one logged just before it (same
// characteristic, same bytes) is only counted. The count goes out as one summary line
// when a different notification arrives, before each UPLOAD START / COMPLETE / FAILED,
// and before the uploader exits. Anything that differs is still logged at once, in full.

#[derive(Default)]
pub(crate) struct RepeatFilter {
    /// The last notification logged in full.
    pub(crate) current: Option<(Uuid, Vec<u8>)>,
    /// Identical ones counted since that line or the last summary.
    pub(crate) repeats: usize,
    pub(crate) first_repeat: String,
    pub(crate) last_repeat: String,
}

impl RepeatFilter {
    pub(crate) const fn new() -> Self {
        RepeatFilter { current: None, repeats: 0, first_repeat: String::new(), last_repeat: String::new() }
    }

    /// The lines to log for one notification, in order: nothing for a repeat; otherwise a
    /// pending summary (if any) and then `line`. `now` is the arrival time.
    pub(crate) fn note(&mut self, uuid: &Uuid, value: &[u8], line: String, now: &str) -> Vec<String> {
        if self.current.as_ref().is_some_and(|(u, v)| u == uuid && v == value) {
            if self.repeats == 0 {
                self.first_repeat = now.to_string();
            }
            self.repeats += 1;
            self.last_repeat = now.to_string();
            return Vec::new();
        }
        let mut lines: Vec<String> = self.flush().into_iter().collect();
        lines.push(line);
        self.current = Some((*uuid, value.to_vec()));
        lines
    }

    /// The summary of the repeats counted so far, if any. Later identical notifications
    /// are counted again from zero, so each summary covers the span since the previous
    /// line about this value.
    pub(crate) fn flush(&mut self) -> Option<String> {
        if self.repeats == 0 {
            return None;
        }
        let (uuid, value) = self.current.as_ref()?;
        let hex: Vec<String> = value.iter().map(|b| format!("{b:02x}")).collect();
        let times = if self.repeats == 1 { "time" } else { "times" };
        let line = format!(
            "NOTIFY repeated {} more {times} char={} len={} value={} first={} last={}",
            self.repeats,
            &uuid.to_string()[..8],
            value.len(),
            hex.join(" "),
            self.first_repeat,
            self.last_repeat
        );
        self.repeats = 0;
        Some(line)
    }
}

pub(crate) static NOTIFY_REPEATS: std::sync::Mutex<RepeatFilter> = std::sync::Mutex::new(RepeatFilter::new());

/// Logs `n` unless it is PROGRESS or COMPLETE, or a repeat of the previous logged one.
/// `since` is when the current wait began.
pub(crate) fn log_notification(n: &btleplug::api::ValueNotification, phase: &str, since: Instant) {
    if should_log_notification(&n.value) {
        let line = notification_line(&n.uuid, &n.value, phase, since.elapsed());
        let lines = match NOTIFY_REPEATS.lock() {
            Ok(mut f) => f.note(&n.uuid, &n.value, line, &utc_now()),
            Err(_) => vec![line],
        };
        for l in lines {
            log!("{l}");
        }
    }
}

/// Writes out the pending repeat summary, if there is one.
pub(crate) fn flush_notification_repeats() {
    if let Some(line) = NOTIFY_REPEATS.lock().ok().and_then(|mut f| f.flush()) {
        log!("{line}");
    }
}

/// Waits `d` between uploads while logging whatever arrives. Returns how many
/// notifications it handled (logged, or counted as repeats); PROGRESS and COMPLETE are
/// not included. A stream that has ended is reported once (`ended`) and then left alone;
/// noticing it is still the next upload's job, as before.
pub(crate) async fn listen_between_uploads<S>(notifications: &mut S, d: Duration, ended: &mut bool) -> usize
where
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    let since = Instant::now();
    let deadline = sleep(d);
    tokio::pin!(deadline);
    let mut logged = 0;
    while !*ended {
        tokio::select! {
            _ = &mut deadline => return logged,
            n = notifications.next() => match n {
                Some(n) => {
                    if should_log_notification(&n.value) {
                        logged += 1;
                    }
                    log_notification(&n, "between uploads", since);
                }
                None => {
                    *ended = true;
                    log!("NOTIFY stream ended while idle (between uploads)");
                }
            }
        }
    }
    deadline.await;
    logged
}

pub(crate) async fn wait_for_ack_pair<S>(
    notifications: &mut S,
    a_expected: &[u8],
    b_expected: &[u8],
    timeout_ms: u64,
    phase: &str,
) -> Result<(), Box<dyn Error>>
where
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    let since = Instant::now();
    let timeout = sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout);
    let (mut a, mut b) = (false, false);

    loop {
        tokio::select! {
            _ = &mut timeout => break,
            n = notifications.next() => {
                let Some(n) = n else { return Err("notification stream ended".into()); };
                log_notification(&n, phase, since);
                if n.value.as_slice() == a_expected { a = true; }
                if n.value.as_slice() == b_expected { b = true; }
                if a && b { return Ok(()); }
            }
        }
    }

    Err("control ACK pair timed out".into())
}

pub(crate) async fn wait_for_progress<S>(
    notifications: &mut S,
    timeout_ms: u64,
    phase: &str,
) -> Result<bool, Box<dyn Error>>
where
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    let since = Instant::now();
    let timeout = sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => return Ok(false),
            n = notifications.next() => {
                let Some(n) = n else { return Err("notification stream ended".into()); };
                log_notification(&n, phase, since);
                if n.value.as_slice() == PROGRESS {
                    return Ok(true);
                }
                if n.value.as_slice() == COMPLETE {
                    // Completion before the final partial block would be odd, but do not
                    // treat it as a progress ACK.
                    println!("warning: completion arrived while waiting for progress");
                }
            }
        }
    }
}

pub(crate) async fn wait_for_completion<S>(
    notifications: &mut S,
    timeout_ms: u64,
) -> Result<bool, Box<dyn Error>>
where
    S: futures::Stream<Item = btleplug::api::ValueNotification> + Unpin,
{
    let since = Instant::now();
    let timeout = sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => return Ok(false),
            n = notifications.next() => {
                let Some(n) = n else { return Err("notification stream ended".into()); };
                log_notification(&n, "waiting for COMPLETE", since);
                if n.value.as_slice() == COMPLETE {
                    return Ok(true);
                }
            }
        }
    }
}
