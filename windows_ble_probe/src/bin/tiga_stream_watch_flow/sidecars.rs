//! The files beside the stream: `.frame`, `.delivered`, `.stop`, `.session`, `.exit`.

use crate::failure::{Failed, Signature};
use crate::logging::{log, utc_now};
use crate::timing::TimingSummary;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

// ---- Diagnostics (issue #9) ------------------------------------------------------
//
// The renderer writes `<stream>.frame` (frame_id + CRC-32 of the stream) before it
// replaces the stream. The uploader logs that frame ID at upload start, completion and
// failure, and records each successful delivery in `<stream>.delivered`, which
// STATUS_STACK.ps1 reads. The CRC-32 matches Python's zlib.crc32.

pub(crate) fn sidecar_path(stream_path: &Path, ext: &str) -> PathBuf {
    let mut name = stream_path.as_os_str().to_owned();
    name.push(ext);
    PathBuf::from(name)
}

pub(crate) fn sidecar_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim())
}

/// Frame ID for a stream, taken from the renderer's sidecar when its CRC matches.
pub(crate) fn frame_id_for(stream_path: &Path, crc: &str) -> String {
    let Ok(text) = fs::read_to_string(sidecar_path(stream_path, ".frame")) else {
        return "unknown(no-sidecar)".into();
    };
    match (sidecar_value(&text, "frame_id"), sidecar_value(&text, "crc32")) {
        (Some(id), Some(c)) if c.eq_ignore_ascii_case(crc) => id.to_string(),
        (Some(id), Some(_)) => format!("unknown(sidecar-is-{id})"),
        _ => "unknown(bad-sidecar)".into(),
    }
}

pub(crate) fn run_id() -> String {
    std::env::var("TIGA_RUN_ID").unwrap_or_default()
}

/// Atomically writes `<stream><ext>` (tmp + rename) so readers never see half a file.
pub(crate) fn write_sidecar(stream_path: &Path, ext: &str, text: &str) {
    let path = sidecar_path(stream_path, ext);
    let tmp = sidecar_path(stream_path, &format!("{ext}.tmp"));
    if let Err(e) = fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, &path)) {
        log!("warning: could not write {}: {e}", path.display());
    }
}

// ---- Stop request (docs/design/CLEAN_STOP.md) --------------------------------------
//
// STOP_STACK.ps1 writes `<stream>.stop` and waits for the uploader to exit. The
// uploader checks for it only between uploads: an upload in progress is always
// finished (or fails the normal way) first, because abandoning one is what leaves the
// keyboard needing its battery disconnected. It then disconnects and exits 0.

pub(crate) fn stop_requested(stream_path: &Path) -> bool {
    sidecar_path(stream_path, ".stop").exists()
}

pub(crate) fn record_delivery(stream_path: &Path, frame_id: &str, crc: &str, elapsed_s: f64, timing: Option<&TimingSummary>) {
    write_sidecar(
        stream_path,
        ".delivered",
        &format!(
            "delivered_at={}\nframe_id={frame_id}\ncrc32={crc}\nelapsed_s={elapsed_s:.2}\nrun_id={}\n{}",
            utc_now(),
            run_id(),
            timing.map(TimingSummary::sidecar_lines).unwrap_or_default()
        ),
    );
}

pub(crate) fn write_session_marker(stream_path: &Path, frame_id: &str, last_acked: usize, total: usize) {
    write_sidecar(
        stream_path,
        ".session",
        &format!(
            "updated_at={}\nframe_id={frame_id}\nlast_acked_block={last_acked}\ntotal_blocks={total}\nrun_id={}\n",
            utc_now(),
            run_id()
        ),
    );
}

pub(crate) fn read_session_marker(stream_path: &Path) -> Option<String> {
    fs::read_to_string(sidecar_path(stream_path, ".session")).ok()
}

pub(crate) fn clear_session_marker(stream_path: &Path) {
    let path = sidecar_path(stream_path, ".session");
    if let Err(e) = fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            log!("warning: could not remove {}: {e}", path.display());
        }
    }
}

/// Human-readable record of why this run exited; the supervisor copies it into its
/// halt state when the run_id matches the process it started.
pub(crate) fn record_exit(stream_path: &Path, f: &Failed) {
    write_sidecar(
        stream_path,
        ".exit",
        &format!(
            "exited_at={}\nrun_id={}\ncode={}\nmeaning={}\ndetail={}\nframe_id={}\nlast_acked_block={}\nsignature={}\n",
            utc_now(),
            run_id(),
            f.kind.code(),
            f.kind.meaning(),
            f.detail.replace(['\r', '\n'], " "),
            f.frame_id.as_deref().unwrap_or(""),
            f.last_acked_block.map(|b| b.to_string()).unwrap_or_default(),
            f.signature.map(Signature::as_str).unwrap_or("")
        ),
    );
}

/// The renderer re-rendered pixels identical to the last completed upload (Standby
/// pressed twice in the same minute, say). Nothing is sent, but the new frame ID is
/// recorded as delivered: the keyboard already shows exactly these bytes, and the
/// control utility waits for its frame ID before shutting down (issue #7).
pub(crate) fn note_already_displayed(stream_path: &Path, last_crc: &str, last_frame_id: &mut String) -> bool {
    if last_crc.is_empty() {
        return false;
    }
    let id = frame_id_for(stream_path, last_crc);
    if id.starts_with("unknown(") || id == *last_frame_id {
        return false;
    }
    log!("ALREADY DISPLAYED frame={id} crc32={last_crc} (identical to the last upload)");
    record_delivery(stream_path, &id, last_crc, 0.0, None);
    *last_frame_id = id;
    true
}

// ---- Waiting for the renderer's first frame (Phase 8, 2026-10-25) -----------------
//
// A stream with no `<stream>.frame` at all was not written by the renderer: it is the
// red/green template the installer or a supervisor seeded. On the first packaged start
// the renderer took 36 s to write its first frame, and the uploader spent 64 s sending
// the template meanwhile. Such a stream is now held, logged once, until the sidecar
// appears. A sidecar that exists but names another CRC is uploaded as before
// (`unknown(sidecar-is-…)`). `--allow-no-sidecar` is for a manual upload of the template.

#[derive(Debug, PartialEq)]
pub(crate) enum Gate {
    Upload,
    /// Don't upload yet: log the line, if any, and look again shortly.
    Hold(Option<String>),
}

pub(crate) struct RendererGate {
    pub(crate) allow_no_sidecar: bool,
    /// When the current hold began, while a stream without a sidecar is held.
    pub(crate) held_since: Option<Instant>,
}

impl RendererGate {
    pub(crate) fn new(allow_no_sidecar: bool) -> Self {
        RendererGate { allow_no_sidecar, held_since: None }
    }

    /// `crc` is the stream's, for the log line.
    pub(crate) fn check(&mut self, stream_path: &Path, crc: &str) -> Gate {
        let sidecar = sidecar_path(stream_path, ".frame");
        let rendered = self.allow_no_sidecar || sidecar.exists();
        match (rendered, self.held_since) {
            (true, None) => Gate::Upload,
            (false, Some(_)) => Gate::Hold(None),
            (false, None) => {
                self.held_since = Some(Instant::now());
                Gate::Hold(Some(format!(
                    "UPLOAD HELD frame=unknown(no-sidecar) crc32={crc}: no {} yet, so the renderer \
                     hasn't written this stream (it is the seeded template); uploading once it has",
                    sidecar.display()
                )))
            }
            // The renderer writes the sidecar just before it replaces the stream: one more
            // look, so that the upload reads its frame and not the template.
            (true, Some(since)) => {
                self.held_since = None;
                Gate::Hold(Some(format!(
                    "RENDERER sidecar appeared after {:.1}s; uploading its frame",
                    since.elapsed().as_secs_f64()
                )))
            }
        }
    }
}
